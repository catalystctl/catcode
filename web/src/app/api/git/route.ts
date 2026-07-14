// Git status + actions for the IDE Source Control panel. User-driven (NOT an
// agent turn) — direct `git` CLI invocations scoped to the workspace cwd.
//
// GET  /api/git?workspace=<abs>                 → GitStatus
// GET  /api/git?workspace=<abs>&diff=<rel>&staged=<0|1>  → { diff }   (task-driven
//      extension for the panel's click→diff-view; absent in the planner contract
//      §4.3 which only specifies status. Documented here as a minimal addition.)
// POST /api/git  body: { action, path?, message?, branch?, workspace? }
//      action: stage|unstage|stageAll|unstageAll|commit|pull|push|fetch|
//              checkout|discard|init  → { ok, status: GitStatus }
//
// All git commands run via execFile (arg arrays — never a shell, so no
// injection). File-path actions are confined via confinePath. Auth required on
// every entry. stdout/stderr captured; no secrets are logged (commit messages
// and paths are not logged at all).

import { execFile } from "node:child_process";
import { getSession } from "@/lib/auth";
import { resolveWorkspace, confinePath } from "@/server/workspace";
import type { GitStatus, GitStatusEntry } from "@/lib/types";

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
      entries.push(makeEntry(parts[1] ?? "  ", parts[8] ?? "", null, false));
    } else if (line.startsWith("2 ")) {
      // 2 XY sub mH mI mW hH hI Xscore path\torigPath
      const parts = line.split(" ", 10);
      const tail = parts[9] ?? "";
      const tab = tail.indexOf("\t");
      const newPath = tab >= 0 ? tail.slice(0, tab) : tail;
      const origPath = tab >= 0 ? tail.slice(tab + 1) : null;
      // path = new (real, openable/stageable) path; oldPath = original.
      entries.push(makeEntry(parts[1] ?? "  ", newPath, origPath, false));
    } else if (line.startsWith("u ")) {
      // u XY sub m1 m2 m3 mW h1 h2 h3 path  (unmerged → conflicted)
      const parts = line.split(" ", 11);
      entries.push(makeEntry(parts[1] ?? "  ", parts[10] ?? "", null, true));
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

function makeEntry(
  xy: string,
  path: string,
  oldPath: string | null,
  conflicted: boolean,
): GitStatusEntry {
  if (conflicted) {
    return { path, oldPath, xy, status: "conflicted", staged: false };
  }
  const X = xy[0] ?? " ";
  const Y = xy[1] ?? " ";
  let status: GitStatusEntry["status"];
  if (X === "R" || Y === "R" || X === "C" || Y === "C") status = "renamed";
  else if (X === "D" || Y === "D") status = "deleted";
  else if (X === "A" || Y === "A") status = "added";
  else status = "modified"; // M, T, or anything else
  const staged = X !== " " && X !== "?";
  return { path, oldPath, xy, status, staged };
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
      return { branch: "", ahead: 0, behind: 0, entries: [], head: null, bare: true };
    }
    throw new Error(
      statusRes.stderr.trim() ||
        statusRes.stdout.trim() ||
        `git status failed (exit ${statusRes.code})`,
    );
  }
  const { branch, ahead, behind, entries } = parsePorcelain(statusRes.stdout);

  let head: GitStatus["head"] = null;
  const headRes = await runGit(workspace, [
    "log",
    "-1",
    "--format=%h%x09%s%x09%an%x09%ct",
  ]);
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
  };
}

function requirePath(p: unknown): string {
  if (typeof p !== "string" || !p) throw new Error("missing path");
  return p;
}

function requireMessage(m: unknown): string {
  if (typeof m !== "string" || !m.trim()) throw new Error("missing commit message");
  return m;
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
    case "commit":
      return ["commit", "-m", requireMessage(body.message)];
    case "pull":
      return ["pull", "--no-edit"];
    case "push":
      return ["push"];
    case "fetch":
      return ["fetch"];
    case "checkout":
      return ["checkout", requireBranch(body.branch)];
    case "discard":
      return ["restore", "--", requirePath(body.path)];
    case "init":
      return ["init"];
    default:
      return null;
  }
}

export async function GET(req: Request) {
  if (!(await getSession(req.headers))) {
    return Response.json({ error: "unauthorized" }, { status: 401 });
  }
  const workspace = resolveWorkspace(req);
  const url = new URL(req.url);
  const diffPath = url.searchParams.get("diff");

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
  const workspace =
    (typeof body.workspace === "string" && body.workspace) ||
    resolveWorkspace(req);

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

  // Confine any path-bearing action's path (stage/unstage/discard) so a crafted
  // relative path cannot escape the workspace. confinePath throws on escape.
  try {
    if (action === "stage" || action === "unstage" || action === "discard") {
      const rel = body.path as string;
      confinePath(workspace, rel);
    }
  } catch (e) {
    const msg = e instanceof Error ? e.message : String(e);
    return Response.json({ error: msg }, { status: 400 });
  }

  const r = await runGit(workspace, args);
  if (r.code !== 0) {
    return Response.json(
      {
        error: `git failed: ${r.stderr.trim() || r.stdout.trim() || `exit ${r.code}`}`,
      },
      { status: 502 },
    );
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
