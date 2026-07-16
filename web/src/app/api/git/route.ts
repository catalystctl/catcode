// Git status + actions for the IDE Source Control panel. User-driven (NOT an
// agent turn) — direct `git` CLI invocations scoped to the workspace cwd.
//
// GET  /api/git?workspace=<abs>                 → GitStatus
// GET  /api/git?workspace=<abs>&diff=<rel>&staged=<0|1>  → { diff }
// GET  /api/git?workspace=<abs>&sides=<rel>&staged=<0|1> → { original, modified, path }
//      (two-sided content for the Monaco DiffEditor in the main work area)
// POST /api/git performs validated, user-initiated changes across files,
// commits, sync, branches, history, stashes, tags, remotes, and recovery.
//
// All git commands run via execFile (arg arrays — never a shell, so no
// injection). File-path actions are confined via confinePath. Auth required on
// every entry. stdout/stderr captured; no secrets are logged (commit messages
// and paths are not logged at all).

import { execFile } from "node:child_process";
import { appendFileSync, existsSync, readFileSync, writeFileSync } from "node:fs";
import { isAbsolute, join, resolve } from "node:path";
import { getSession } from "@/lib/auth";
import {
  confinePath,
  resolveAuthorizedWorkspace,
} from "@/server/workspace";
import type {
  GitBranch,
  GitCommit,
  GitOperation,
  GitRemote,
  GitStash,
  GitStatus,
  GitStatusEntry,
  GitTag,
} from "@/lib/types";

export const dynamic = "force-dynamic";
export const runtime = "nodejs";

const MAX_DIFF = 200_000;
const MAX_BUFFER = 8 * 1024 * 1024;

/** Run git without throwing; returns exit code + captured stdout/stderr. */
function runGit(
  workspace: string,
  args: string[],
): Promise<{ code: number; stdout: string; stderr: string }> {
  return new Promise((resolve) => {
    execFile("git", args, { cwd: workspace, maxBuffer: MAX_BUFFER }, (err, stdout, stderr) => {
      let code = 0;
      let errOut = typeof stderr === "string" ? stderr : "";
      if (err) {
        // On non-zero exit Node sets err.code to the numeric exit status (it is
        // typed as string|undefined for errno codes like ENOENT — guard below).
        const c = (err as { code?: unknown }).code;
        code = typeof c === "number" ? c : -1;
        if (!errOut && err.message) errOut = err.message;
      }
      resolve({
        code,
        stdout: typeof stdout === "string" ? stdout : "",
        stderr: errOut,
      });
    });
  });
}

function isNotARepo(stderr: string): boolean {
  return /not a git repository/i.test(stderr);
}

/** Cap a blob/string for the DiffEditor (same budget as unified diffs). */
function truncateBlob(text: string): string {
  if (text.length <= MAX_DIFF) return text;
  return text.slice(0, MAX_DIFF) + "\n... (content truncated)";
}

/** Read a git blob; missing paths return empty string (new/deleted files). */
async function gitBlob(workspace: string, spec: string): Promise<string> {
  const r = await runGit(workspace, ["show", "--textconv", spec]);
  if (r.code !== 0) return "";
  return truncateBlob(r.stdout);
}

/** Two-sided file content for Monaco DiffEditor. */
async function readDiffSides(
  workspace: string,
  relPath: string,
  staged: boolean,
): Promise<{ path: string; original: string; modified: string; staged: boolean }> {
  const abs = join(workspace, relPath);
  let original = "";
  let modified = "";

  if (staged) {
    // HEAD → index
    original = await gitBlob(workspace, `HEAD:${relPath}`);
    modified = await gitBlob(workspace, `:${relPath}`);
  } else {
    // index → worktree (fall back to HEAD when the path is not in the index)
    original = await gitBlob(workspace, `:${relPath}`);
    if (!original) original = await gitBlob(workspace, `HEAD:${relPath}`);
    if (existsSync(abs)) {
      try {
        modified = truncateBlob(readFileSync(abs, "utf8"));
      } catch {
        modified = "";
      }
    } else {
      modified = "";
    }
  }

  return { path: relPath, original, modified, staged };
}

/** Parse `git status --porcelain=v2 --branch` output into GitStatus entries. */
function parsePorcelain(stdout: string): {
  branch: string;
  ahead: number;
  behind: number;
  entries: GitStatusEntry[];
} {
  let branch = "";
  let ahead = 0;
  let behind = 0;
  const entries: GitStatusEntry[] = [];
  for (const line of stdout.split("\n")) {
    if (!line) continue;
    if (line.startsWith("# branch.head ")) {
      const v = line.slice("# branch.head ".length).trim();
      branch = v === "(detached)" ? "HEAD (detached)" : v;
    } else if (line.startsWith("# branch.ab ")) {
      const m = line.match(/\+(\d+)\s+-(\d+)/);
      if (m) {
        ahead = parseInt(m[1], 10);
        behind = parseInt(m[2], 10);
      }
    } else if (line.startsWith("1 ")) {
      // 1 XY sub mH mI mW hH hI path  (path may contain spaces → keep rest)
      const parts = line.split(" ", 9);
      entries.push(...makeEntries(parts[1] ?? "  ", parts[8] ?? "", null, false));
    } else if (line.startsWith("2 ")) {
      // 2 XY sub mH mI mW hH hI Xscore path\torigPath
      const parts = line.split(" ", 10);
      const tail = parts[9] ?? "";
      const tab = tail.indexOf("\t");
      const newPath = tab >= 0 ? tail.slice(0, tab) : tail;
      const origPath = tab >= 0 ? tail.slice(tab + 1) : null;
      // path = new (real, openable/stageable) path; oldPath = original.
      entries.push(...makeEntries(parts[1] ?? "  ", newPath, origPath, false));
    } else if (line.startsWith("u ")) {
      // u XY sub m1 m2 m3 mW h1 h2 h3 path  (unmerged → conflicted)
      const parts = line.split(" ", 11);
      entries.push(...makeEntries(parts[1] ?? "  ", parts[10] ?? "", null, true));
    } else if (line.startsWith("? ")) {
      const path = line.slice(2);
      entries.push({
        path,
        oldPath: null,
        xy: "??",
        status: "untracked",
        staged: false,
      });
    }
    // "!" (ignored) lines are skipped.
  }
  return { branch, ahead, behind, entries };
}

function makeEntries(
  xy: string,
  path: string,
  oldPath: string | null,
  conflicted: boolean,
): GitStatusEntry[] {
  if (conflicted) {
    return [{ path, oldPath, xy, status: "conflicted", staged: false }];
  }
  const X = xy[0] ?? " ";
  const Y = xy[1] ?? " ";
  const entries: GitStatusEntry[] = [];
  if (X !== " " && X !== "?") {
    entries.push({ path, oldPath, xy, status: statusFromCode(X), staged: true });
  }
  if (Y !== " " && Y !== "?") {
    entries.push({ path, oldPath, xy, status: statusFromCode(Y), staged: false });
  }
  return entries;
}

function statusFromCode(code: string): GitStatusEntry["status"] {
  if (code === "R" || code === "C") return "renamed";
  if (code === "D") return "deleted";
  if (code === "A") return "added";
  return "modified";
}

async function detectOperations(workspace: string): Promise<GitOperation[]> {
  const ops: GitOperation[] = [];
  const checks: Array<[string, GitOperation]> = [
    ["MERGE_HEAD", "merge"],
    ["REBASE_HEAD", "rebase"],
    ["CHERRY_PICK_HEAD", "cherry-pick"],
    ["REVERT_HEAD", "revert"],
  ];
  await Promise.all(
    checks.map(async ([ref, name]) => {
      const r = await runGit(workspace, ["rev-parse", "-q", "--verify", ref]);
      if (r.code === 0 && !ops.includes(name)) ops.push(name);
    }),
  );
  // Some rebases expose dirs without REBASE_HEAD yet.
  const [mergePath, applyPath] = await Promise.all([
    runGit(workspace, ["rev-parse", "--git-path", "rebase-merge"]),
    runGit(workspace, ["rev-parse", "--git-path", "rebase-apply"]),
  ]);
  for (const pathRes of [mergePath, applyPath]) {
    const p = pathRes.stdout.trim();
    const abs = p ? (isAbsolute(p) ? p : resolve(workspace, p)) : "";
    if (abs && existsSync(abs) && !ops.includes("rebase")) ops.push("rebase");
  }
  return ops;
}

/** Build the full GitStatus for a workspace (status + HEAD commit). */
async function buildStatus(workspace: string): Promise<GitStatus> {
  const statusRes = await runGit(workspace, [
    "status",
    "--porcelain=v2",
    "--branch",
  ]);
  if (statusRes.code !== 0) {
    if (isNotARepo(statusRes.stderr)) {
      return {
        branch: "",
        ahead: 0,
        behind: 0,
        entries: [],
        head: null,
        bare: true,
        operations: [],
      };
    }
    throw new Error(
      statusRes.stderr.trim() ||
        statusRes.stdout.trim() ||
        `git status failed (exit ${statusRes.code})`,
    );
  }
  const { branch, ahead, behind, entries } = parsePorcelain(statusRes.stdout);

  const [
    headRes,
    upstreamRes,
    branchesRes,
    commitsRes,
    stashesRes,
    tagsRes,
    remotesRes,
    operations,
  ] = await Promise.all([
    runGit(workspace, ["log", "-1", "--format=%h%x09%s%x09%an%x09%ct"]),
    runGit(workspace, ["rev-parse", "--abbrev-ref", "--symbolic-full-name", "@{upstream}"]),
    runGit(workspace, [
      "for-each-ref",
      "--sort=-committerdate",
      "--format=%(refname:short)%00%(objectname:short)%00%(HEAD)%00%(upstream:short)%00%(upstream:track,nobracket)%00%(refname)",
      "refs/heads",
      "refs/remotes",
    ]),
    runGit(workspace, [
      "log",
      "--all",
      "-n",
      "100",
      "--date-order",
      "--format=%H%x1f%h%x1f%P%x1f%s%x1f%an%x1f%ae%x1f%ct%x1f%D%x1e",
    ]),
    runGit(workspace, ["stash", "list", "--format=%gd%x1f%H%x1f%s%x1f%ct%x1e"]),
    runGit(workspace, [
      "for-each-ref",
      "--sort=-creatordate",
      "--format=%(refname:short)%00%(objectname:short)%00%(subject)",
      "refs/tags",
    ]),
    runGit(workspace, ["remote", "-v"]),
    detectOperations(workspace),
  ]);

  let head: GitStatus["head"] = null;
  if (headRes.code === 0 && headRes.stdout.trim()) {
    const [oid, message, author, ts] = headRes.stdout.trim().split("\t");
    head = {
      oid: oid ?? "",
      message: message ?? "",
      author: author ?? "",
      ts: Number(ts ?? 0) * 1000,
    };
  }

  return {
    branch: branch || "(unknown)",
    ahead,
    behind,
    entries,
    head,
    bare: false,
    upstream: upstreamRes.code === 0 ? upstreamRes.stdout.trim() || null : null,
    branches: parseBranches(branchesRes.stdout),
    commits: parseCommits(commitsRes.stdout),
    stashes: parseStashes(stashesRes.stdout),
    tags: parseTags(tagsRes.stdout),
    remotes: parseRemotes(remotesRes.stdout),
    operations,
  };
}

function parseBranches(stdout: string): GitBranch[] {
  return stdout.split("\n").filter(Boolean).map((line) => {
    const [name = "", oid = "", marker = "", upstream = "", track = "", ref = ""] =
      line.split("\0");
    return {
      name,
      oid,
      current: marker === "*",
      remote: ref.startsWith("refs/remotes/"),
      upstream: upstream || null,
      ahead: Number(track.match(/ahead (\d+)/)?.[1] ?? 0),
      behind: Number(track.match(/behind (\d+)/)?.[1] ?? 0),
    };
  }).filter((branch) => branch.name && !branch.name.endsWith("/HEAD"));
}

function parseCommits(stdout: string): GitCommit[] {
  return stdout.split("\x1e").map((record) => record.trim()).filter(Boolean).map((record) => {
    const [oid = "", shortOid = "", parents = "", subject = "", author = "", email = "", ts = "0", refs = ""] =
      record.split("\x1f");
    return {
      oid,
      shortOid,
      parents: parents ? parents.split(" ") : [],
      subject,
      author,
      email,
      ts: Number(ts) * 1000,
      refs: refs ? refs.split(", ").map((ref) => ref.replace(/^HEAD -> /, "")) : [],
    };
  });
}

function parseStashes(stdout: string): GitStash[] {
  return stdout.split("\x1e").map((record) => record.trim()).filter(Boolean).map((record) => {
    const [ref = "", oid = "", subject = "", ts = "0"] = record.split("\x1f");
    return { ref, oid, subject, ts: Number(ts) * 1000 };
  });
}

function parseTags(stdout: string): GitTag[] {
  return stdout.split("\n").filter(Boolean).map((line) => {
    const [name = "", oid = "", subject = ""] = line.split("\0");
    return { name, oid, subject };
  }).filter((tag) => tag.name);
}

function parseRemotes(stdout: string): GitRemote[] {
  const remotes = new Map<string, GitRemote>();
  for (const line of stdout.split("\n")) {
    const match = line.match(/^(\S+)\s+(.+)\s+\((fetch|push)\)$/);
    if (!match) continue;
    const [, name, url, kind] = match;
    const remote = remotes.get(name) ?? { name, fetchUrl: "", pushUrl: "" };
    if (kind === "fetch") remote.fetchUrl = url;
    else remote.pushUrl = url;
    remotes.set(name, remote);
  }
  return [...remotes.values()];
}

function requirePath(p: unknown): string {
  if (typeof p !== "string" || !p) throw new Error("missing path");
  return p;
}

function requireMessage(m: unknown): string {
  if (typeof m !== "string" || !m.trim()) throw new Error("missing commit message");
  return m;
}

function optionalMessage(m: unknown): string | null {
  if (typeof m !== "string") return null;
  const trimmed = m.trim();
  return trimmed || null;
}

function requireBranch(b: unknown): string {
  if (typeof b !== "string" || !b) throw new Error("missing branch");
  // Sane ref name only (no leading option-injection, no shell metacharacters —
  // execFile already prevents shell injection, this guards git arg injection).
  if (!/^[A-Za-z0-9._/-]+$/.test(b) || b.startsWith("-")) {
    throw new Error("invalid branch name");
  }
  return b;
}

function requireRef(value: unknown, label = "ref"): string {
  if (typeof value !== "string" || !value.trim() || value.startsWith("-")) {
    throw new Error(`invalid ${label}`);
  }
  if (!/^[A-Za-z0-9._/@{}:+~-]+$/.test(value)) throw new Error(`invalid ${label}`);
  return value;
}

function requireUrl(value: unknown): string {
  if (typeof value !== "string" || !value.trim() || value.startsWith("-")) {
    throw new Error("invalid remote URL");
  }
  const url = value.trim();
  // Block git "ext::" / "fd::" helpers and other non-URL schemes that can RCE.
  if (/^(ext|fd|hg|bzr):/i.test(url) || url.includes("::")) {
    throw new Error("unsupported remote URL scheme");
  }
  if (
    /^(https?:\/\/|git:\/\/|ssh:\/\/|git@)/i.test(url) ||
    /^[A-Za-z0-9_.-]+@[A-Za-z0-9_.-]+:/.test(url)
  ) {
    return url;
  }
  throw new Error("invalid remote URL");
}

function oneOf<T extends string>(value: unknown, allowed: readonly T[], fallback: T): T {
  return typeof value === "string" && allowed.includes(value as T) ? value as T : fallback;
}

/** Build the git arg array for an action. Returns null for unknown actions. */
function argsForAction(
  action: string,
  body: Record<string, unknown>,
): string[] | null {
  switch (action) {
    case "stage":
      return ["add", "--", requirePath(body.path)];
    case "unstage":
      return ["restore", "--staged", "--", requirePath(body.path)];
    case "stageAll":
      return ["add", "-A"];
    case "unstageAll":
      return ["reset", "-q"];
    case "commit": {
      const amend = body.amend === true;
      const msg = optionalMessage(body.message);
      if (!amend && !msg) throw new Error("missing commit message");
      return [
        "commit",
        ...(amend ? ["--amend"] : []),
        ...(body.signoff === true ? ["--signoff"] : []),
        ...(body.noVerify === true ? ["--no-verify"] : []),
        ...(msg ? ["-m", msg] : ["--no-edit"]),
      ];
    }
    case "pull":
      return ["pull", oneOf(body.mode, ["--rebase", "--no-rebase", "--ff-only"] as const, "--no-rebase"), "--no-edit"];
    case "push":
      return [
        "push",
        ...(body.forceWithLease === true ? ["--force-with-lease"] : []),
        ...(body.tags === true ? ["--tags"] : []),
        ...(body.setUpstream === true
          ? ["--set-upstream", requireRef(body.remote ?? "origin", "remote"), requireBranch(body.branch)]
          : []),
      ];
    case "fetch":
      return ["fetch", ...(body.all === true ? ["--all"] : []), ...(body.prune === true ? ["--prune"] : [])];
    case "checkout":
      return ["checkout", requireRef(body.branch, "ref")];
    case "discard":
      return ["restore", "--", requirePath(body.path)];
    case "discardAll":
      return ["restore", "."];
    case "clean":
      return ["clean", "-fd", ...(body.includeIgnored === true ? ["-x"] : [])];
    case "createBranch":
      return body.checkout === false
        ? ["branch", requireBranch(body.branch), ...(body.startPoint ? [requireRef(body.startPoint, "start point")] : [])]
        : ["switch", "-c", requireBranch(body.branch), ...(body.startPoint ? [requireRef(body.startPoint, "start point")] : [])];
    case "deleteBranch":
      return ["branch", body.force === true ? "-D" : "-d", requireBranch(body.branch)];
    case "renameBranch":
      return ["branch", "-m", requireBranch(body.branch), requireBranch(body.newName)];
    case "merge":
      return ["merge", ...(body.noFf === true ? ["--no-ff"] : []), ...(body.squash === true ? ["--squash"] : []), requireRef(body.ref)];
    case "rebase":
      return ["rebase", requireRef(body.ref)];
    case "continue":
      return [oneOf(body.operation, ["merge", "rebase", "cherry-pick", "revert"] as const, "rebase"), "--continue"];
    case "abort":
      return [oneOf(body.operation, ["merge", "rebase", "cherry-pick", "revert"] as const, "rebase"), "--abort"];
    case "cherryPick":
      return ["cherry-pick", requireRef(body.ref)];
    case "revert":
      return ["revert", "--no-edit", requireRef(body.ref)];
    case "reset":
      return ["reset", `--${oneOf(body.mode, ["soft", "mixed", "hard"] as const, "mixed")}`, requireRef(body.ref)];
    case "stashPush":
      return ["stash", "push", ...(body.includeUntracked === true ? ["--include-untracked"] : []), ...(body.keepIndex === true ? ["--keep-index"] : []), ...(typeof body.message === "string" && body.message.trim() ? ["-m", body.message.trim()] : [])];
    case "stashApply":
      return ["stash", body.pop === true ? "pop" : "apply", ...(body.index === true ? ["--index"] : []), requireRef(body.ref)];
    case "stashDrop":
      return ["stash", "drop", requireRef(body.ref)];
    case "stashBranch":
      return ["stash", "branch", requireBranch(body.branch), requireRef(body.ref)];
    case "tagCreate":
      return ["tag", ...(body.annotated === true ? ["-a", "-m", requireMessage(body.message)] : []), requireRef(body.tag, "tag"), ...(body.ref ? [requireRef(body.ref)] : [])];
    case "tagDelete":
      return ["tag", "-d", requireRef(body.tag, "tag")];
    case "tagPush":
      return ["push", requireRef(body.remote ?? "origin", "remote"), requireRef(body.tag, "tag")];
    case "remoteAdd":
      return ["remote", "add", requireRef(body.remote, "remote"), requireUrl(body.url)];
    case "remoteRemove":
      return ["remote", "remove", requireRef(body.remote, "remote")];
    case "remoteRename":
      return ["remote", "rename", requireRef(body.remote, "remote"), requireRef(body.newName, "remote")];
    case "remoteSetUrl":
      return ["remote", "set-url", requireRef(body.remote, "remote"), requireUrl(body.url)];
    case "setUpstream":
      return ["branch", "--set-upstream-to", requireRef(body.upstream, "upstream"), requireBranch(body.branch)];
    case "unsetUpstream":
      return ["branch", "--unset-upstream", requireBranch(body.branch)];
    case "init":
      return ["init"];
    case "ignore":
      // Handled specially in POST (appends to .gitignore).
      return [];
    default:
      return null;
  }
}

export async function GET(req: Request) {
  if (!(await getSession(req.headers))) {
    return Response.json({ error: "unauthorized" }, { status: 401 });
  }
  let workspace: string;
  try {
    workspace = resolveAuthorizedWorkspace(req);
  } catch {
    return Response.json({ error: "unauthorized workspace" }, { status: 403 });
  }
  const url = new URL(req.url);
  const diffPath = url.searchParams.get("diff");
  const sidesPath = url.searchParams.get("sides");
  const commitOid = url.searchParams.get("commit");
  const stashRef = url.searchParams.get("stash");

  // Commit detail: unified patch for one commit.
  if (commitOid !== null) {
    try {
      const oid = requireRef(commitOid, "commit");
      const r = await runGit(workspace, [
        "show",
        "--format=fuller",
        "--stat",
        "--patch",
        "--find-renames",
        oid,
      ]);
      if (r.code !== 0) {
        return Response.json(
          { error: `git failed: ${r.stderr.trim() || r.stdout.trim()}` },
          { status: 502 },
        );
      }
      let patch = r.stdout;
      if (patch.length > MAX_DIFF) {
        patch = patch.slice(0, MAX_DIFF) + "\n... (patch truncated)";
      }
      return Response.json({ patch });
    } catch (e) {
      const msg = e instanceof Error ? e.message : String(e);
      return Response.json({ error: msg }, { status: 400 });
    }
  }

  // Stash patch preview.
  if (stashRef !== null) {
    try {
      const ref = requireRef(stashRef, "stash");
      const r = await runGit(workspace, ["stash", "show", "-p", "--stat", ref]);
      if (r.code !== 0) {
        return Response.json(
          { error: `git failed: ${r.stderr.trim() || r.stdout.trim()}` },
          { status: 502 },
        );
      }
      let patch = r.stdout;
      if (patch.length > MAX_DIFF) {
        patch = patch.slice(0, MAX_DIFF) + "\n... (patch truncated)";
      }
      return Response.json({ patch });
    } catch (e) {
      const msg = e instanceof Error ? e.message : String(e);
      return Response.json({ error: msg }, { status: 400 });
    }
  }

  // Two-sided content for Monaco DiffEditor (original vs modified).
  // staged=1 → HEAD vs index; staged=0 → index vs worktree.
  if (sidesPath !== null) {
    try {
      confinePath(workspace, sidesPath);
      const staged = url.searchParams.get("staged") === "1";
      const sides = await readDiffSides(workspace, sidesPath, staged);
      return Response.json(sides);
    } catch (e) {
      const msg = e instanceof Error ? e.message : String(e);
      return Response.json(
        { error: msg },
        { status: msg === "path outside workspace" ? 400 : 500 },
      );
    }
  }

  // Diff mode: return the unified diff for one path (task-driven extension).
  if (diffPath !== null) {
    try {
      confinePath(workspace, diffPath); // validate confinement
      const staged = url.searchParams.get("staged") === "1";
      const r = await runGit(workspace, [
        "diff",
        ...(staged ? ["--cached"] : []),
        "--",
        diffPath,
      ]);
      if (r.code !== 0 && r.code !== 1) {
        return Response.json(
          { error: `git failed: ${r.stderr.trim() || r.stdout.trim()}` },
          { status: 502 },
        );
      }
      let diff = r.stdout;
      if (diff.length > MAX_DIFF) {
        diff = diff.slice(0, MAX_DIFF) + "\n... (diff truncated)";
      }
      return Response.json({ diff });
    } catch (e) {
      const msg = e instanceof Error ? e.message : String(e);
      return Response.json(
        { error: msg },
        { status: msg === "path outside workspace" ? 400 : 500 },
      );
    }
  }

  try {
    const status = await buildStatus(workspace);
    return Response.json(status);
  } catch (e) {
    const msg = e instanceof Error ? e.message : String(e);
    return Response.json({ error: `git failed: ${msg}` }, { status: 502 });
  }
}

function appendGitignore(workspace: string, relPath: string): void {
  const absIgnore = join(workspace, ".gitignore");
  const line = relPath.replace(/\\/g, "/");
  if (!line || line.includes("\n") || line.includes("\0")) {
    throw new Error("invalid ignore path");
  }
  let existing = "";
  if (existsSync(absIgnore)) {
    existing = readFileSync(absIgnore, "utf8");
    const lines = existing.split(/\r?\n/);
    if (lines.some((l) => l.trim() === line)) return;
    if (existing.length > 0 && !existing.endsWith("\n")) {
      appendFileSync(absIgnore, "\n");
    }
  } else {
    writeFileSync(absIgnore, "", "utf8");
  }
  appendFileSync(absIgnore, `${line}\n`);
}

export async function POST(req: Request) {
  if (!(await getSession(req.headers))) {
    return Response.json({ error: "unauthorized" }, { status: 401 });
  }
  let body: Record<string, unknown> = {};
  try {
    body = (await req.json()) as Record<string, unknown>;
  } catch {
    body = {};
  }
  const action = typeof body.action === "string" ? body.action : "";
  let workspace: string;
  try {
    workspace = resolveAuthorizedWorkspace(
      req,
      typeof body.workspace === "string" ? body.workspace : null,
    );
  } catch {
    return Response.json({ error: "unauthorized workspace" }, { status: 403 });
  }

  let args: string[] | null;
  try {
    args = argsForAction(action, body);
  } catch (e) {
    const msg = e instanceof Error ? e.message : String(e);
    // missing path/message/branch or invalid branch name → bad request.
    return Response.json({ error: msg }, { status: 400 });
  }
  if (args === null) {
    return Response.json({ error: "unknown action" }, { status: 400 });
  }

  // Confine any path-bearing action's path (stage/unstage/discard/ignore) so a
  // crafted relative path cannot escape the workspace.
  try {
    if (action === "stage" || action === "unstage" || action === "discard" || action === "ignore") {
      const rel = body.path as string;
      confinePath(workspace, rel);
    }
  } catch (e) {
    const msg = e instanceof Error ? e.message : String(e);
    return Response.json({ error: msg }, { status: 400 });
  }

  if (action === "ignore") {
    try {
      const rel = requirePath(body.path);
      confinePath(workspace, rel);
      appendGitignore(workspace, rel);
      const stageIgnore = await runGit(workspace, ["add", "--", ".gitignore"]);
      if (stageIgnore.code !== 0) {
        return Response.json(
          {
            error: `git failed: ${stageIgnore.stderr.trim() || stageIgnore.stdout.trim() || `exit ${stageIgnore.code}`}`,
          },
          { status: 502 },
        );
      }
    } catch (e) {
      const msg = e instanceof Error ? e.message : String(e);
      return Response.json({ error: msg }, { status: 400 });
    }
  } else {
    const r = await runGit(workspace, args);
    if (r.code !== 0) {
      return Response.json(
        {
          error: `git failed: ${r.stderr.trim() || r.stdout.trim() || `exit ${r.code}`}`,
        },
        { status: 502 },
      );
    }
  }

  // Re-run status so the UI updates atomically after every successful action.
  try {
    const status = await buildStatus(workspace);
    return Response.json({ ok: true, status });
  } catch (e) {
    const msg = e instanceof Error ? e.message : String(e);
    return Response.json({ error: `git failed: ${msg}` }, { status: 502 });
  }
}
