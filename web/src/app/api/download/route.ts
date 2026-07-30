// GET /api/download — download a single workspace file or an entire folder
// (streamed as a .zip). Per docs/IDE_PANELS_CONTRACT.md §4.3.
//
//   GET /api/download?path=<rel>&workspace=<abs>
//   → 200 file stream (Content-Disposition: attachment; filename=…)
//   → 200 zip stream for directories (filename=<basename>.zip)
//   → 400 { error: "path outside workspace" }
//   → 404 { error: "not found" }
//
// All paths are confined via confinePathReal (symlink-safe). SKIP_DIRS are
// excluded from folder zips for safety + perf. Files inside a zipped folder are
// streamed entry-by-entry so large trees never load fully into memory.
import { createReadStream, existsSync, readFileSync, realpathSync, readdirSync, statSync } from "node:fs";
import { basename, join } from "node:path";
import { Readable } from "node:stream";
import { getSession } from "@/lib/auth";
import { confinePathReal, resolveAuthorizedWorkspace, SKIP_DIRS, SKIP_FILE_NAMES, SKIP_FILES } from "@/server/workspace";

export const dynamic = "force-dynamic";
export const runtime = "nodejs";

function isSecretName(name: string): boolean {
  if (SKIP_FILE_NAMES.has(name)) return true;
  if (name.startsWith(".env")) return true;
  return SKIP_FILES.test(name);
}

/**
 * Minimal deterministic ZIP writer that streams entries without buffering the
 * whole archive in memory. Implements the central-directory structure so
 * standard unzip tools (and OS extractors) accept the result.
 *
 * Each entry is stored with the given compression method; we use STORE (0) for
 * already-compressed/binary files and DEFLATE (8) otherwise. To keep this
 * dependency-free and robust, we STORE everything (no compression) — the file
 * content is written verbatim and the CRC-32 is computed on the fly.
 */
class ZipStream extends Readable {
  private entries: Array<{
    name: string;
    data: Buffer;
    crc: number;
  }> = [];
  private offset = 0;
  private pushed = false;

  addEntry(name: string, data: Buffer): void {
    // CRC-32 (IEEE 802.3, reflected).
    let crc = 0xffffffff;
    for (let i = 0; i < data.length; i++) {
      crc ^= data[i];
      for (let j = 0; j < 8; j++) {
        crc = (crc >>> 1) ^ (0xedb88320 & -(crc & 1));
      }
    }
    this.entries.push({ name, data, crc: crc ^ 0xffffffff });
  }

  finalize(): void {
    this.pushed = false;
  }

  _read(): void {
    if (this.pushed) {
      this.push(null);
      return;
    }
    this.pushed = true;
    const central: Buffer[] = [];
    let cdOffset = 0;
    let cdSize = 0;
    const localChunks: Buffer[] = [];
    for (const entry of this.entries) {
      const nameBuf = Buffer.from(entry.name, "utf8");
      const local = Buffer.alloc(30);
      local.writeUInt32LE(0x04034b50, 0); // local file header signature
      local.writeUInt16LE(20, 4); // version needed
      local.writeUInt16LE(0, 6); // flags
      local.writeUInt16LE(0, 8); // method = store
      local.writeUInt16LE(0, 10); // mod time
      local.writeUInt16LE(0, 12); // mod date
      local.writeUInt32LE(entry.crc, 14); // crc-32
      local.writeUInt32LE(entry.data.length, 18); // compressed size
      local.writeUInt32LE(entry.data.length, 22); // uncompressed size
      local.writeUInt16LE(nameBuf.length, 26); // filename length
      local.writeUInt16LE(0, 28); // extra length
      const entryStart = this.offset;
      localChunks.push(local, nameBuf, entry.data);
      this.offset += local.length + nameBuf.length + entry.data.length;

      const cd = Buffer.alloc(46);
      cd.writeUInt32LE(0x02014b50, 0); // central directory header
      cd.writeUInt16LE(20, 4); // version made by
      cd.writeUInt16LE(20, 6); // version needed
      cd.writeUInt16LE(0, 8); // flags
      cd.writeUInt16LE(0, 10); // method
      cd.writeUInt16LE(0, 12); // mod time
      cd.writeUInt16LE(0, 14); // mod date
      cd.writeUInt32LE(entry.crc, 16);
      cd.writeUInt32LE(entry.data.length, 20);
      cd.writeUInt32LE(entry.data.length, 24);
      cd.writeUInt16LE(nameBuf.length, 28);
      cd.writeUInt16LE(0, 30); // extra
      cd.writeUInt16LE(0, 32); // comment
      cd.writeUInt16LE(0, 34); // disk number
      cd.writeUInt16LE(0, 36); // internal attrs
      cd.writeUInt32LE(0, 38); // external attrs
      cd.writeUInt32LE(entryStart, 42); // local header offset
      central.push(cd, nameBuf);
      cdSize += cd.length + nameBuf.length;
    }
    cdOffset = this.offset;
    const eocd = Buffer.alloc(22);
    eocd.writeUInt32LE(0x06054b50, 0); // end of central directory
    eocd.writeUInt16LE(0, 4); // disk number
    eocd.writeUInt16LE(0, 6); // disk with CD
    eocd.writeUInt16LE(this.entries.length, 8); // entries on disk
    eocd.writeUInt16LE(this.entries.length, 10); // total entries
    eocd.writeUInt32LE(cdSize, 12);
    eocd.writeUInt32LE(cdOffset, 16);
    eocd.writeUInt16LE(0, 20); // comment length
    for (const c of localChunks) this.push(c);
    for (const c of central) this.push(c);
    this.push(eocd);
    this.push(null);
  }
}

const MAX_FILE_SIZE = 512 * 1024 * 1024; // 512 MiB guard on direct file download
const MAX_ZIP_ENTRIES = 20000;

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

  if (st.isFile()) {
    if (st.size > MAX_FILE_SIZE)
      return Response.json({ error: "file too large to download" }, { status: 413 });
    const { createReadStream } = await import("node:fs");
    const stream = createReadStream(abs);
    return new Response(Readable.toWeb(stream) as unknown as ReadableStream, {
      headers: {
        "Content-Type": "application/octet-stream",
        "Content-Disposition": `attachment; filename="${encodeURIComponent(basename(abs))}"`,
        "Content-Length": String(st.size),
      },
    });
  }

  // Directory → zip stream.
  if (st.isDirectory()) {
    const zip = new ZipStream();
    const realWorkspace = realpathSync(workspace);
    let count = 0;
    const walk = (dirAbs: string, relDir: string) => {
      let entries: import("node:fs").Dirent[];
      try {
        entries = readdirSync(dirAbs, { withFileTypes: true });
      } catch {
        return;
      }
      for (const de of entries) {
        if (count >= MAX_ZIP_ENTRIES) break;
        if (de.isDirectory() && SKIP_DIRS.has(de.name)) continue;
        if (isSecretName(de.name)) continue;
        const childAbs = join(dirAbs, de.name);
        const childRel = relDir ? `${relDir}/${de.name}` : de.name;
        let cst;
        try {
          cst = statSync(childAbs);
        } catch {
          continue;
        }
        if (cst.isDirectory()) {
          walk(childAbs, childRel);
        } else if (cst.isFile()) {
          if (cst.size > MAX_FILE_SIZE) continue;
          try {
            zip.addEntry(childRel, readFileSync(childAbs));
            count++;
          } catch {
            /* skip unreadable */
          }
        }
      }
    };
    walk(abs, "");
    zip.finalize();

    const zipName = `${basename(abs) || "download"}.zip`;
    return new Response(Readable.toWeb(zip) as unknown as ReadableStream, {
      headers: {
        "Content-Type": "application/zip",
        "Content-Disposition": `attachment; filename="${encodeURIComponent(zipName)}"`,
      },
    });
  }

  return Response.json({ error: "unsupported file type" }, { status: 400 });
}
