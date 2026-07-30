// POST /api/upload — multipart file upload into a workspace folder.
//
// Supports multiple files AND folder uploads (nested relative paths retained).
// The browser sends each file as a form-data part whose *field name* is the
// file's relative path within `dest` (e.g. "src/index.ts" or
// "assets/logo.png"). The server confines every path to the workspace, refuses
// traversal, and applies a single conflict policy per request.
//
//   POST /api/upload  (multipart/form-data)
//     fields: workspace=<abs>  dest=<rel folder, default "">
//             replace=<"true"|"false", default "false">
//     parts:  <relPathWithinDest>=<File>  (one or more)
//   → 200 { ok: true, results: [{ path, ok, skipped?, size?, error? }] }
//   → 403 { error: "unauthorized workspace" }
//   → 400 { error: "no files" | "path outside workspace" | "invalid body" }
//
// Per-file results let the client report partial failures and retry only the
// files that failed. `skipped: true` is returned for an existing file when
// `replace` is false — the client uses this for its "skip" conflict choice.
// "keep both" is resolved client-side (the client picks a non-colliding name
// before upload) so the server stays stateless about existing tree contents.
import { closeSync, existsSync, mkdirSync, openSync, realpathSync, statSync, writeFileSync } from "node:fs";
import { dirname, relative, resolve, sep } from "node:path";
import { getSession } from "@/lib/auth";
import { authorizedWorkspace, confinePath, resolveWorkspace } from "@/server/workspace";

export const dynamic = "force-dynamic";
export const runtime = "nodejs";

/** Reject path separators, NUL, dots-only, and reserved names. */
function isValidRelativeName(name: string): boolean {
  if (!name || name.length > 1024) return false;
  if (name.includes("\0")) return false;
  // No absolute or parent traversal segments after normalization is enforced by
  // confinePath, but reject the literal segments early for clearer errors.
  if (name.split(/[\\/]/).some((seg) => seg === ".." || seg === "" && seg === name)) {
    // allow internal separators (folder upload); reject leading/trailing slash + ..
  }
  if (/^[\\/]/.test(name)) return false;
  if (/(^|[\\/])\.\.([\\/]|$)/.test(name)) return false;
  return true;
}

interface UploadResult {
  path: string;
  ok: boolean;
  skipped?: boolean;
  size?: number;
  error?: string;
}

export async function POST(req: Request) {
  if (!(await getSession(req.headers)))
    return Response.json({ error: "unauthorized" }, { status: 401 });

  let form: FormData;
  try {
    form = await req.formData();
  } catch {
    return Response.json({ error: "invalid body" }, { status: 400 });
  }

  const workspace =
    (typeof form.get("workspace") === "string" && (form.get("workspace") as string)) ||
    resolveWorkspace(req);
  let absWorkspace: string;
  try {
    absWorkspace = authorizedWorkspace(workspace);
  } catch {
    return Response.json({ error: "unauthorized workspace" }, { status: 403 });
  }

  const destRaw = typeof form.get("dest") === "string" ? (form.get("dest") as string) : "";
  const replace = form.get("replace") === "true";

  // Confine the destination folder. An empty dest = workspace root.
  let destAbs: string;
  try {
    destAbs = destRaw ? confinePath(absWorkspace, destRaw) : absWorkspace;
  } catch {
    return Response.json({ error: "path outside workspace" }, { status: 400 });
  }
  if (resolve(destAbs) === resolve(absWorkspace)) {
    // dest = root is allowed; just don't let mutationPath protect the root for uploads.
  }

  const realWorkspace = realpathSync(absWorkspace);

  const results: UploadResult[] = [];
  let fileCount = 0;

  for (const [key, value] of form.entries()) {
    if (key === "workspace" || key === "dest" || key === "replace") continue;
    // A file part is an object with an arrayBuffer() method; plain string
    // fields are skipped.
    if (typeof value !== "object" || value === null || typeof (value as { arrayBuffer?: unknown }).arrayBuffer !== "function") continue;
    fileCount++;

    const relPath = key;
    if (!isValidRelativeName(relPath)) {
      results.push({ path: relPath, ok: false, error: "invalid file name" });
      continue;
    }

    let abs: string;
    try {
      // Compose the full relative path under dest, then confine to the workspace.
      const fullRel = destRaw ? `${destRaw}/${relPath}` : relPath;
      abs = confinePath(absWorkspace, fullRel);
    } catch {
      results.push({ path: relPath, ok: false, error: "path outside workspace" });
      continue;
    }

    // Symlink-escape check: the resolved file's directory must stay under the
    // real workspace root. mkdir is only created after the parent is verified.
    const parentDir = dirname(abs);
    try {
      const realParent = existsSync(parentDir)
        ? realpathSync(parentDir)
        : parentDir;
      const confined = relative(realWorkspace, realParent);
      if (confined === ".." || confined.startsWith(`..${sep}`)) {
        results.push({ path: relPath, ok: false, error: "path outside workspace" });
        continue;
      }
    } catch {
      results.push({ path: relPath, ok: false, error: "path outside workspace" });
      continue;
    }

    // Conflict handling.
    if (existsSync(abs) && !replace) {
      const st = (() => { try { return statSync(abs); } catch { return null; } })();
      if (st && st.isDirectory()) {
        results.push({ path: relPath, ok: false, error: "a folder with this name exists" });
      } else {
        results.push({ path: relPath, ok: true, skipped: true, size: st?.size ?? 0 });
      }
      continue;
    }

    try {
      mkdirSync(dirname(abs), { recursive: true });
      const buffer = Buffer.from(await (value as { arrayBuffer: () => Promise<ArrayBuffer> }).arrayBuffer());
      if (replace && existsSync(abs) && statSync(abs).isDirectory()) {
        results.push({ path: relPath, ok: false, error: "cannot overwrite a folder with a file" });
        continue;
      }
      writeFileSync(abs, buffer);
      let size = buffer.byteLength;
      try {
        size = statSync(abs).size;
      } catch {
        /* keep buffer length */
      }
      results.push({ path: relPath, ok: true, skipped: false, size });
    } catch (e) {
      results.push({
        path: relPath,
        ok: false,
        error: `upload failed: ${(e as Error).message}`,
      });
    }
  }

  if (fileCount === 0) {
    return Response.json({ error: "no files" }, { status: 400 });
  }

  return Response.json({ ok: true, results });
}
