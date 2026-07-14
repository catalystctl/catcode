"use client";
/* eslint-disable @next/next/no-img-element -- the editor previews workspace
   image files via <img src="/api/preview?..."> per contract §5.2; next/image
   doesn't fit a dynamic workspace-path src. */
// Tabbed code editor. Per docs/IDE_PANELS_CONTRACT.md §5.2.
//   export function Editor({ tab }: { tab: IdeTab })
// CodeMirror 6 via @uiw/react-codemirror. On mount / tab.target change: GET
// /api/file → set content. On edit: ide.markDirty(tab.id, true). Ctrl/Cmd+S →
// PUT /api/file → ide.markDirty(tab.id, false). Language packs are dynamically
// imported per tab.language (§6.3) so only the needed one ships. Non-text image
// files render via <img src="/api/preview?path="> (read-only).
//
// Note: @uiw/react-codemirror annotates programmatic `value` changes with an
// ExternalChange annotation and its update listener SKIPS onChange for those —
// so loading file content via setState does not spuriously mark the file dirty.
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import CodeMirror from "@uiw/react-codemirror";
import { keymap } from "@codemirror/view";
import type { Extension } from "@codemirror/state";
import { useIdeContext } from "@/lib/ide-context";
import type { IdeTab } from "@/lib/types";

const IMAGE_EXT = /\.(png|jpe?g|gif|webp|svg|bmp|avif)$/i;

/** Dynamically load only the needed CodeMirror language extension (§6.3). */
async function loadLangExtension(lang: string | undefined, path: string): Promise<Extension[]> {
  const jsx = /\.(t|j)sx$/.test(path);
  switch (lang) {
    case "typescript":
    case "javascript": {
      const m = await import("@codemirror/lang-javascript");
      return [m.javascript({ typescript: lang === "typescript", jsx })];
    }
    case "python": {
      const m = await import("@codemirror/lang-python");
      return [m.python()];
    }
    case "rust": {
      const m = await import("@codemirror/lang-rust");
      return [m.rust()];
    }
    case "markdown": {
      const m = await import("@codemirror/lang-markdown");
      return [m.markdown()];
    }
    case "json": {
      const m = await import("@codemirror/lang-json");
      return [m.json()];
    }
    case "css": {
      const m = await import("@codemirror/lang-css");
      return [m.css()];
    }
    case "html": {
      const m = await import("@codemirror/lang-html");
      return [m.html()];
    }
    case "yaml": {
      const m = await import("@codemirror/lang-yaml");
      return [m.yaml()];
    }
    case "sql": {
      const m = await import("@codemirror/lang-sql");
      return [m.sql()];
    }
    default:
      return [];
  }
}

export function Editor({ tab }: { tab: IdeTab }) {
  const { workspace, ide } = useIdeContext();
  const [content, setContent] = useState("");
  const [langExt, setLangExt] = useState<Extension[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);
  const isImage = IMAGE_EXT.test(tab.target);
  // `ide` is recreated on every IdeState change (its useMemo deps include
  // `state` so consumers see fresh state). Destructure the stable `markDirty`
  // callback (useCallback []) and depend on IT, not `ide`, in the effects below
  // — otherwise markDirty(false) after a fetch mutates state → new `ide` ref →
  // effect re-runs → cancels the in-flight fetch → infinite "Loading…" loop.
  const { markDirty } = ide;

  // Fetch file content on mount / when the target changes.
  useEffect(() => {
    if (isImage) {
      setLoading(false);
      return;
    }
    let cancelled = false;
    setLoading(true);
    setError(null);
    fetch(`/api/file?path=${encodeURIComponent(tab.target)}&workspace=${encodeURIComponent(workspace)}`)
      .then(async (r) => {
        if (r.status === 404) throw new Error("File not found");
        if (!r.ok) throw new Error(`Failed to load (${r.status})`);
        return (await r.json()) as { content?: string };
      })
      .then((d) => {
        if (cancelled) return;
        setContent(d.content ?? "");
        markDirty(tab.id, false);
      })
      .catch((e: unknown) => {
        if (!cancelled) setError(e instanceof Error ? e.message : String(e));
      })
      .finally(() => {
        if (!cancelled) setLoading(false);
      });
    return () => {
      cancelled = true;
    };
  }, [tab.target, tab.id, workspace, markDirty, isImage]);

  // Load the language extension (dynamic import — only the needed pack ships).
  useEffect(() => {
    let cancelled = false;
    loadLangExtension(tab.language, tab.target).then((ext) => {
      if (!cancelled) setLangExt(ext);
    });
    return () => {
      cancelled = true;
    };
  }, [tab.language, tab.target]);

  const contentRef = useRef(content);
  contentRef.current = content;

  const save = useCallback(async () => {
    setSaving(true);
    try {
      const r = await fetch("/api/file", {
        method: "PUT",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ path: tab.target, content: contentRef.current, workspace }),
      });
      if (r.ok) markDirty(tab.id, false);
    } catch {
      /* ignore transient network errors */
    } finally {
      setSaving(false);
    }
  }, [tab.target, tab.id, workspace, markDirty]);
  const saveRef = useRef(save);
  saveRef.current = save;

  // Ctrl/Cmd+S → save (intercepted so the browser "save page" dialog doesn't fire).
  const saveKeymap = useMemo(
    () =>
      keymap.of([
        { key: "Mod-s", preventDefault: true, run: () => { void saveRef.current(); return true; } },
      ]),
    [],
  );

  const extensions = useMemo(() => [...langExt, saveKeymap], [langExt, saveKeymap]);

  const onChange = useCallback(
    (value: string) => {
      setContent(value);
      markDirty(tab.id, true);
    },
    [markDirty, tab.id],
  );

  if (isImage) {
    return (
      <div className="flex h-full items-center justify-center overflow-auto bg-ink-950 p-4">
        <img
          src={`/api/preview?path=${encodeURIComponent(tab.target)}&workspace=${encodeURIComponent(workspace)}`}
          alt={tab.label}
          className="max-h-full max-w-full object-contain"
        />
      </div>
    );
  }

  if (loading)
    return <div className="flex h-full items-center justify-center text-sm text-ink-500">Loading…</div>;
  if (error)
    return (
      <div className="flex h-full items-center justify-center px-6 text-center text-sm text-ink-500">
        {error}
      </div>
    );

  return (
    <div className="relative flex h-full flex-col bg-ink-950">
      <div className="min-h-0 flex-1 overflow-hidden">
        <CodeMirror
          value={content}
          onChange={onChange}
          extensions={extensions}
          theme="dark"
          height="100%"
          className="h-full text-[13px]"
        />
      </div>
      {saving ? (
        <div className="absolute right-3 top-2 rounded bg-ink-800/80 px-2 py-0.5 text-[11px] text-ink-300">
          Saving…
        </div>
      ) : null}
      {tab.dirty ? (
        <div className="absolute left-3 top-2 text-[11px] text-amber-300" title="Unsaved changes">
          ●
        </div>
      ) : null}
    </div>
  );
}
