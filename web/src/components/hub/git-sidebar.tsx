"use client";

// The hub's git sidebar wraps GitPanel (changes, history, branches, stashes,
// remotes — every /api/git action) in a minimal IdeContext shim. openDiff /
// openPatch / openFile open an in-hub viewer modal instead of an editor.

import { useCallback, useEffect, useRef, useState } from "react";
import { Diff } from "@/components/diff";
import { GitPanel } from "@/components/ide/git-panel";
import { XIcon } from "@/components/icons";
import { IdeContext, type IdeApi, type IdeContextValue } from "@/lib/ide-context";
import type { GitStatus } from "@/lib/types";
import { useBodyScrollLock } from "@/lib/use-body-scroll-lock";
import { useFocusTrap } from "@/lib/use-focus-trap";
import { mergeRefs, useOutsideClose } from "@/lib/use-outside-close";

type ViewerState =
  | { kind: "none" }
  | { kind: "diff"; title: string; loading: boolean; content?: string; error?: string }
  | { kind: "patch"; title: string; loading: boolean; content?: string; error?: string }
  | { kind: "file"; title: string; loading: boolean; content?: string; error?: string };

export function HubGitSidebar({ workspace }: { workspace: string }) {
  const [gitStatus, setGitStatus] = useState<GitStatus | null>(null);
  const [viewer, setViewer] = useState<ViewerState>({ kind: "none" });
  const viewerRequestRef = useRef<{ generation: number; controller: AbortController | null }>({
    generation: 0,
    controller: null,
  });

  const cancelViewerRequest = useCallback(() => {
    viewerRequestRef.current.controller?.abort();
    viewerRequestRef.current = {
      generation: viewerRequestRef.current.generation + 1,
      controller: null,
    };
  }, []);

  // A project switch must not leak the previous project's git state.
  useEffect(() => {
    cancelViewerRequest();
    setGitStatus(null);
    setViewer({ kind: "none" });
    return cancelViewerRequest;
  }, [cancelViewerRequest, workspace]);

  const fetchText = useCallback(
    async (url: string, field: "diff" | "patch" | "content", signal: AbortSignal): Promise<string> => {
      const res = await fetch(url, { cache: "no-store", signal });
      const data = (await res.json().catch(() => ({}))) as Record<string, unknown> & {
        error?: string;
      };
      if (!res.ok) throw new Error(data.error ?? `request failed (${res.status})`);
      return typeof data[field] === "string" ? (data[field] as string) : "";
    },
    [],
  );

  const openDiff = useCallback(
    (path: string, opts?: { staged?: boolean }) => {
      cancelViewerRequest();
      const controller = new AbortController();
      const generation = viewerRequestRef.current.generation;
      viewerRequestRef.current.controller = controller;
      const staged = !!opts?.staged;
      setViewer({
        kind: "diff",
        title: `${path} ${staged ? "(staged)" : ""}`,
        loading: true,
      });
      fetchText(
        `/api/git?workspace=${encodeURIComponent(workspace)}&diff=${encodeURIComponent(path)}&staged=${staged ? "1" : "0"}`,
        "diff",
        controller.signal,
      )
        .then((content) => {
          if (viewerRequestRef.current.generation !== generation) return;
          setViewer((v) =>
            v.kind === "diff" ? { ...v, loading: false, content: content || "(no changes)" } : v,
          );
        })
        .catch((e) => {
          if (controller.signal.aborted || viewerRequestRef.current.generation !== generation) return;
          setViewer((v) =>
            v.kind === "diff" ? { ...v, loading: false, error: String(e?.message ?? e) } : v,
          );
        });
    },
    [cancelViewerRequest, fetchText, workspace],
  );

  const openPatch = useCallback(
    (source: "commit" | "stash", ref: string, label: string) => {
      cancelViewerRequest();
      const controller = new AbortController();
      const generation = viewerRequestRef.current.generation;
      viewerRequestRef.current.controller = controller;
      setViewer({ kind: "patch", title: label || ref, loading: true });
      const param = source === "commit" ? `commit=${encodeURIComponent(ref)}` : `stash=${encodeURIComponent(ref)}`;
      fetchText(`/api/git?workspace=${encodeURIComponent(workspace)}&${param}`, "patch", controller.signal)
        .then((content) => {
          if (viewerRequestRef.current.generation !== generation) return;
          setViewer((v) =>
            v.kind === "patch" ? { ...v, loading: false, content: content || "(empty patch)" } : v,
          );
        })
        .catch((e) => {
          if (controller.signal.aborted || viewerRequestRef.current.generation !== generation) return;
          setViewer((v) =>
            v.kind === "patch" ? { ...v, loading: false, error: String(e?.message ?? e) } : v,
          );
        });
    },
    [cancelViewerRequest, fetchText, workspace],
  );

  const openFile = useCallback(
    (path: string) => {
      cancelViewerRequest();
      const controller = new AbortController();
      const generation = viewerRequestRef.current.generation;
      viewerRequestRef.current.controller = controller;
      setViewer({ kind: "file", title: path, loading: true });
      fetchText(
        `/api/file?workspace=${encodeURIComponent(workspace)}&path=${encodeURIComponent(path)}`,
        "content",
        controller.signal,
      )
        .then((content) => {
          if (viewerRequestRef.current.generation !== generation) return;
          setViewer((v) =>
            v.kind === "file" ? { ...v, loading: false, content } : v,
          );
        })
        .catch((e) => {
          if (controller.signal.aborted || viewerRequestRef.current.generation !== generation) return;
          setViewer((v) =>
            v.kind === "file" ? { ...v, loading: false, error: String(e?.message ?? e) } : v,
          );
        });
    },
    [cancelViewerRequest, fetchText, workspace],
  );

  // Minimal IdeApi shim — GitPanel consumes exactly: state.gitStatus,
  // setGitStatus, openDiff, openPatch, openFile, selectEditor. Everything
  // else is an inert no-op (the hub has no editor/tabs/docks).
  const ide: IdeApi = {
    state: { gitStatus, openTabs: [], activeTabId: null },
    setGitStatus,
    openDiff,
    openPatch,
    openFile,
    selectEditor: () => {},
    setUiMode: () => {},
  };

  const contextValue: IdeContextValue = {
    workspace,
    ide,
    openSettings: () => {},
    openProjects: () => {},
    attachToChat: () => {},
    registerAttachToChat: () => {},
  };

  return (
    <div className="flex h-full min-h-0 flex-col">
      <IdeContext.Provider value={contextValue}>
        <GitPanel />
      </IdeContext.Provider>
      {viewer.kind !== "none" ? (
        <ViewerModal
          viewer={viewer}
          onClose={() => {
            cancelViewerRequest();
            setViewer({ kind: "none" });
          }}
        />
      ) : null}
    </div>
  );
}

function ViewerModal({ viewer, onClose }: { viewer: Exclude<ViewerState, { kind: "none" }>; onClose: () => void }) {
  const closeRef = useOutsideClose(onClose);
  const trapRef = useFocusTrap<HTMLElement>();
  useBodyScrollLock();

  const kindLabel =
    viewer.kind === "diff" ? "Diff" : viewer.kind === "patch" ? "Patch" : "File";

  return (
    <div
      className="fixed inset-0 z-[80] flex items-center justify-center bg-ink-950/80 p-4"
      onMouseDown={onClose}
    >
      <section
        ref={mergeRefs(closeRef, trapRef)}
        role="dialog"
        aria-modal="true"
        aria-label={`${kindLabel}: ${viewer.title}`}
        onMouseDown={(e) => e.stopPropagation()}
        className="flex h-[min(85dvh,60rem)] w-full max-w-4xl flex-col overflow-hidden rounded-sm border border-ink-700 bg-ink-900 shadow-elev-2 animate-fade-in"
      >
        <header className="flex items-center gap-2 border-b border-ink-800 px-3 py-2">
          <span className="shrink-0 rounded-sm border border-ink-700 bg-ink-850 px-1.5 py-0.5 font-mono text-[10px] font-medium uppercase tracking-wider text-ink-400">
            {kindLabel}
          </span>
          <h2 className="min-w-0 flex-1 truncate font-mono text-[12px] text-ink-100" title={viewer.title}>
            {viewer.title}
          </h2>
          <button
            type="button"
            onClick={onClose}
            className="flex min-h-11 min-w-11 items-center justify-center rounded-sm p-1 text-ink-500 hover:bg-ink-800 hover:text-ink-100 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-accent sm:min-h-0 sm:min-w-0"
            aria-label="Close viewer"
          >
            <XIcon width={14} height={14} />
          </button>
        </header>
        <div className="min-h-0 flex-1 overflow-auto">
          {viewer.loading ? (
            <div className="px-4 py-8 text-center text-[12px] text-ink-500">Loading…</div>
          ) : "error" in viewer && viewer.error ? (
            <div className="px-4 py-8 text-center text-[12px] text-danger">{viewer.error}</div>
          ) : viewer.kind === "file" ? (
            <pre className="p-3 font-mono text-[12px] leading-relaxed text-ink-200">
              {viewer.content || "(empty file)"}
            </pre>
          ) : (
            <Diff diff={viewer.content ?? ""} className="border-0" />
          )}
        </div>
      </section>
    </div>
  );
}
