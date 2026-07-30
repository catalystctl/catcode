"use client";

// Explorer upload overlay: conflict resolution dialog + per-file progress +
// retry/cancel. Driven by the shared upload helpers in @/lib/upload.
//
// Lifecycle:
//   idle → (files dropped/selected) → resolving conflicts
//        → if conflicts: conflict dialog (replace/skip/keep-both × all, cancel)
//        → uploading (per-file XHR with progress)
//        → done (summary; failed items are retryable)
//
// The tree is refreshed only for affected folders on completion.

import { useCallback, useEffect, useRef, useState } from "react";
import { useFocusTrap } from "@/lib/use-focus-trap";
import {
  type ConflictResolution,
  type UploadFile,
  type UploadItemState,
  type UploadOutcome,
  filesToUploadFiles,
  dataTransferToUploadFiles,
  isSafeUploadName,
  keepBothName,
  uploadFile,
} from "@/lib/upload";
import { CheckIcon, XIcon, RefreshIcon, FolderIcon } from "@/components/icons";

export interface UploadSession {
  dest: string;
  files: UploadFile[];
}

type Phase = "idle" | "conflict" | "uploading" | "done";

interface ConflictEntry {
  file: UploadFile;
}

export interface UploadController {
  /** Begin an upload session for the given destination folder. */
  start: (dest: string, files: File[] | FileList | DataTransfer) => Promise<void>;
}

interface UploadOverlayProps {
  workspace: string;
  /** Existing known child paths under a destination folder (for conflict checks). */
  getExisting: (dest: string) => Set<string>;
  /** Refresh + reveal a folder after upload completes. */
  onUploaded: (dest: string, paths: string[]) => void;
}

export function UploadOverlay({ workspace, getExisting, onUploaded }: UploadOverlayProps) {
  const [phase, setPhase] = useState<Phase>("idle");
  const [dest, setDest] = useState("");
  const [items, setItems] = useState<UploadItemState[]>([]);
  const [conflicts, setConflicts] = useState<ConflictEntry[]>([]);
  const abortRef = useRef<(() => void) | null>(null);
  const cancelledRef = useRef(false);
  const phaseRef = useRef<Phase>("idle");
  const trapRef = useFocusTrap<HTMLDivElement>(phase === "conflict");

  const buildItems = useCallback((files: UploadFile[]): UploadItemState[] => {
    return files.map((f) => ({
      relPath: f.relPath,
      name: f.name,
      size: f.file.size,
      status: "pending" as const,
      progress: 0,
    }));
  }, []);

  const beginUpload = useCallback(
    async (resolved: UploadFile[], replace: boolean, uploadDest: string) => {
      // Reset the cancel flag for this batch; cancel()/close() set it true.
      cancelledRef.current = false;
      const states = buildItems(resolved);
      setItems(states);
      setPhase("uploading");
      setConflicts([]);

      const completedPaths: string[] = [];
      // Upload sequentially so per-file progress is meaningful and abort is
      // per-batch (the explorer doesn't need parallel uploads for correctness).
      for (let i = 0; i < resolved.length; i++) {
        if (cancelledRef.current) break; // cancelled via cancel()/close()
        const file = resolved[i];
        setItems((prev) =>
          prev.map((it, idx) =>
            idx === i ? { ...it, status: "uploading", progress: 0 } : it,
          ),
        );
        const handle = uploadFile(workspace, uploadDest, file, replace, (loaded, total) => {
          setItems((prev) =>
            prev.map((it, idx) =>
              idx === i ? { ...it, progress: total ? loaded / total : 0 } : it,
            ),
          );
        });
        abortRef.current = handle.abort;
        const outcome: UploadOutcome = await handle.promise;
        abortRef.current = null;
        if (cancelledRef.current) break; // aborted mid-file; stop the batch
        setItems((prev) =>
          prev.map((it, idx) => {
            if (idx !== i) return it;
            if (outcome.ok) {
              if (outcome.skipped) {
                return { ...it, status: "skipped", progress: 1 };
              }
              completedPaths.push(it.relPath);
              return { ...it, status: "done", progress: 1 };
            }
            return { ...it, status: "error", error: outcome.error };
          }),
        );
      }
      // If cancelled, cancel()/close() already reset phase to "idle" and cleared
      // items — don't clobber that with a "done" transition (async race).
      if (cancelledRef.current) return;
      setPhase("done");
      if (completedPaths.length) onUploaded(uploadDest, completedPaths);
    },
    [workspace, buildItems, onUploaded],
  );

  const resolveConflicts = useCallback(
    async (resolution: ConflictResolution) => {
      if (resolution === "cancel") {
        setPhase("idle");
        setConflicts([]);
        setItems([]);
        return;
      }
      const existing = new Set(getExisting(dest));
      // Rebuild the full file list from conflicts + non-conflicts (kept in items).
      const conflictRels = new Set(conflicts.map((c) => c.file.relPath));
      const allFiles: UploadFile[] = [];
      // Recover the original UploadFile list from the pending items + conflicts.
      for (const c of conflicts) allFiles.push(c.file);
      // We need the non-conflict files too; they were in the pending set.
      const pendingFiles = pendingFilesRef.current;
      for (const f of pendingFiles) {
        if (!conflictRels.has(f.relPath)) allFiles.push(f);
      }

      let replace = false;
      const skipSet = new Set<string>();
      const finalFiles: UploadFile[] = [];
      for (const f of allFiles) {
        const isConflict = conflictRels.has(f.relPath);
        if (isConflict) {
          if (resolution === "replace" || resolution === "replaceAll") {
            replace = true;
            finalFiles.push(f);
          } else if (resolution === "skip" || resolution === "skipAll") {
            skipSet.add(f.relPath);
          } else if (resolution === "keepBoth" || resolution === "keepBothAll") {
            const renamed = keepBothName(f.relPath, existing);
            finalFiles.push({ ...f, relPath: renamed, name: renamed.split("/").pop() ?? renamed });
          }
        } else {
          finalFiles.push(f);
        }
      }
      // Mark skipped items in the state.
      setItems((prev) => {
        const base = buildItems(finalFiles);
        return base;
      });
      setConflicts([]);
      // `replace` only applies to the files that were conflicts; non-conflict
      // files don't need replacement. We pass replace=true only if a replace
      // resolution was chosen (server double-checks existence anyway).
      await beginUpload(finalFiles, replace, dest);
    },
    [beginUpload, buildItems, conflicts, dest, getExisting],
  );

  // Keep a ref to the full pending file list so resolveConflicts can rebuild.
  const pendingFilesRef = useRef<UploadFile[]>([]);

  const start = useCallback(
    async (destination: string, input: File[] | FileList | DataTransfer) => {
      // The drop veil is pointer-events-none, so a drop can reach the tree even
      // while an upload is running or a conflict dialog is open. Ignore those —
      // a second concurrent upload would clobber the in-flight batch's items.
      if (phaseRef.current === "uploading" || phaseRef.current === "conflict") return;
      let files: UploadFile[];
      if (input instanceof DataTransfer) {
        files = await dataTransferToUploadFiles(input);
      } else {
        files = filesToUploadFiles(input);
      }
      files = files.filter((f) => isSafeUploadName(f.relPath));
      if (!files.length) return;
      pendingFilesRef.current = files;
      setDest(destination);

      const existing = new Set(getExisting(destination));
      const conflictFiles = files.filter((f) => existing.has(f.relPath));
      if (conflictFiles.length) {
        setConflicts(conflictFiles.map((file) => ({ file })));
        setItems(buildItems(files));
        setPhase("conflict");
      } else {
        // Pass the destination explicitly: `dest` state hasn't re-rendered into
        // beginUpload's closure yet, so reading it there would use the previous
        // destination (uploads landing in the wrong folder).
        await beginUpload(files, false, destination);
      }
    },
    [beginUpload, buildItems, getExisting],
  );

  const cancel = useCallback(() => {
    cancelledRef.current = true;
    abortRef.current?.();
    abortRef.current = null;
    setPhase("idle");
    setItems([]);
    setConflicts([]);
  }, []);

  const retryFailed = useCallback(async () => {
    const failed = items.filter((it) => it.status === "error");
    if (!failed.length) return;
    const files: UploadFile[] = [];
    for (const it of failed) {
      const original = pendingFilesRef.current.find((f) => f.relPath === it.relPath) ??
        pendingFilesRef.current.find((f) => f.name === it.name);
      if (original) files.push(original);
    }
    if (!files.length) return;
    // Reset failed items to pending.
    setItems((prev) =>
      prev.map((it) =>
        it.status === "error" ? { ...it, status: "pending", progress: 0, error: undefined } : it,
      ),
    );
    cancelledRef.current = false;
    setPhase("uploading");
    for (let i = 0; i < files.length; i++) {
      if (cancelledRef.current) break;
      const file = files[i];
      const itemIdx = items.findIndex((it) => it.relPath === file.relPath || it.name === file.name);
      setItems((prev) =>
        prev.map((it, idx) =>
          idx === itemIdx ? { ...it, status: "uploading", progress: 0 } : it,
        ),
      );
      const handle = uploadFile(workspace, dest, file, false, (loaded, total) => {
        setItems((prev) =>
          prev.map((it, idx) =>
            idx === itemIdx ? { ...it, progress: total ? loaded / total : 0 } : it,
          ),
        );
      });
      abortRef.current = handle.abort;
      const outcome = await handle.promise;
      abortRef.current = null;
      if (cancelledRef.current) break;
      setItems((prev) =>
        prev.map((it, idx) => {
          if (idx !== itemIdx) return it;
          if (outcome.ok) {
            if (!outcome.skipped) onUploaded(dest, [it.relPath]);
            return { ...it, status: outcome.skipped ? "skipped" : "done", progress: 1 };
          }
          return { ...it, status: "error", error: outcome.error };
        }),
      );
    }
    if (cancelledRef.current) return;
    setPhase("done");
  }, [items, workspace, dest, onUploaded]);

  const close = useCallback(() => {
    cancelledRef.current = true;
    abortRef.current?.();
    setPhase("idle");
    setItems([]);
    setConflicts([]);
  }, []);

  // Expose the controller via ref-free callback is awkward; instead this
  // component is rendered by the tree which passes `start` down. To keep it
  // simple, the parent reads `start` from a ref we expose through render-prop.
  // Simpler: the tree owns a ref to this component. We attach to window for now
  // is NOT acceptable. Instead, return the controller as part of render below
  // via a hidden imperative handle the parent registers.
  useEffect(() => {
    registerStart(start);
    // Clear the module-global on unmount so a dropped file can't invoke a
    // closure belonging to an unmounted tree (stale workspace/refs).
    return () => registerStart(null);
  }, [start]);

  // Mirror phase into a ref so `start` can read the live phase without adding
  // `phase` to its deps (which would re-create and re-register it each change).
  useEffect(() => {
    phaseRef.current = phase;
  }, [phase]);

  // Auto-dismiss the overlay shortly after a fully-successful upload. Without
  // this the near-invisible full-screen veil lingers at phase "done"; even
  // though it is now pointer-events-none (so it no longer blocks drops), leaving
  // the "Upload complete" card up is confusing. Keep it open on errors so they
  // can be retried.
  const hasErrors = items.some((i) => i.status === "error");
  useEffect(() => {
    if (phase !== "done" || hasErrors) return;
    const timer = window.setTimeout(() => {
      setPhase("idle");
      setItems([]);
      setConflicts([]);
    }, 1500);
    return () => window.clearTimeout(timer);
  }, [phase, hasErrors]);

  if (phase === "idle") return null;

  const doneCount = items.filter((i) => i.status === "done").length;
  const errorCount = items.filter((i) => i.status === "error").length;
  const skippedCount = items.filter((i) => i.status === "skipped").length;
  const uploading = items.some((i) => i.status === "uploading");

  return (
    <div className="pointer-events-none fixed inset-0 z-[75] flex items-end justify-end bg-black/40 p-3 backdrop-blur-[1px] sm:items-center sm:justify-center sm:p-4">
      <section
        ref={trapRef}
        role="dialog"
        aria-modal="true"
        aria-label="Upload files"
        className="pointer-events-auto flex max-h-[80dvh] w-full max-w-lg flex-col overflow-hidden rounded-xl border border-ink-700 bg-ink-925 shadow-2xl shadow-black/50 animate-fade-in"
        onMouseDown={(e) => e.stopPropagation()}
      >
        <header className="flex items-center gap-2 border-b border-ink-800 px-3 py-2.5">
          <FolderIcon width={15} height={15} className="shrink-0 text-accent-soft" />
          <div className="min-w-0 flex-1">
            <h2 className="truncate text-[13px] font-semibold text-ink-100">
              {phase === "conflict" ? "File conflict" : phase === "done" ? "Upload complete" : "Uploading…"}
            </h2>
            <p className="truncate font-mono text-[10px] text-ink-500">
              {dest ? `→ ${dest}/` : "→ workspace root"}
            </p>
          </div>
          <button
            type="button"
            onClick={close}
            aria-label="Close upload"
            className="rounded-md p-1 text-ink-500 hover:bg-ink-800 hover:text-ink-100"
          >
            <XIcon width={14} height={14} />
          </button>
        </header>

        {phase === "conflict" ? (
          <div className="flex flex-col gap-2 p-3">
            <p className="text-[12px] text-ink-300">
              {conflicts.length === 1
                ? `“${conflicts[0].file.name}” already exists in this folder.`
                : `${conflicts.length} files already exist in this folder.`}
            </p>
            <ul className="max-h-40 overflow-y-auto rounded-lg border border-ink-800 bg-ink-950 p-1">
              {conflicts.slice(0, 8).map((c) => (
                <li key={c.file.relPath} className="truncate px-1.5 py-1 text-[11px] text-ink-300">
                  {c.file.relPath}
                </li>
              ))}
              {conflicts.length > 8 ? (
                <li className="px-1.5 py-1 text-[11px] text-ink-500">
                  …and {conflicts.length - 8} more
                </li>
              ) : null}
            </ul>
            <div className="grid grid-cols-2 gap-1.5 sm:grid-cols-3">
              <button type="button" onClick={() => void resolveConflicts("replace")} className="rounded-lg border border-ink-700 px-2 py-1.5 text-[11px] text-ink-200 hover:bg-ink-800">Replace</button>
              <button type="button" onClick={() => void resolveConflicts("replaceAll")} className="rounded-lg border border-ink-700 px-2 py-1.5 text-[11px] text-ink-200 hover:bg-ink-800">Replace all</button>
              <button type="button" onClick={() => void resolveConflicts("skip")} className="rounded-lg border border-ink-700 px-2 py-1.5 text-[11px] text-ink-200 hover:bg-ink-800">Skip</button>
              <button type="button" onClick={() => void resolveConflicts("skipAll")} className="rounded-lg border border-ink-700 px-2 py-1.5 text-[11px] text-ink-200 hover:bg-ink-800">Skip all</button>
              <button type="button" onClick={() => void resolveConflicts("keepBoth")} className="rounded-lg border border-ink-700 px-2 py-1.5 text-[11px] text-ink-200 hover:bg-ink-800">Keep both</button>
              <button type="button" onClick={() => void resolveConflicts("keepBothAll")} className="rounded-lg border border-ink-700 px-2 py-1.5 text-[11px] text-ink-200 hover:bg-ink-800">Keep both all</button>
            </div>
            <button type="button" onClick={() => resolveConflicts("cancel")} className="mt-1 rounded-lg px-2 py-1.5 text-[11px] text-ink-500 hover:text-ink-300">
              Cancel upload
            </button>
          </div>
        ) : (
          <>
            <div className="min-h-0 flex-1 overflow-y-auto p-2">
              {items.map((it) => (
                <div key={it.relPath} className="flex items-center gap-2 px-1.5 py-1.5">
                  <span className="w-4 shrink-0 text-center">
                    {it.status === "done" ? (
                      <CheckIcon width={13} height={13} className="text-emerald-400" />
                    ) : it.status === "error" ? (
                      <XIcon width={13} height={13} className="text-danger" />
                    ) : it.status === "skipped" ? (
                      <span className="text-[10px] text-ink-500">–</span>
                    ) : it.status === "uploading" ? (
                      <span className="text-[10px] text-accent-soft">↻</span>
                    ) : null}
                  </span>
                  <span className="min-w-0 flex-1">
                    <span className="block truncate text-[12px] text-ink-200">{it.relPath}</span>
                    {it.status === "uploading" ? (
                      <span className="mt-0.5 block h-1 overflow-hidden rounded bg-ink-800">
                        <span className="block h-full bg-accent transition-[width]" style={{ width: `${Math.round(it.progress * 100)}%` }} />
                      </span>
                    ) : null}
                  </span>
                  <span className="shrink-0 text-[10px] text-ink-500">
                    {it.status === "error"
                      ? it.error ?? "failed"
                      : it.status === "done"
                        ? "done"
                        : it.status === "skipped"
                          ? "skipped"
                          : `${Math.round(it.progress * 100)}%`}
                  </span>
                </div>
              ))}
            </div>
            <footer className="flex items-center justify-between gap-2 border-t border-ink-800 px-3 py-2">
              <span className="text-[11px] text-ink-500">
                {phase === "done"
                  ? `${doneCount} done${skippedCount ? `, ${skippedCount} skipped` : ""}${errorCount ? `, ${errorCount} failed` : ""}`
                  : `${items.length} files`}
              </span>
              <div className="flex items-center gap-1.5">
                {uploading ? (
                  <button type="button" onClick={cancel} className="rounded-lg border border-ink-700 px-2.5 py-1 text-[11px] text-ink-300 hover:bg-ink-800">
                    Cancel
                  </button>
                ) : (
                  <>
                    {errorCount > 0 ? (
                      <button type="button" onClick={() => void retryFailed()} className="flex items-center gap-1.5 rounded-lg border border-ink-700 px-2.5 py-1 text-[11px] text-ink-300 hover:bg-ink-800">
                        <RefreshIcon width={12} height={12} /> Retry failed ({errorCount})
                      </button>
                    ) : null}
                    <button type="button" onClick={close} className="rounded-lg bg-accent px-2.5 py-1 text-[11px] font-medium text-white hover:bg-accent-soft">
                      Done
                    </button>
                  </>
                )}
              </div>
            </footer>
          </>
        )}
      </section>
    </div>
  );
}

// Lightweight imperative bridge: the file tree registers a `start` function so
// its header buttons / context menu / drop handlers can launch uploads without
// prop-drilling the controller through every TreeNode.
let startFn: ((dest: string, files: File[] | FileList | DataTransfer) => Promise<void>) | null = null;
function registerStart(fn: typeof startFn): void {
  startFn = fn;
}
export function triggerUpload(dest: string, files: File[] | FileList | DataTransfer): Promise<void> {
  if (!startFn) return Promise.resolve();
  return startFn(dest, files);
}
