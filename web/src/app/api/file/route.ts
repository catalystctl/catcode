// GET / PUT /api/file — read and write a single workspace file.
// Per docs/IDE_PANELS_CONTRACT.md §4.2.
//
//   GET /api/file?path=<rel>&workspace=<abs>
//   → 200 { path, content, size, language?, mtime? }
//   → 400 { error: "path outside workspace" | "path is a directory" | "file too large" }
//   → 404 { error: "not found" }
//   → 401 { error: "unauthorized" }
//
//   PUT /api/file  body: { path: string; content: string; workspace?: string }
//   → 200 { ok: true, path, size }
//   → 400 { error: "path outside workspace" | "invalid body" }
//   → 401 { error: "unauthorized" }
//
// Both confined via confinePath (mirrors api/files/route.ts). No secret filtering
// (§4.2/§8.5: the user edits their own workspace — VSCode parity, you can edit
// .env). Reads are capped at 5 MiB so binaries aren't loaded into the editor.
import { closeSync, existsSync, mkdirSync, openSync, readFileSync, realpathSync, renameSync, rmSync, statSync, writeFileSync } from "node:fs";
import { dirname, relative, resolve, sep } from "node:path";
import { getSession } from "@/lib/auth";
import {
  authorizedWorkspace,
  confinePath,
  confinePathReal,
  resolveAuthorizedWorkspace,
  resolveWorkspace,
} from "@/server/workspace";
import { detectLanguage } from "@/lib/lang";

export const dynamic = "force-dynamic";
export const runtime = "nodejs";

const MAX_BYTES = 5 * 1024 * 1024; // 5 MiB

function mutationPath(workspace: string, rel: string, existing: boolean): string {
  const abs = confinePath(workspace, rel);
  if (resolve(abs) === resolve(workspace)) throw new Error("workspace root is protected");
  const realWorkspace = realpathSync(workspace);
  const realTarget = existing ? realpathSync(abs) : realpathSync(dirname(abs));
  const confined = relative(realWorkspace, realTarget);
  if (confined === ".." || confined.startsWith(`..${sep}`)) throw new Error("path outside workspace");
  return abs;
}

export async function GET(req: Request) {
  if (!(await getSession(req.headers)))
    return Response.json({ error: "unauthorized" }, { status: 401 });

  const url = new URL(req.url);
  let workspace: string;
  try {
    workspace = resolveAuthorizedWorkspace(req);
  } catch {
    return Response.json({ error: "unauthorized workspace" }, { status: 403 });
  }
  const rel = url.searchParams.get("path") ?? "";

  let abs: string;
  try {
    abs = confinePathReal(workspace, rel);
  } catch {
    return Response.json({ error: "path outside workspace" }, { status: 400 });
  }

  let st;
  try {
    st = statSync(abs);
  } catch {
    return Response.json({ error: "not found" }, { status: 404 });
  }
  if (st.isDirectory())
    return Response.json({ error: "path is a directory" }, { status: 400 });
  if (st.size > MAX_BYTES)
    return Response.json({ error: "file too large" }, { status: 400 });

  let content: string;
  try {
    content = readFileSync(abs, "utf8");
  } catch {
    return Response.json({ error: "not readable" }, { status: 400 });
  }

  return Response.json({
    path: rel,
    content,
    size: st.size,
    mtime: st.mtimeMs,
    language: detectLanguage(rel),
  });
}

export async function PUT(req: Request) {
  if (!(await getSession(req.headers)))
    return Response.json({ error: "unauthorized" }, { status: 401 });

  let body: { path?: unknown; content?: unknown; workspace?: unknown };
  try {
    body = await req.json();
  } catch {
    return Response.json({ error: "invalid body" }, { status: 400 });
  }

  const rel = body.path;
  const content = body.content;
  if (typeof rel !== "string" || typeof content !== "string")
    return Response.json({ error: "invalid body" }, { status: 400 });

  // Workspace: explicit body field, else ?workspace= query, else default.
  let workspace: string;
  try {
    workspace = authorizedWorkspace(
      (typeof body.workspace === "string" && body.workspace) ||
      new URL(req.url).searchParams.get("workspace") ||
      resolveWorkspace(req),
    );
  } catch {
    return Response.json({ error: "unauthorized workspace" }, { status: 403 });
  }

  let abs: string;
  try {
    // Resolve + confine first. Only mkdir after the parent is verified under the
    // real workspace — never create directories before the realpath check.
    const confined = confinePath(workspace, rel);
    const existing = existsSync(confined);
    abs = mutationPath(workspace, rel, existing);
    if (!existing) mkdirSync(dirname(abs), { recursive: true });
  } catch {
    return Response.json({ error: "file not found or path outside workspace" }, { status: 400 });
  }

  try {
    writeFileSync(abs, content, "utf8");
  } catch (e) {
    return Response.json({ error: `write failed: ${(e as Error).message}` }, { status: 500 });
  }

  let size = content.length;
  try {
    size = statSync(abs).size;
  } catch {
    /* fall back to byte length */
  }
  return Response.json({ ok: true, path: rel, size });
}

/** Create a file or directory without overwriting an existing entry. */
export async function POST(req: Request) {
  if (!(await getSession(req.headers)))
    return Response.json({ error: "unauthorized" }, { status: 401 });
  let body: { path?: unknown; workspace?: unknown; kind?: unknown };
  try { body = await req.json(); } catch { return Response.json({ error: "invalid body" }, { status: 400 }); }
  if (typeof body.path !== "string" || !body.path.trim())
    return Response.json({ error: "invalid body" }, { status: 400 });
  if (body.kind !== "file" && body.kind !== "dir" && body.kind !== "folder")
    return Response.json({ error: "kind must be \"file\" or \"dir\"" }, { status: 400 });
  const kind = body.kind === "folder" ? "dir" : body.kind;
  let workspace: string;
  try {
    workspace = authorizedWorkspace(typeof body.workspace === "string" && body.workspace ? body.workspace : resolveWorkspace(req));
  } catch {
    return Response.json({ error: "unauthorized workspace" }, { status: 403 });
  }
  let abs: string;
  try { abs = mutationPath(workspace, body.path, false); } catch { return Response.json({ error: "path outside workspace" }, { status: 400 }); }
  if (existsSync(abs)) return Response.json({ error: "already exists" }, { status: 409 });
  try {
    if (kind === "file") closeSync(openSync(abs, "wx"));
    else mkdirSync(abs, { recursive: false });
  } catch (e) {
    return Response.json({ error: `create failed: ${(e as Error).message}` }, { status: 500 });
  }
  return Response.json({ ok: true, path: body.path });
}

/** Rename a workspace file or directory. Existing destinations are never overwritten. */
export async function PATCH(req: Request) {
  if (!(await getSession(req.headers)))
    return Response.json({ error: "unauthorized" }, { status: 401 });
  let body: { path?: unknown; newPath?: unknown; workspace?: unknown };
  try { body = await req.json(); } catch { return Response.json({ error: "invalid body" }, { status: 400 }); }
  if (typeof body.path !== "string" || !body.path.trim() || typeof body.newPath !== "string" || !body.newPath.trim())
    return Response.json({ error: "invalid body" }, { status: 400 });
  let workspace: string;
  try {
    workspace = authorizedWorkspace(typeof body.workspace === "string" && body.workspace ? body.workspace : resolveWorkspace(req));
  } catch {
    return Response.json({ error: "unauthorized workspace" }, { status: 403 });
  }
  let abs: string;
  let nextAbs: string;
  try {
    abs = mutationPath(workspace, body.path, true);
    nextAbs = mutationPath(workspace, body.newPath, false);
  } catch { return Response.json({ error: "path outside workspace" }, { status: 400 }); }
  if (!existsSync(abs)) return Response.json({ error: "not found" }, { status: 404 });
  if (existsSync(nextAbs)) return Response.json({ error: "already exists" }, { status: 409 });
  try { renameSync(abs, nextAbs); } catch (e) {
    return Response.json({ error: `rename failed: ${(e as Error).message}` }, { status: 500 });
  }
  return Response.json({ ok: true, path: body.newPath });
}

/** Permanently remove a workspace file or directory. Root deletion is rejected. */
export async function DELETE(req: Request) {
  if (!(await getSession(req.headers)))
    return Response.json({ error: "unauthorized" }, { status: 401 });
  const url = new URL(req.url);
  const rel = url.searchParams.get("path") ?? "";
  let workspace: string;
  try {
    workspace = authorizedWorkspace(url.searchParams.get("workspace") || resolveWorkspace(req));
  } catch {
    return Response.json({ error: "unauthorized workspace" }, { status: 403 });
  }
  if (!rel.trim()) return Response.json({ error: "invalid path" }, { status: 400 });
  let abs: string;
  try { abs = mutationPath(workspace, rel, true); } catch { return Response.json({ error: "path outside workspace" }, { status: 400 }); }
  if (!existsSync(abs)) return Response.json({ error: "not found" }, { status: 404 });
  try { rmSync(abs, { recursive: true, force: false }); } catch (e) {
    return Response.json({ error: `delete failed: ${(e as Error).message}` }, { status: 500 });
  }
  return Response.json({ ok: true, path: rel });
}
