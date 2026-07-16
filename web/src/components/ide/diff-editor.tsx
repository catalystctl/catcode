"use client";

// Monaco DiffEditor for Source Control file diffs. Opens as an IdeTab (kind
// "diff") in the main work area. Toolbar: Unified | Split | Collapse changes.

import { useEffect, useRef, useState } from "react";
import type * as Monaco from "monaco-editor";
import { useIdeContext } from "@/lib/ide-context";
import type { IdeTab } from "@/lib/types";
import { currentMonacoTheme, loadMonaco } from "./monaco-loader";

type MonacoApi = typeof Monaco;
type ViewMode = "unified" | "split";

function resolveLanguage(monaco: MonacoApi, path: string, hint?: string): string {
  const normalized = path.replace(/\\/g, "/").toLowerCase();
  const filename = normalized.split("/").pop() ?? normalized;
  const languages = monaco.languages.getLanguages();
  const filenameMatch = languages.find((language) =>
    language.filenames?.some((name) => name.toLowerCase() === filename),
  );
  if (filenameMatch) return filenameMatch.id;

  const extensionMatch = languages
    .flatMap((language) => (language.extensions ?? []).map((extension) => ({ language, extension })))
    .sort((a, b) => b.extension.length - a.extension.length)
    .find(({ extension }) => normalized.endsWith(extension.toLowerCase()));
  if (extensionMatch) return extensionMatch.language.id;

  if (hint && languages.some((language) => language.id === hint)) return hint;
  return "plaintext";
}

function toolbarBtn(active: boolean): string {
  return `rounded px-2 py-1 text-[11px] font-medium transition-colors ${
    active
      ? "bg-ink-700 text-ink-100"
      : "text-ink-400 hover:bg-ink-800 hover:text-ink-200"
  }`;
}

export function DiffEditor({ tab }: { tab: IdeTab }) {
  const { workspace, ide } = useIdeContext();
  const containerRef = useRef<HTMLDivElement>(null);
  const editorRef = useRef<Monaco.editor.IStandaloneDiffEditor | null>(null);
  const modelsRef = useRef<{ original: Monaco.editor.ITextModel; modified: Monaco.editor.ITextModel } | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [viewMode, setViewMode] = useState<ViewMode>("split");
  const [collapse, setCollapse] = useState(true);
  const viewModeRef = useRef(viewMode);
  const collapseRef = useRef(collapse);
  viewModeRef.current = viewMode;
  collapseRef.current = collapse;
  const staged = !!tab.diffMeta?.staged;

  useEffect(() => {
    let cancelled = false;
    setLoading(true);
    setError(null);

    void (async () => {
      try {
        const response = await fetch(
          `/api/git?workspace=${encodeURIComponent(workspace)}&sides=${encodeURIComponent(tab.target)}&staged=${staged ? 1 : 0}`,
        );
        const data = (await response.json().catch(() => ({}))) as {
          original?: string;
          modified?: string;
          error?: string;
        };
        if (!response.ok) throw new Error(data.error ?? `Failed to load diff (${response.status})`);
        if (cancelled) return;

        const monaco = await loadMonaco();
        if (cancelled || !containerRef.current) return;

        editorRef.current?.dispose();
        modelsRef.current?.original.dispose();
        modelsRef.current?.modified.dispose();
        modelsRef.current = null;
        editorRef.current = null;

        const language = resolveLanguage(monaco, tab.target, tab.language);
        const originalUri = monaco.Uri.from({
          scheme: "catalyst-diff",
          path: `/${tab.id}/original/${tab.target}`,
        });
        const modifiedUri = monaco.Uri.from({
          scheme: "catalyst-diff",
          path: `/${tab.id}/modified/${tab.target}`,
        });
        monaco.editor.getModel(originalUri)?.dispose();
        monaco.editor.getModel(modifiedUri)?.dispose();

        const original = monaco.editor.createModel(data.original ?? "", language, originalUri);
        const modified = monaco.editor.createModel(data.modified ?? "", language, modifiedUri);
        modelsRef.current = { original, modified };

        const editor = monaco.editor.createDiffEditor(containerRef.current, {
          theme: currentMonacoTheme(),
          automaticLayout: true,
          readOnly: true,
          originalEditable: false,
          renderSideBySide: viewModeRef.current === "split",
          renderOverviewRuler: true,
          enableSplitViewResizing: true,
          ignoreTrimWhitespace: false,
          fontFamily: '"JetBrains Mono Variable", "SFMono-Regular", Consolas, monospace',
          fontSize: 13,
          lineHeight: 20,
          fontLigatures: true,
          minimap: { enabled: false },
          scrollBeyondLastLine: false,
          padding: { top: 8, bottom: 8 },
          hideUnchangedRegions: {
            enabled: collapseRef.current,
            contextLineCount: 3,
            minimumLineCount: 3,
            revealLineCount: 20,
          },
        });
        editor.setModel({ original, modified });
        editorRef.current = editor;
        setLoading(false);
        requestAnimationFrame(() => editor.layout());
      } catch (reason: unknown) {
        if (!cancelled) {
          setError(reason instanceof Error ? reason.message : String(reason));
          setLoading(false);
        }
      }
    })();

    return () => {
      cancelled = true;
      editorRef.current?.dispose();
      editorRef.current = null;
      modelsRef.current?.original.dispose();
      modelsRef.current?.modified.dispose();
      modelsRef.current = null;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps -- remount on tab identity only
  }, [workspace, tab.id, tab.target, tab.language, staged]);

  useEffect(() => {
    const editor = editorRef.current;
    if (!editor) return;
    editor.updateOptions({
      renderSideBySide: viewMode === "split",
      hideUnchangedRegions: {
        enabled: collapse,
        contextLineCount: 3,
        minimumLineCount: 3,
        revealLineCount: 20,
      },
    });
  }, [viewMode, collapse]);

  useEffect(() => {
    let cancelled = false;
    const apply = () => {
      void loadMonaco().then((monaco) => {
        if (!cancelled) monaco.editor.setTheme(currentMonacoTheme());
      });
    };
    apply();
    const observer = new MutationObserver(apply);
    observer.observe(document.documentElement, { attributes: true, attributeFilter: ["data-theme"] });
    return () => {
      cancelled = true;
      observer.disconnect();
    };
  }, []);

  return (
    <div className="relative flex h-full flex-col bg-ink-950">
      <div className="flex h-8 shrink-0 items-center gap-1 border-b border-ink-850 bg-ink-950 px-2">
        <span className="mr-2 truncate font-mono text-[11px] text-ink-500" title={tab.target}>
          {staged ? "Staged" : "Changes"} · {tab.target}
        </span>
        <div className="ml-auto flex items-center gap-0.5 rounded-md border border-ink-800 bg-ink-900/60 p-0.5">
          <button
            type="button"
            className={toolbarBtn(viewMode === "unified")}
            aria-pressed={viewMode === "unified"}
            onClick={() => setViewMode("unified")}
            title="Unified diff"
          >
            Unified
          </button>
          <button
            type="button"
            className={toolbarBtn(viewMode === "split")}
            aria-pressed={viewMode === "split"}
            onClick={() => setViewMode("split")}
            title="Side-by-side diff"
          >
            Split
          </button>
        </div>
        <button
          type="button"
          className={`${toolbarBtn(collapse)} ml-1 border border-ink-800`}
          aria-pressed={collapse}
          onClick={() => setCollapse((v) => !v)}
          title={collapse ? "Show all lines" : "Collapse unchanged regions"}
        >
          {collapse ? "Show all" : "Collapse unchanged"}
        </button>
        <button
          type="button"
          className={`${toolbarBtn(false)} ml-1 border border-ink-800`}
          onClick={() => ide.openFile(tab.target)}
          title="Open file"
        >
          Open file
        </button>
      </div>
      <div ref={containerRef} className="min-h-0 flex-1 overflow-hidden" aria-label={`Diff for ${tab.label}`} />
      {loading ? (
        <div className="absolute inset-x-0 bottom-0 top-8 flex items-center justify-center bg-ink-950 text-sm text-ink-500">
          Loading diff…
        </div>
      ) : null}
      {error ? (
        <div className="absolute inset-x-0 top-8 z-10 border-b border-danger/30 bg-danger/10 px-3 py-2 text-center text-xs text-danger">
          {error}
        </div>
      ) : null}
    </div>
  );
}

/** Read-only unified patch viewer (commits / stashes) with collapse-context. */
export function PatchViewer({ tab }: { tab: IdeTab }) {
  const { workspace } = useIdeContext();
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [patch, setPatch] = useState("");
  const [collapse, setCollapse] = useState(true);
  const source = tab.diffMeta?.source === "stash" ? "stash" : "commit";

  useEffect(() => {
    let cancelled = false;
    setLoading(true);
    setError(null);
    const param = source === "stash" ? "stash" : "commit";
    void fetch(
      `/api/git?workspace=${encodeURIComponent(workspace)}&${param}=${encodeURIComponent(tab.target)}`,
    )
      .then(async (response) => {
        const data = (await response.json().catch(() => ({}))) as { patch?: string; error?: string };
        if (!response.ok) throw new Error(data.error ?? `Failed to load ${source}`);
        if (!cancelled) setPatch(typeof data.patch === "string" ? data.patch : "");
      })
      .catch((reason: unknown) => {
        if (!cancelled) setError(reason instanceof Error ? reason.message : String(reason));
      })
      .finally(() => {
        if (!cancelled) setLoading(false);
      });
    return () => {
      cancelled = true;
    };
  }, [workspace, tab.target, source]);

  const lines = patch.split("\n");
  const visible = collapse
    ? lines.filter((line, index) => {
        if (!line) return false;
        if (line.startsWith("@@") || line.startsWith("diff ") || line.startsWith("index ")) return true;
        if (line.startsWith("---") || line.startsWith("+++")) return true;
        if (line.startsWith("+") || line.startsWith("-")) return true;
        const prev = lines[index - 1] ?? "";
        const next = lines[index + 1] ?? "";
        return (
          prev.startsWith("+") ||
          prev.startsWith("-") ||
          next.startsWith("+") ||
          next.startsWith("-") ||
          prev.startsWith("@@") ||
          next.startsWith("@@")
        );
      })
    : lines;

  return (
    <div className="relative flex h-full flex-col bg-ink-950">
      <div className="flex h-8 shrink-0 items-center gap-1 border-b border-ink-850 px-2">
        <span className="mr-2 truncate font-mono text-[11px] text-ink-500">{tab.label}</span>
        <button
          type="button"
          className={`${toolbarBtn(collapse)} ml-auto border border-ink-800`}
          aria-pressed={collapse}
          onClick={() => setCollapse((v) => !v)}
        >
          {collapse ? "Show all" : "Collapse unchanged"}
        </button>
      </div>
      <div className="min-h-0 flex-1 overflow-auto p-3">
        {loading ? (
          <div className="text-sm text-ink-500">Loading…</div>
        ) : error ? (
          <div className="text-xs text-danger">{error}</div>
        ) : patch ? (
          <pre className="overflow-x-auto rounded-lg border border-ink-800 bg-ink-950 p-3 text-[12px] leading-relaxed">
            {visible.map((line, i) => {
              const cls =
                line.startsWith("+") && !line.startsWith("+++")
                  ? "diff-line-add"
                  : line.startsWith("-") && !line.startsWith("---")
                    ? "diff-line-del"
                    : line.startsWith("@@")
                      ? "diff-line-hunk"
                      : "";
              return (
                <div key={i} className={`${cls} px-1`}>
                  {line || " "}
                </div>
              );
            })}
          </pre>
        ) : (
          <div className="text-sm text-ink-500">No patch available.</div>
        )}
      </div>
    </div>
  );
}
