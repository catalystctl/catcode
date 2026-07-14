"use client";

// Preview panel — renders a workspace file (HTML / Markdown / image) or an
// external URL in a preview surface.
//
//   • Markdown (.md/.markdown): fetched raw from /api/preview and rendered
//     client-side via <Markdown> (react-markdown + rehype-highlight + GFM).
//   • HTML (.html/.htm): served by /api/preview into a SANDBOXED <iframe>.
//   • Images (svg/png/jpg/jpeg/gif/webp): served by /api/preview into an <img>.
//   • PDF: served into an <iframe> (browser built-in viewer).
//   • External URL (kind:"url"): rendered directly in an <iframe src=url>.
//
// This is a USER-driven panel: it never touches the core agent loop. State is
// supplied via props so the panel is self-contained and does not depend on the
// IDE context/store (which lives in use-ide.ts / ide-context.ts). The optional
// `target` prop overrides the URL (kind=url) — matching the integration
// contract signature `Preview({ target })`.
//
// When `onPreviewChange` is provided the component is CONTROLLED (the parent
// owns the canonical PreviewState and we drive it through back/forward/address
// navigation); without it the panel manages a local copy so the address bar and
// history still work standalone.

import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { MarkdownDocument } from "@/components/markdown-document";
import { ChevronRight, GlobeIcon, RefreshIcon } from "@/components/icons";
import { PREVIEW_IMAGE_EXTENSIONS, previewExtension } from "@/lib/preview-support";
import type { PreviewState } from "@/lib/types";

export interface PreviewProps {
  /** Optional override: preview this URL directly (kind="url"). */
  target?: string;
  /** Absolute workspace path (forwarded to /api/preview as ?workspace=). */
  workspace?: string;
  /** Current preview state. Defaults to { kind: "none", target: "" }. */
  preview?: PreviewState;
  /** Notifies the parent of a navigation (back/forward/address submit). */
  onPreviewChange?: (p: PreviewState) => void;
}

const NONE: PreviewState = { kind: "none", target: "" };

/** Small inline icon (kept local to avoid touching the shared icons module). */
function ExternalLinkIcon({ width = 14, height = 14 }: { width?: number; height?: number }) {
  return (
    <svg
      width={width}
      height={height}
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth={2}
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden="true"
    >
      <path d="M18 13v6a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V8a2 2 0 0 1 2-2h6" />
      <path d="M15 3h6v6" />
      <path d="M10 14L21 3" />
    </svg>
  );
}

export function Preview({ target, workspace, preview, onPreviewChange }: PreviewProps) {
  // External (prop-driven) preview; `target` prop wins as a URL.
  const external: PreviewState = target ? { kind: "url", target } : preview ?? NONE;

  const controlled = typeof onPreviewChange === "function";
  const [internal, setInternal] = useState<PreviewState>(external);
  // Sync the local copy when the parent's preview changes (uncontrolled mode).
  useEffect(() => {
    if (!controlled) setInternal(external);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [external.kind, external.target, external.query, controlled]);

  const active = controlled ? external : internal;
  const commit = useCallback(
    (p: PreviewState) => {
      if (controlled) onPreviewChange!(p);
      else setInternal(p);
    },
    [controlled, onPreviewChange],
  );

  // Local back/forward history (wraps commit). Independent of parent state.
  const [back, setBack] = useState<PreviewState[]>([]);
  const [fwd, setFwd] = useState<PreviewState[]>([]);
  const navigate = useCallback(
    (p: PreviewState) => {
      setBack((b) => [...b, active]);
      setFwd([]);
      commit(p);
    },
    [active, commit],
  );
  const goBack = useCallback(() => {
    if (back.length === 0) return;
    const prev = back[back.length - 1];
    setBack((b) => b.slice(0, -1));
    setFwd((f) => [active, ...f]);
    commit(prev);
  }, [back, active, commit]);
  const goForward = useCallback(() => {
    if (fwd.length === 0) return;
    const next = fwd[0];
    setFwd((f) => f.slice(1));
    setBack((b) => [...b, active]);
    commit(next);
  }, [fwd, active, commit]);

  // Address bar text (kept in sync with the active target).
  const [addr, setAddr] = useState("");
  useEffect(() => {
    setAddr(active.kind === "none" ? "" : active.target + (active.query ?? ""));
  }, [active.kind, active.target, active.query]);

  // Reload: bump a key that remounts the iframe/img and re-fetches markdown.
  const [reloadKey, setReloadKey] = useState(0);
  const reload = useCallback(() => setReloadKey((k) => k + 1), []);

  // The /api/preview URL for a workspace file (only valid for kind="file").
  const fileUrl = useMemo(() => {
    if (active.kind !== "file" || !active.target) return null;
    const qs = new URLSearchParams({ path: active.target });
    if (workspace) qs.set("workspace", workspace);
    return `/api/preview?${qs.toString()}`;
  }, [active.kind, active.target, workspace]);

  const ext = previewExtension(active.target);
  const isMarkdown = ext === "md" || ext === "markdown";
  const isHtml = ext === "html" || ext === "htm";
  const isPdf = ext === "pdf";
  const isImage = PREVIEW_IMAGE_EXTENSIONS.has(ext);

  // ── Markdown fetch (rendered client-side) ──
  const [md, setMd] = useState<string | null>(null);
  const [mdError, setMdError] = useState<string | null>(null);
  const [mdLoading, setMdLoading] = useState(false);
  const fetchingRef = useRef(0);
  useEffect(() => {
    if (active.kind !== "file" || !isMarkdown || !fileUrl) {
      setMd(null);
      setMdError(null);
      setMdLoading(false);
      return;
    }
    const myToken = ++fetchingRef.current;
    let cancelled = false;
    setMdLoading(true);
    setMdError(null);
    fetch(fileUrl)
      .then(async (res) => {
        if (!res.ok) {
          const msg =
            res.status === 403
              ? "secret file blocked"
              : res.status === 404
                ? "file not found"
                : res.status === 401
                  ? "unauthorized"
                  : `preview error (${res.status})`;
          throw new Error(msg);
        }
        return res.text();
      })
      .then((text) => {
        if (cancelled || myToken !== fetchingRef.current) return;
        setMd(text);
        setMdLoading(false);
      })
      .catch((e: unknown) => {
        if (cancelled || myToken !== fetchingRef.current) return;
        setMdError(e instanceof Error ? e.message : "fetch failed");
        setMdLoading(false);
      });
    return () => {
      cancelled = true;
    };
  }, [fileUrl, isMarkdown, active.kind, reloadKey, active.target]);

  const onAddrSubmit = useCallback(() => {
    const v = addr.trim();
    if (!v) return;
    const isUrl = /^https?:\/\//i.test(v);
    navigate({ kind: isUrl ? "url" : "file", target: v });
  }, [addr, navigate]);

  const openInNewTab = useCallback(() => {
    if (active.kind === "url" && active.target) {
      window.open(active.target, "_blank", "noopener,noreferrer");
    } else if (fileUrl) {
      window.open(fileUrl, "_blank", "noopener,noreferrer");
    }
  }, [active.kind, active.target, fileUrl]);

  const canOpen = active.kind === "url" ? !!active.target : !!fileUrl;

  const resolveMarkdownImage = useCallback(
    (source: string) => {
      if (!source || /^(?:[a-z][a-z\d+.-]*:|\/\/|#)/i.test(source)) return source;
      const cleanSource = source.split(/[?#]/, 1)[0].replace(/\\/g, "/");
      const base = source.startsWith("/")
        ? []
        : active.target.replace(/\\/g, "/").split("/").slice(0, -1);
      for (const segment of cleanSource.replace(/^\/+/, "").split("/")) {
        if (!segment || segment === ".") continue;
        if (segment === "..") base.pop();
        else base.push(segment);
      }
      const qs = new URLSearchParams({ path: base.join("/") });
      if (workspace) qs.set("workspace", workspace);
      return `/api/preview?${qs.toString()}`;
    },
    [active.target, workspace],
  );

  // ── Toolbar ──
  const toolbar = (
    <div className="flex items-center gap-1 border-b border-ink-800/80 bg-ink-925/60 px-2 py-1.5">
      <button
        onClick={goBack}
        disabled={back.length === 0}
        title="Back"
        aria-label="Back"
        className="flex h-7 w-7 items-center justify-center rounded-md text-ink-400 transition-colors hover:bg-ink-800 hover:text-ink-100 disabled:cursor-not-allowed disabled:opacity-30"
      >
        <ChevronRight className="rotate-180" width={16} height={16} />
      </button>
      <button
        onClick={goForward}
        disabled={fwd.length === 0}
        title="Forward"
        aria-label="Forward"
        className="flex h-7 w-7 items-center justify-center rounded-md text-ink-400 transition-colors hover:bg-ink-800 hover:text-ink-100 disabled:cursor-not-allowed disabled:opacity-30"
      >
        <ChevronRight width={16} height={16} />
      </button>
      <button
        onClick={reload}
        title="Reload"
        aria-label="Reload"
        className="flex h-7 w-7 items-center justify-center rounded-md text-ink-400 transition-colors hover:bg-ink-800 hover:text-ink-100"
      >
        <RefreshIcon width={15} height={15} />
      </button>
      <div className="flex min-w-0 flex-1 items-center gap-1.5">
        <GlobeIcon width={13} height={13} className="shrink-0 text-ink-500" />
        <input
          value={addr}
          onChange={(e) => setAddr(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter") {
              e.preventDefault();
              onAddrSubmit();
            }
          }}
          placeholder="Enter a URL or workspace file path…"
          spellCheck={false}
          autoComplete="off"
          className="h-7 min-w-0 flex-1 rounded-md border border-ink-800 bg-ink-950 px-2 font-mono text-[11px] text-ink-200 outline-none focus:border-ink-600 focus:text-ink-100"
        />
      </div>
      <button
        onClick={openInNewTab}
        disabled={!canOpen}
        title="Open in new tab"
        aria-label="Open in new tab"
        className="flex h-7 w-7 items-center justify-center rounded-md text-ink-400 transition-colors hover:bg-ink-800 hover:text-ink-100 disabled:cursor-not-allowed disabled:opacity-30"
      >
        <ExternalLinkIcon width={15} height={15} />
      </button>
    </div>
  );

  // ── Content ──
  let content: React.ReactNode;
  if (active.kind === "none") {
    content = (
      <div className="flex h-full flex-col items-center justify-center gap-3 p-6 text-center">
        <GlobeIcon width={28} height={28} className="text-ink-600" />
        <p className="text-sm text-ink-400">No preview open</p>
        <p className="max-w-xs text-xs text-ink-500">
          Open an <code className="text-ink-300">.html</code> or{" "}
          <code className="text-ink-300">.md</code> file to preview, or enter a URL above.
        </p>
      </div>
    );
  } else if (active.kind === "url") {
    content = (
      <div className="relative h-full">
        <iframe
          key={reloadKey}
          src={active.target || undefined}
          title="Preview"
          className="h-full w-full border-0 bg-white"
          referrerPolicy="no-referrer"
          sandbox="allow-scripts allow-forms allow-popups allow-modals allow-same-origin"
        />
        <p className="pointer-events-none absolute bottom-2 left-1/2 -translate-x-1/2 rounded bg-ink-950/80 px-2 py-1 text-[10px] text-ink-500">
          Blank? The site may block embedding — use “open in new tab”.
        </p>
      </div>
    );
  } else if (active.kind === "file") {
    if (!fileUrl) {
      content = <EmptyHint text="No file selected." />;
    } else if (isMarkdown) {
      if (mdError) {
        content = <EmptyHint text={mdError} tone="error" />;
      } else if (mdLoading) {
        content = <EmptyHint text="Loading…" />;
      } else if (md != null) {
        content = (
          <div className="h-full overflow-auto bg-ink-950 px-4 py-5 text-ink-200 sm:px-7 sm:py-7">
            <MarkdownDocument resolveImageUrl={resolveMarkdownImage}>{md}</MarkdownDocument>
          </div>
        );
      } else {
        content = <EmptyHint text="Loading…" />;
      }
    } else if (isHtml || isPdf) {
      // Sandbox WITHOUT allow-same-origin for workspace HTML so a previewed file
      // cannot reach the app's cookies/localStorage or call authenticated
      // /api/* routes. The document itself still loads (the top-level request
      // is same-origin and carries the auth cookie); only in-page script fetches
      // are de-originated. PDF uses the same frame (browser viewer).
      content = (
        <iframe
          key={reloadKey}
          src={fileUrl}
          title="Preview"
          className="h-full w-full border-0 bg-white"
          sandbox="allow-scripts allow-forms allow-popups allow-modals"
        />
      );
    } else if (isImage) {
      content = (
        <div className="flex h-full items-center justify-center overflow-auto bg-ink-900 p-4">
          {/* eslint-disable-next-line @next/next/no-img-element */}
          <img
            key={reloadKey}
            src={fileUrl}
            alt={active.target}
            className="max-h-full max-w-full object-contain"
          />
        </div>
      );
    } else {
      content = (
        <EmptyHint text={`Cannot preview “.${ext}” files.`} />
      );
    }
  } else {
    content = <EmptyHint text="Nothing to preview." />;
  }

  return (
    <div className="flex h-full min-h-0 w-full flex-col bg-ink-950">
      {toolbar}
      <div className="relative min-h-0 flex-1">{content}</div>
    </div>
  );
}

function EmptyHint({ text, tone }: { text: string; tone?: "error" }) {
  return (
    <div className="flex h-full items-center justify-center p-6 text-center">
      <p className={`text-xs ${tone === "error" ? "text-red-400" : "text-ink-500"}`}>{text}</p>
    </div>
  );
}
