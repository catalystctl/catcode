// POST /api/project — create a new project folder or clone a repository.
//
//   POST /api/project  body: { action: "create", parentDir, name, initGit?, createReadme? }
//   → 200 { ok: true, path, name }
//   → 400 { error: "invalid name" | "destination exists" | … }
//
//   POST /api/project  body: { action: "clone", url, parentDir, name?, branch? }
//   → 200 { ok: true, path, name }
//   → 400/502 { error: "invalid url" | "clone failed: …" }
//
// Creating a project: validates the name, refuses unsafe characters, refuses
// paths outside the user's home subtree (defense in depth), creates the
// directory (and missing parents), optionally `git init`s + creates a README,
// then registers it via the bridge so it appears in Recent and is switchable.
//
// Cloning: validates the URL, refuses a non-empty destination, runs
// `git clone --recurse-submodules` (cancelled if the name is taken), then
// registers the result. Clone errors surface the real git stderr so the user
// can act on auth failures without secrets leaking.
import { existsSync, mkdirSync, writeFileSync, accessSync, constants } from "node:fs";
import { join, resolve, basename, dirname } from "node:path";
import { homedir } from "node:os";
import { execFile } from "node:child_process";
import { getSession } from "@/lib/auth";
import { touchProject } from "@/lib/projects";
import { validateProjectName, validateCloneUrl, nameFromUrl, isUnderHome } from "@/lib/project-validate";

export const dynamic = "force-dynamic";
export const runtime = "nodejs";

const MAX_BUFFER = 8 * 1024 * 1024;

function runGit(cwd: string, args: string[]): Promise<{ code: number; stdout: string; stderr: string }> {
  return new Promise((resolve) => {
    execFile("git", args, { cwd, maxBuffer: MAX_BUFFER }, (err, stdout, stderr) => {
      let code = 0;
      let errOut = typeof stderr === "string" ? stderr : "";
      if (err) {
        const c = (err as { code?: unknown }).code;
        code = typeof c === "number" ? c : -1;
        if (!errOut && err.message) errOut = err.message;
      }
      resolve({ code, stdout: typeof stdout === "string" ? stdout : "", stderr: errOut });
    });
  });
}

/** Defense-in-depth: keep created/cloned projects under the user's home tree. */
function assertUnderHome(abs: string): void {
  const home = resolve(homedir() || "/");
  if (!isUnderHome(abs, home)) {
    throw new Error("destination must be inside your home directory");
  }
}
function dirIsEmpty(abs: string): boolean {
  try {
    return require("node:fs").readdirSync(abs).length === 0;
  } catch {
    return true;
  }
}

function tryAccess(abs: string): boolean {
  try {
    accessSync(abs, constants.W_OK);
    return true;
  } catch {
    return false;
  }
}

export async function POST(req: Request) {
  if (!(await getSession(req.headers)))
    return Response.json({ error: "unauthorized" }, { status: 401 });

  let body: Record<string, unknown>;
  try {
    body = await req.json();
  } catch {
    return Response.json({ error: "invalid body" }, { status: 400 });
  }

  const action = typeof body.action === "string" ? body.action : "";
  try {
    if (action === "create") {
      const name = validateProjectName(typeof body.name === "string" ? body.name : "");
      const parentRaw = typeof body.parentDir === "string" && body.parentDir.trim()
        ? body.parentDir
        : homedir();
      const parentAbs = resolve(parentRaw.replace(/^~(?=$|\/|\\)/, homedir()));
      assertUnderHome(parentAbs);
      if (!existsSync(parentAbs)) {
        try {
          mkdirSync(parentAbs, { recursive: true });
        } catch (e) {
          return Response.json({ error: `could not create parent directory: ${(e as Error).message}` }, { status: 400 });
        }
      }
      if (!tryAccess(parentAbs)) {
        return Response.json({ error: "parent directory is not writable" }, { status: 403 });
      }
      const projectAbs = join(parentAbs, name);
      if (existsSync(projectAbs)) {
        return Response.json({ error: "a folder with this name already exists — open it instead", exists: true, path: projectAbs }, { status: 409 });
      }
      try {
        mkdirSync(projectAbs, { recursive: true });
      } catch (e) {
        return Response.json({ error: `could not create project folder: ${(e as Error).message}` }, { status: 500 });
      }

      if (body.createReadme === true) {
        try {
          writeFileSync(join(projectAbs, "README.md"), `# ${name}\n\n`, "utf8");
        } catch {
          /* non-fatal */
        }
      }

      if (body.initGit === true) {
        const r = await runGit(projectAbs, ["init"]);
        if (r.code !== 0) {
          // Folder was created; git init failure is non-fatal but reported.
          return Response.json({ ok: true, path: projectAbs, name, warning: `folder created, but git init failed: ${r.stderr.trim() || r.stdout.trim()}` });
        }
      }

      touchProject(projectAbs);
      return Response.json({ ok: true, path: projectAbs, name });
    }

    if (action === "clone") {
      const url = validateCloneUrl(typeof body.url === "string" ? body.url : "");
      const parentRaw = typeof body.parentDir === "string" && body.parentDir.trim()
        ? body.parentDir
        : homedir();
      const parentAbs = resolve(parentRaw.replace(/^~(?=$|\/|\\)/, homedir()));
      assertUnderHome(parentAbs);
      if (!existsSync(parentAbs)) {
        try {
          mkdirSync(parentAbs, { recursive: true });
        } catch (e) {
          return Response.json({ error: `could not create parent directory: ${(e as Error).message}` }, { status: 400 });
        }
      }
      const folderName = validateProjectName(
        typeof body.name === "string" && body.name.trim() ? body.name : nameFromUrl(url),
      );
      const projectAbs = join(parentAbs, folderName);
      if (existsSync(projectAbs) && !dirIsEmpty(projectAbs)) {
        return Response.json({ error: `destination “${folderName}” already exists and is not empty` }, { status: 409 });
      }
      if (existsSync(projectAbs) && !tryAccess(projectAbs)) {
        return Response.json({ error: "destination directory is not writable" }, { status: 403 });
      }

      const args = ["clone"];
      if (typeof body.branch === "string" && body.branch.trim()) {
        args.push("--branch", body.branch.trim());
      }
      args.push("--recurse-submodules", url, projectAbs);

      const r = await runGit(parentAbs, args);
      if (r.code !== 0) {
        const msg = r.stderr.trim() || r.stdout.trim() || `git clone exited ${r.code}`;
        return Response.json({ error: `clone failed: ${msg}` }, { status: 502 });
      }

      touchProject(projectAbs);
      return Response.json({ ok: true, path: projectAbs, name: folderName });
    }

    return Response.json({ error: "unknown action" }, { status: 400 });
  } catch (e) {
    const msg = e instanceof Error ? e.message : String(e);
    return Response.json({ error: msg }, { status: 400 });
  }
}
