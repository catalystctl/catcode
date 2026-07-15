"use client";
/* eslint-disable @next/next/no-img-element -- workspace image previews use a
   dynamic API path and are intentionally not processed by next/image. */

import { useCallback, useEffect, useRef, useState } from "react";
import type * as Monaco from "monaco-editor";
import { GlobeIcon } from "@/components/icons";
import { useIdeContext } from "@/lib/ide-context";
import { registerEditorModel } from "@/lib/editor-model-registry";
import { canPreviewFile } from "@/lib/preview-support";
import type { IdeTab } from "@/lib/types";
import { currentMonacoTheme, loadMonaco } from "./monaco-loader";

const IMAGE_EXT = /\.(png|jpe?g|gif|webp|svg|bmp|avif)$/i;

type MonacoApi = typeof Monaco;

function Breadcrumbs({ path }: { path: string }) {
  const parts = path.replace(/\\/g, "/").split("/").filter(Boolean);
  return (
    <nav aria-label="File breadcrumb" className="flex h-7 shrink-0 items-center gap-1 overflow-hidden border-b border-ink-900/80 bg-ink-950 px-3 text-[10px] text-ink-500">
      {parts.map((part, index) => (
        <span key={`${part}-${index}`} className="flex min-w-0 items-center gap-1">
          {index > 0 && <span aria-hidden className="text-ink-700">›</span>}
          <span className={`truncate ${index === parts.length - 1 ? "text-ink-300" : ""}`}>{part}</span>
        </span>
      ))}
    </nav>
  );
}

function modelUri(monaco: MonacoApi, workspace: string, path: string): Monaco.Uri {
  const normalized = path.replace(/\\/g, "/").replace(/^\/+/, "");
  return monaco.Uri.from({
    scheme: "catalyst-workspace",
    path: `/${normalized}`,
    query: `workspace=${encodeURIComponent(workspace)}`,
  });
}

/** Match Monaco's complete language registry, including filenames such as
 * Dockerfile and extensions that are not part of the server's small hint map. */
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

function useMonacoTheme(enabled: boolean): void {
  useEffect(() => {
    if (!enabled) return;
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
  }, [enabled]);
}

export function Editor({ tab, onOpenPreview }: { tab: IdeTab; onOpenPreview?: () => void }) {
  const { workspace, ide } = useIdeContext();
  const containerRef = useRef<HTMLDivElement>(null);
  const editorRef = useRef<Monaco.editor.IStandaloneCodeEditor | null>(null);
  const contentRef = useRef("");
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);
  const isImage = IMAGE_EXT.test(tab.target);
  const previewable = canPreviewFile(tab.target);
  const { markDirty, setPreview, showDockPanel } = ide;

  useMonacoTheme(!isImage);

  const save = useCallback(async () => {
    setSaving(true);
    setError(null);
    try {
      const response = await fetch("/api/file", {
        method: "PUT",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ path: tab.target, content: contentRef.current, workspace }),
      });
      if (!response.ok) throw new Error(`Failed to save (${response.status})`);
      markDirty(tab.id, false);
    } catch (reason: unknown) {
      setError(reason instanceof Error ? reason.message : String(reason));
    } finally {
      setSaving(false);
    }
  }, [markDirty, tab.id, tab.target, workspace]);
  const saveRef = useRef(save);
  saveRef.current = save;

  const openPreview = useCallback(() => {
    if (!previewable) return;
    setPreview({ kind: "file", target: tab.target });
    showDockPanel("preview");
    onOpenPreview?.();
  }, [onOpenPreview, previewable, setPreview, showDockPanel, tab.target]);

  const previewButton = previewable ? (
    <button
      type="button"
      onClick={openPreview}
      title={tab.dirty ? "Open saved version in Preview — save changes to refresh" : "Open in Preview"}
      aria-label="Open in Preview"
      className="flex h-7 items-center gap-1.5 rounded-md border border-ink-700 bg-ink-900/90 px-2 text-[11px] font-medium text-ink-300 shadow-sm backdrop-blur transition-colors hover:border-ink-600 hover:bg-ink-800 hover:text-ink-100"
    >
      <GlobeIcon width={13} height={13} />
      <span>Preview</span>
    </button>
  ) : null;

  useEffect(() => {
    if (isImage) {
      setLoading(false);
      return;
    }

    let cancelled = false;
    let changeSubscription: Monaco.IDisposable | undefined;
    let saveAction: Monaco.IDisposable | undefined;
    setLoading(true);
    setError(null);

    void loadMonaco()
      .then(async (monaco) => {
        if (cancelled) return;
        const uri = modelUri(monaco, workspace, tab.target);
        let model = monaco.editor.getModel(uri);

        // Models outlive the visible editor so switching tabs preserves dirty
        // text, selections, and undo history. Only fetch a model the first time.
        if (!model) {
          const response = await fetch(
            `/api/file?path=${encodeURIComponent(tab.target)}&workspace=${encodeURIComponent(workspace)}`,
          );
          if (response.status === 404) throw new Error("File not found");
          if (!response.ok) throw new Error(`Failed to load (${response.status})`);
          const data = (await response.json()) as { content?: string };
          if (cancelled) return;
          model = monaco.editor.createModel(
            data.content ?? "",
            resolveLanguage(monaco, tab.target, tab.language),
            uri,
          );
          markDirty(tab.id, false);
        }

        registerEditorModel(tab.id, () => {
          if (!model?.isDisposed()) model?.dispose();
        });

        contentRef.current = model.getValue();
        if (!containerRef.current || cancelled) return;

        const editor = monaco.editor.create(containerRef.current, {
          model,
          theme: currentMonacoTheme(),
          automaticLayout: true,
          fontFamily: '"JetBrains Mono Variable", "SFMono-Regular", Consolas, monospace',
          fontSize: 13,
          lineHeight: 20,
          fontLigatures: true,
          cursorBlinking: "smooth",
          cursorSmoothCaretAnimation: "on",
          smoothScrolling: true,
          mouseWheelZoom: true,
          minimap: { enabled: true, showSlider: "mouseover", scale: 1 },
          stickyScroll: { enabled: true },
          bracketPairColorization: { enabled: true, independentColorPoolPerBracketType: true },
          guides: { bracketPairs: true, indentation: true, highlightActiveIndentation: true },
          formatOnPaste: true,
          links: true,
          folding: true,
          glyphMargin: true,
          renderWhitespace: "selection",
          renderControlCharacters: true,
          suggest: { showWords: true, preview: true },
          padding: { top: 8, bottom: 8 },
          fixedOverflowWidgets: true,
          scrollBeyondLastLine: false,
        });
        editorRef.current = editor;

        changeSubscription = model.onDidChangeContent(() => {
          contentRef.current = model.getValue();
          markDirty(tab.id, true);
        });
        saveAction = editor.addAction({
          id: "catalyst.save-file",
          label: "Save File",
          keybindings: [monaco.KeyMod.CtrlCmd | monaco.KeyCode.KeyS],
          run: () => saveRef.current(),
        });
        setLoading(false);
        requestAnimationFrame(() => editor.layout());
      })
      .catch((reason: unknown) => {
        if (!cancelled) {
          setError(reason instanceof Error ? reason.message : String(reason));
          setLoading(false);
        }
      });

    return () => {
      cancelled = true;
      changeSubscription?.dispose();
      saveAction?.dispose();
      editorRef.current?.dispose();
      editorRef.current = null;
    };
  }, [isImage, markDirty, tab.id, tab.language, tab.target, workspace]);

  if (isImage) {
    return (
      <div className="relative flex h-full flex-col bg-ink-950">
        <Breadcrumbs path={tab.target} />
        <div className="relative flex min-h-0 flex-1 items-center justify-center overflow-auto p-4">
          <img
            src={`/api/preview?path=${encodeURIComponent(tab.target)}&workspace=${encodeURIComponent(workspace)}`}
            alt={tab.label}
            className="max-h-full max-w-full object-contain"
          />
          <div className="absolute right-3 top-2 z-20">{previewButton}</div>
        </div>
      </div>
    );
  }

  return (
    <div className="relative flex h-full flex-col bg-ink-950">
      <Breadcrumbs path={tab.target} />
      <div ref={containerRef} className="min-h-0 flex-1 overflow-hidden" aria-label={`Editor for ${tab.label}`} />
      {loading ? (
        <div className="absolute inset-0 flex items-center justify-center bg-ink-950 text-sm text-ink-500">
          Loading editor…
        </div>
      ) : null}
      {error ? (
        <div className="absolute inset-x-0 top-0 z-10 border-b border-red-500/30 bg-red-950/90 px-3 py-2 text-center text-xs text-red-200">
          {error}
        </div>
      ) : null}
      <div className="absolute right-3 top-8 z-20 flex items-center gap-2">
        {saving ? (
          <div className="rounded bg-ink-800/80 px-2 py-0.5 text-[11px] text-ink-300">Saving…</div>
        ) : null}
        {previewButton}
      </div>
      {tab.dirty ? (
        <div className="absolute left-3 top-8 z-20 text-[11px] text-amber-300" title="Unsaved changes">
          ●
        </div>
      ) : null}
    </div>
  );
}
