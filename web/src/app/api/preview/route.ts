// GET /api/preview?path=<rel>&workspace=<abs>
//
// Serves a workspace file for in-browser preview with a safe Content-Type.
// Mirrors api/files/route.ts for auth + workspace path confinement, and adds:
//   • secret-file blocking (403) — credentials are never served for preview;
//   • a strict allowlist of previewable extensions with correct Content-Type;
//   • Content-Disposition: inline so browsers render (not download) the bytes.
//
// Markdown (.md/.markdown) is served as raw `text/markdown` and rendered
// client-side by the <Preview> component (react-markdown). HTML/HTM are served
// as `text/html` for direct <iframe> rendering. This is a USER-driven panel —
// it never touches the core agent loop.

import { readFileSync, statSync } from "node:fs";
import { basename, extname } from "node:path";
import { getSession } from "@/lib/auth";
import { injectPreviewHelpers } from "@/server/preview-inject";
import {
  confinePathReal,
  isSecretFile,
  resolveAuthorizedWorkspace,
} from "@/server/workspace";

export const dynamic = "force-dynamic";
export const runtime = "nodejs";

/** Cap served bytes so a huge binary is never loaded into memory. */
const MAX_BYTES = 5 * 1024 * 1024; // 5 MiB

/**
 * Previewable extensions → Content-Type. Anything else is refused (415) so the
 * route never guesses a type for an arbitrary file (prevents serving, e.g., a
 * `.svg` masquerading as HTML after a rename — type follows the extension).
 */
const CONTENT_TYPES: Record<string, string> = {
  ".html": "text/html; charset=utf-8",
  ".htm": "text/html; charset=utf-8",
  ".md": "text/markdown; charset=utf-8",
  ".markdown": "text/markdown; charset=utf-8",
  ".svg": "image/svg+xml",
  ".png": "image/png",
  ".jpg": "image/jpeg",
  ".jpeg": "image/jpeg",
  ".gif": "image/gif",
  ".webp": "image/webp",
  ".pdf": "application/pdf",
  ".txt": "text/plain; charset=utf-8",
  ".json": "application/json; charset=utf-8",
  ".css": "text/css; charset=utf-8",
  ".js": "text/javascript; charset=utf-8",
};

/** CSP for SVG (no scripts). HTML preview relies on the iframe sandbox +
 *  injectPreviewHelpers inspect bootstrap, so it must allow same-document scripts. */
const SVG_CSP = "default-src 'none'; script-src 'none'; sandbox";
const HTML_CSP =
  "default-src 'none'; img-src data: blob: *; style-src 'unsafe-inline'; script-src 'unsafe-inline'; connect-src 'none'; frame-src 'none'; object-src 'none'; base-uri 'none'";

export async function GET(req: Request) {
  if (!(await getSession(req.headers)))
    return Response.json({ error: "unauthorized" }, { status: 401 });

  let workspace: string;
  try {
    workspace = resolveAuthorizedWorkspace(req);
  } catch {
    return Response.json({ error: "unauthorized workspace" }, { status: 403 });
  }

  const url = new URL(req.url);
  const rel = (url.searchParams.get("path") ?? "").trim();

  if (!rel) return Response.json({ error: "missing path" }, { status: 400 });

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
  if (st.isDirectory()) {
    return Response.json({ error: "path is a directory" }, { status: 400 });
  }

  const name = basename(abs);
  if (isSecretFile(name)) {
    return Response.json({ error: "refused: secret file" }, { status: 403 });
  }

  const ext = extname(abs).toLowerCase();
  const contentType = CONTENT_TYPES[ext];
  if (!contentType) {
    return Response.json(
      { error: `unsupported preview type: ${ext || "(none)"}` },
      { status: 415 },
    );
  }

  if (st.size > MAX_BYTES) {
    return Response.json({ error: "file too large" }, { status: 400 });
  }

  const isHtml = ext === ".html" || ext === ".htm";
  const isSvg = ext === ".svg";

  // HTML gets the inspect bootstrap so Preview can pick elements into chat.
  // Other types are served as raw bytes.
  if (isHtml) {
    let text: string;
    try {
      text = readFileSync(abs, "utf8");
    } catch {
      return Response.json({ error: "not found" }, { status: 404 });
    }
    const rewritten = injectPreviewHelpers(text, { inspect: true });
    return new Response(rewritten, {
      status: 200,
      headers: {
        "Content-Type": contentType,
        "Content-Disposition": "inline",
        "Content-Security-Policy": HTML_CSP,
        "Cache-Control": "no-store",
        "X-Content-Type-Options": "nosniff",
      },
    });
  }

  // readFileSync returns Buffer<ArrayBufferLike>, but BodyInit's BufferSource
  // requires ArrayBufferView<ArrayBuffer> (TS 5.9 lib.dom). Copy into a fresh
  // ArrayBuffer-backed Uint8Array so the body type-checks cleanly. The 5 MiB cap
  // above keeps this copy cheap.
  let bytes: Uint8Array<ArrayBuffer>;
  try {
    const raw = readFileSync(abs);
    bytes = new Uint8Array(raw.byteLength);
    bytes.set(raw);
  } catch {
    return Response.json({ error: "not found" }, { status: 404 });
  }

  const headers: Record<string, string> = {
    "Content-Type": contentType,
    "Content-Disposition": isSvg ? "attachment" : "inline",
    "Content-Length": String(bytes.byteLength),
    // Don't let browsers cache a stale preview across edits; don't let the
    // served bytes be re-sniffed into a different type.
    "Cache-Control": "no-store",
    "X-Content-Type-Options": "nosniff",
  };
  if (isSvg) {
    headers["Content-Security-Policy"] = SVG_CSP;
  }

  return new Response(bytes, { status: 200, headers });
}
