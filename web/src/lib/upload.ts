"use client";

// File-upload helper for the explorer. Handles:
//  - flattening a FileList / dropped DataTransfer into per-file relative paths
//    (folder uploads retain nested structure via webkitRelativePath)
//  - client-side conflict detection against the cached tree
//  - "keep both" name generation (non-colliding)
//  - XHR-based multipart upload with real upload progress (fetch can't report
//    upload progress) + cancellation
//  - per-file status + partial-failure reporting + retry
//
// The server (POST /api/upload) is the final authority: if `replace` is false
// and a file exists, it returns `skipped: true`, so a stale cache cannot cause a
// silent overwrite.

export type ConflictChoice = "replace" | "skip" | "keep-both" | "cancel";
/** `*All` variants apply to every remaining conflict in the batch. */
export type ConflictResolution =
  | "replace"
  | "replaceAll"
  | "skip"
  | "skipAll"
  | "keepBoth"
  | "keepBothAll"
  | "cancel";

export interface UploadFile {
  /** Browser File object. */
  file: File;
  /** Relative path within the destination folder (may include subfolders). */
  relPath: string;
  /** Display name (basename). */
  name: string;
}

export type UploadStatus = "pending" | "uploading" | "done" | "skipped" | "error";

export interface UploadItemState {
  relPath: string;
  name: string;
  size: number;
  status: UploadStatus;
  /** 0..1 during "uploading". */
  progress: number;
  error?: string;
}

/**
 * Convert a dropped/selected file list into UploadFile[] with relative paths.
 * Folder inputs (webkitdirectory / dropped folders) expose webkitRelativePath.
 */
export function filesToUploadFiles(files: File[] | FileList): UploadFile[] {
  const list = Array.from(files);
  return list
    .map((file): UploadFile | null => {
      const rel = (file as File & { webkitRelativePath?: string }).webkitRelativePath;
      if (rel && rel.includes("/")) {
        // Folder upload: strip the top-level folder name so files land inside dest.
        const parts = rel.split("/").filter(Boolean);
        parts.shift(); // drop the root folder name
        const within = parts.join("/");
        if (!within) {
          // The dropped item was the folder itself with a single file at root.
          return { file, relPath: file.name, name: file.name };
        }
        return { file, relPath: within, name: parts[parts.length - 1] };
      }
      return { file, relPath: file.name, name: file.name };
    })
    .filter((f): f is UploadFile => f !== null);
}

/** Extract files from a drag DataTransfer (handles directory drops on Chrome). */
export async function dataTransferToUploadFiles(
  dt: DataTransfer,
): Promise<UploadFile[]> {
  // Prefer the items API so we can traverse directories.
  const items = Array.from(dt.items ?? []);
  if (items.length && typeof items[0].webkitGetAsEntry === "function") {
    const entries = items
      .map((item) => item.webkitGetAsEntry())
      .filter((e): e is FileSystemEntry => e !== null);
    if (entries.length) {
      const out: UploadFile[] = [];
      for (const entry of entries) await walkEntry(entry, "", out);
      return out;
    }
  }
  return filesToUploadFiles(dt.files);
}

function walkEntry(
  entry: FileSystemEntry,
  prefix: string,
  out: UploadFile[],
): Promise<void> {
  return new Promise((resolve) => {
    if (entry.isFile) {
      (entry as FileSystemFileEntry).file((file) => {
        const relPath = prefix ? `${prefix}/${file.name}` : file.name;
        out.push({ file, relPath, name: file.name });
        resolve();
      }, () => resolve());
    } else if (entry.isDirectory) {
      const reader = (entry as FileSystemDirectoryEntry).createReader();
      const readBatch = () => {
        reader.readEntries(async (children) => {
          if (!children.length) {
            resolve();
            return;
          }
          const name = entry.name;
          const nextPrefix = prefix ? `${prefix}/${name}` : name;
          for (const child of children) {
            await walkEntry(child, nextPrefix, out);
          }
          readBatch();
        }, () => resolve());
      };
      readBatch();
    } else {
      resolve();
    }
  });
}

/**
 * Detect which upload files collide with existing relative paths in `dest`.
 * `existingNames` is the set of immediate+known child paths under dest.
 */
export function findConflicts(
  files: UploadFile[],
  existing: Set<string>,
): UploadFile[] {
  return files.filter((f) => existing.has(f.relPath));
}

/**
 * Generate a non-colliding "keep both" name within the existing set, mutating
 * `existing` so successive keeps don't re-collide.
 *
 *   report.pdf → report (1).pdf
 *   README     → README (1)
 */
export function keepBothName(relPath: string, existing: Set<string>): string {
  const slash = relPath.lastIndexOf("/");
  const dir = slash >= 0 ? relPath.slice(0, slash) : "";
  const base = slash >= 0 ? relPath.slice(slash + 1) : relPath;
  const dot = base.lastIndexOf(".");
  const stem = dot > 0 ? base.slice(0, dot) : base;
  const ext = dot > 0 ? base.slice(dot) : "";
  let n = 1;
  let candidate = relPath;
  while (existing.has(candidate)) {
    candidate = dir ? `${dir}/${stem} (${n})${ext}` : `${stem} (${n})${ext}`;
    n++;
  }
  existing.add(candidate);
  return candidate;
}

export interface UploadOutcome {
  ok: boolean;
  skipped?: boolean;
  size?: number;
  error?: string;
}

export interface UploadHandle {
  /** Resolves with per-file outcomes when the upload completes or is aborted. */
  promise: Promise<UploadOutcome>;
  /** Abort the in-flight upload. */
  abort: () => void;
}

/**
 * Upload a single file via multipart form-data with progress. Returns a handle
 * so the caller can cancel. Uses XHR (the only browser API that reports upload
 * progress). `replace=false` → server returns `skipped:true` for collisions.
 */
export function uploadFile(
  workspace: string,
  dest: string,
  file: UploadFile,
  replace: boolean,
  onProgress?: (loaded: number, total: number) => void,
): UploadHandle {
  const xhr = new XMLHttpRequest();
  const form = new FormData();
  form.append("workspace", workspace);
  form.append("dest", dest);
  form.append("replace", replace ? "true" : "false");
  form.append(file.relPath, file.file, file.name);

  const promise = new Promise<UploadOutcome>((resolve) => {
    xhr.upload.onprogress = (event) => {
      if (event.lengthComputable && onProgress) {
        onProgress(event.loaded, event.total);
      }
    };
    xhr.onload = () => {
      if (xhr.status >= 200 && xhr.status < 300) {
        try {
          const data = JSON.parse(xhr.responseText) as {
            results?: Array<{ path: string; ok: boolean; skipped?: boolean; size?: number; error?: string }>;
            error?: string;
          };
          const result = data.results?.[0];
          if (result) {
            resolve({
              ok: result.ok,
              skipped: result.skipped === true,
              size: result.size,
              error: result.error,
            });
            return;
          }
          resolve({ ok: false, error: data.error ?? "upload failed" });
        } catch {
          resolve({ ok: false, error: "invalid response" });
        }
      } else {
        let msg = `upload failed (${xhr.status})`;
        try {
          const data = JSON.parse(xhr.responseText) as { error?: string };
          if (data.error) msg = data.error;
        } catch {
          /* keep default */
        }
        resolve({ ok: false, error: msg });
      }
    };
    xhr.onerror = () => resolve({ ok: false, error: "network error" });
    xhr.onabort = () => resolve({ ok: false, error: "cancelled" });
    xhr.open("POST", "/api/upload");
    xhr.send(form);
  });

  return {
    promise,
    abort: () => {
      try {
        xhr.abort();
      } catch {
        /* ignore */
      }
    },
  };
}

/** Whether a filename is safe to accept client-side (fast reject before round-trip). */
export function isSafeUploadName(relPath: string): boolean {
  if (!relPath || relPath.length > 1024) return false;
  if (relPath.includes("\0")) return false;
  if (/^[\\/]/.test(relPath)) return false;
  if (/(^|[\\/])\.\.([\\/]|$)/.test(relPath)) return false;
  return true;
}
