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
import { basename, extname, join, normalize, relative, sep } from "node:path";
import { getBridge } from "@/server/core-bridge";
import { getSession } from "@/lib/auth";

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

/** Secret-ish filenames / extensions that must never be served for preview. */
const SECRET_EXT = /\.(env|pem|key|p12|pfx|crt|cer)$/i;
const SECRET_NAMES = new Set([
  ".env",
  ".env.local",
  ".env.development",
  ".env.production",
  ".env.test",
  "credentials.json",
  "credentials",
  "id_rsa",
  "id_ed25519",
  "id_ecdsa",
  "id_dsa",
  "id_rsa.pub",
  "known_hosts",
  "authorized_keys",
]);

function isSecretFile(name: string): boolean {
  if (SECRET_NAMES.has(name)) return true;
  if (name.startsWith(".env")) return true;
  return SECRET_EXT.test(name);
}

export async function GET(req: Request) {
  if (!(await getSession(req.headers)))
    return Response.json({ error: "unauthorized" }, { status: 401 });

  const bridge = getBridge();
  const url = new URL(req.url);
  const workspace = url.searchParams.get("workspace") ?? bridge.getDefaultWorkspace();
  const rel = (url.searchParams.get("path") ?? "").trim();

  if (!rel) return Response.json({ error: "missing path" }, { status: 400 });

  // Confine the path to the workspace (mirror api/files/route.ts:38-46).
  const abs = normalize(join(workspace, rel));
  const r = relative(workspace, abs);
  if (r.startsWith("..") || r.includes(`..${sep}`)) {
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

  return new Response(bytes, {
    status: 200,
    headers: {
      "Content-Type": contentType,
      "Content-Disposition": "inline",
      "Content-Length": String(bytes.byteLength),
      // Don't let browsers cache a stale preview across edits; don't let the
      // served bytes be re-sniffed into a different type.
      "Cache-Control": "no-store",
      "X-Content-Type-Options": "nosniff",
    },
  });
}
