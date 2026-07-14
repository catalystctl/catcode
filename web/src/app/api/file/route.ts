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
import { readFileSync, statSync, writeFileSync, mkdirSync } from "node:fs";
import { dirname } from "node:path";
import { getSession } from "@/lib/auth";
import { resolveWorkspace, confinePath } from "@/server/workspace";
import { detectLanguage } from "@/lib/lang";

export const dynamic = "force-dynamic";
export const runtime = "nodejs";

const MAX_BYTES = 5 * 1024 * 1024; // 5 MiB

export async function GET(req: Request) {
  if (!(await getSession(req.headers)))
    return Response.json({ error: "unauthorized" }, { status: 401 });

  const url = new URL(req.url);
  const workspace = resolveWorkspace(req);
  const rel = url.searchParams.get("path") ?? "";

  let abs: string;
  try {
    abs = confinePath(workspace, rel);
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
  const workspace =
    (typeof body.workspace === "string" && body.workspace) ||
    new URL(req.url).searchParams.get("workspace") ||
    resolveWorkspace(req);

  let abs: string;
  try {
    abs = confinePath(workspace, rel);
  } catch {
    return Response.json({ error: "path outside workspace" }, { status: 400 });
  }

  try {
    mkdirSync(dirname(abs), { recursive: true });
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
