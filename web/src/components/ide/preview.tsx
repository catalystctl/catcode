"use client";

// Preview panel — renders a workspace file (HTML / Markdown / image) or an
// external / loopback URL in a preview surface.
//
//   • Markdown (.md/.markdown): fetched raw from /api/preview and rendered
//     client-side via <Markdown> (react-markdown + rehype-highlight + GFM).
//   • HTML (.html/.htm): served by /api/preview into a SANDBOXED <iframe>
//     (inspect script injected server-side for element → chat).
//   • Images (svg/png/jpg/jpeg/gif/webp): served by /api/preview into an <img>.
//   • PDF: served into an <iframe> (browser built-in viewer).
//   • Loopback URL (localhost / 127.0.0.1): rewritten to /api/dev-proxy/:port
//     so remote browsers can view the host machine's ports without VPN.
//   • Other URL (kind:"url"): rendered directly in an <iframe src=url>.
//
// Inspect mode (toolbar): toggles element picking inside injected HTML; picks
// are attached to the chat composer via IdeContext.attachToChat.

import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { MarkdownDocument } from "@/components/markdown-document";
import { ChevronRight, GlobeIcon, RefreshIcon } from "@/components/icons";
import { useIdeContext } from "@/lib/ide-context";
import { PREVIEW_IMAGE_EXTENSIONS, previewExtension } from "@/lib/preview-support";
import { PREVIEW_INSPECT_SOURCE, toProxiedPreviewSrc } from "@/lib/preview-proxy";
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

/** Crosshair / inspect icon (local to avoid growing the shared icons module). */
function InspectIcon({ width = 14, height = 14 }: { width?: number; height?: number }) {
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
      <path d="M6 2v6M2 6h6" />
      <path d="M18 2v6M22 6h-6" />
      <path d="M6 22v-6M2 18h6" />
      <path d="M18 22v-6M22 18h-6" />
      <circle cx="12" cy="12" r="3" />
    </svg>
  );
}

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

type InspectPayload = {
  tag?: string;
  id?: string;
  classes?: string[];
  selector?: string;
  outerHTML?: string;
  pageUrl?: string;
};

function formatElementForChat(payload: InspectPayload): string {
  const lines = ["[Preview element]"];
  if (payload.selector) lines.push(`selector: ${payload.selector}`);
  if (payload.tag) {
    const bits = [payload.tag];
    if (payload.id) bits.push(`#${payload.id}`);
    if (payload.classes?.length) bits.push(payload.classes.map((c) => `.${c}`).join(""));
    lines.push(`element: ${bits.join("")}`);
  }
  if (payload.pageUrl) lines.push(`url: ${payload.pageUrl}`);
  if (payload.outerHTML) {
    lines.push("html:");
    lines.push("```html");
    lines.push(payload.outerHTML);
    lines.push("```");
  }
  return lines.join("\n");
}

export function Preview({ target, workspace, preview, onPreviewChange }: PreviewProps) {
  const { attachToChat } = useIdeContext();

  return (
    <PreviewInner
      target={target}
      workspace={workspace}
      preview={preview}
      onPreviewChange={onPreviewChange}
      attachToChat={attachToChat}
    />
  );
}

function PreviewInner({
  target,
  workspace,
  preview,
  onPreviewChange,
  attachToChat,
}: PreviewProps & {
  attachToChat: ((p: { text: string; image?: string }) => void) | null;
}) {
  const external: PreviewState = target ? { kind: "url", target } : preview ?? NONE;

  const controlled = typeof onPreviewChange === "function";
  const [internal, setInternal] = useState<PreviewState>(external);
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

  const [addr, setAddr] = useState("");
  useEffect(() => {
    setAddr(active.kind === "none" ? "" : active.target + (active.query ?? ""));
  }, [active.kind, active.target, active.query]);

  const [reloadKey, setReloadKey] = useState(0);
  const reload = useCallback(() => setReloadKey((k) => k + 1), []);

  const fileUrl = useMemo(() => {
    if (active.kind !== "file" || !active.target) return null;
    const qs = new URLSearchParams({ path: active.target });
    if (workspace) qs.set("workspace", workspace);
    return `/api/preview?${qs.toString()}`;
  }, [active.kind, active.target, workspace]);

  const ext = previewExtension(active.target);
  const isMarkdown = ext === "md" || ext === "markdown";
  const isHtml = ext === "html" || ext === "htm";
  const isSvg = ext === "svg";
  const isPdf = ext === "pdf";
  const isImage = PREVIEW_IMAGE_EXTENSIONS.has(ext);
  const isHtmlOrSvg = isHtml || isSvg;

  // Loopback URLs → same-origin proxy so remote clients can view host ports.
  const proxiedUrlSrc = useMemo(() => {
    if (active.kind !== "url" || !active.target) return null;
    return toProxiedPreviewSrc(active.target + (active.query ?? ""));
  }, [active.kind, active.target, active.query]);

  const iframeSrc = useMemo(() => {
    if (active.kind === "url") {
      return proxiedUrlSrc ?? (active.target || undefined);
    }
    return undefined;
  }, [active.kind, active.target, proxiedUrlSrc]);

  const inspectSupported =
    (active.kind === "file" && isHtml) ||
    (active.kind === "url" && !!proxiedUrlSrc);

  const [inspecting, setInspecting] = useState(false);
  const iframeRef = useRef<HTMLIFrameElement>(null);

  // Drop inspect mode when the surface no longer supports it.
  useEffect(() => {
    if (!inspectSupported) setInspecting(false);
  }, [inspectSupported]);

  // Tell the injected script to enable/disable picking.
  useEffect(() => {
    const win = iframeRef.current?.contentWindow;
    if (!win) return;
    try {
      win.postMessage(
        { source: PREVIEW_INSPECT_SOURCE, type: "set-inspect", enabled: inspecting },
        "*",
      );
    } catch {
      /* opaque / cross-origin */
    }
  }, [inspecting, reloadKey, iframeSrc, fileUrl]);

  // Receive picks from the iframe.
  useEffect(() => {
    const onMessage = (ev: MessageEvent) => {
      const data = ev.data;
      if (!data || data.source !== PREVIEW_INSPECT_SOURCE) return;
      if (data.type === "inspect-off") {
        setInspecting(false);
        return;
      }
      if (data.type !== "element" || !data.payload) return;
      if (!attachToChat) return;
      attachToChat({ text: formatElementForChat(data.payload as InspectPayload) });
    };
    window.addEventListener("message", onMessage);
    return () => window.removeEventListener("message", onMessage);
  }, [attachToChat]);

  // Re-assert inspect after iframe load (script may have just installed).
  const onIframeLoad = useCallback(() => {
    if (!inspecting) return;
    const win = iframeRef.current?.contentWindow;
    if (!win) return;
    try {
      win.postMessage(
        { source: PREVIEW_INSPECT_SOURCE, type: "set-inspect", enabled: true },
        "*",
      );
    } catch {
      /* ignore */
    }
  }, [inspecting]);

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
      const src = toProxiedPreviewSrc(active.target + (active.query ?? "")) ?? active.target;
      window.open(src, "_blank", "noopener,noreferrer");
      return;
    }
    // Avoid opening raw /api/preview for HTML/SVG (XSS risk as a top-level document).
    if (isHtmlOrSvg) {
      if (!fileUrl) return;
      void (async () => {
        try {
          const res = await fetch(fileUrl, { cache: "no-store" });
          if (!res.ok) return;
          const text = await res.text();
          const mime = isHtml ? "text/html" : "image/svg+xml";
          const blob = new Blob([text], { type: mime });
          const url = URL.createObjectURL(blob);
          window.open(url, "_blank", "noopener,noreferrer");
          window.setTimeout(() => URL.revokeObjectURL(url), 60_000);
        } catch {
          /* ignore — open-in-tab is best-effort */
        }
      })();
      return;
    }
    if (fileUrl) {
      window.open(fileUrl, "_blank", "noopener,noreferrer");
    }
  }, [active.kind, active.target, active.query, fileUrl, isHtmlOrSvg, isHtml]);

  const canOpen = active.kind === "url" ? !!active.target : !!fileUrl;
  const openTitle = isHtmlOrSvg
    ? "Open safe snapshot in new tab"
    : "Open in new tab";

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

  const toolbar = (
    <div className="flex items-center gap-1 border-b border-ink-800/80 bg-ink-925/60 px-2 py-1.5">
      <button
        type="button"
        onClick={goBack}
        disabled={back.length === 0}
        title="Back"
        aria-label="Back"
        className="flex h-7 w-7 items-center justify-center rounded-md text-ink-400 transition-colors hover:bg-ink-800 hover:text-ink-100 disabled:cursor-not-allowed disabled:opacity-30"
      >
        <ChevronRight className="rotate-180" width={16} height={16} />
      </button>
      <button
        type="button"
        onClick={goForward}
        disabled={fwd.length === 0}
        title="Forward"
        aria-label="Forward"
        className="flex h-7 w-7 items-center justify-center rounded-md text-ink-400 transition-colors hover:bg-ink-800 hover:text-ink-100 disabled:cursor-not-allowed disabled:opacity-30"
      >
        <ChevronRight width={16} height={16} />
      </button>
      <button
        type="button"
        onClick={reload}
        title="Reload"
        aria-label="Reload"
        className="flex h-7 w-7 items-center justify-center rounded-md text-ink-400 transition-colors hover:bg-ink-800 hover:text-ink-100"
      >
        <RefreshIcon width={15} height={15} />
      </button>
      <button
        type="button"
        onClick={() => setInspecting((v) => !v)}
        disabled={!inspectSupported}
        title={
          !inspectSupported
            ? "Select element works on localhost (proxied) and workspace HTML"
            : inspecting
              ? "Stop selecting elements"
              : "Select element to add to chat"
        }
        aria-label="Select element"
        aria-pressed={inspecting}
        className={`flex h-7 w-7 items-center justify-center rounded-md transition-colors disabled:cursor-not-allowed disabled:opacity-30 ${
          inspecting
            ? "bg-accent/20 text-accent-soft"
            : "text-ink-400 hover:bg-ink-800 hover:text-ink-100"
        }`}
      >
        <InspectIcon width={15} height={15} />
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
          className="h-7 min-w-0 flex-1 rounded-md border border-ink-800 bg-ink-950 px-2 font-mono text-[11px] text-ink-200 focus:border-accent/50 focus:outline-none focus:text-ink-100"
        />
      </div>
      <button
        type="button"
        onClick={openInNewTab}
        disabled={!canOpen}
        title={openTitle}
        aria-label={openTitle}
        className="flex h-7 w-7 items-center justify-center rounded-md text-ink-400 transition-colors hover:bg-ink-800 hover:text-ink-100 disabled:cursor-not-allowed disabled:opacity-30"
      >
        <ExternalLinkIcon width={15} height={15} />
      </button>
    </div>
  );

  // Proxied localhost needs allow-same-origin so Astro/Vite client-router
  // fetch() sends the session cookie (opaque sandbox → 401 unauthorized).
  // Safe for *your* loopback apps; workspace HTML stays without same-origin.
  const proxiedSandbox =
    "allow-scripts allow-forms allow-popups allow-modals allow-same-origin";
  const fileHtmlSandbox = "allow-scripts allow-forms allow-popups allow-modals";
  const externalSandbox =
    "allow-scripts allow-forms allow-popups allow-modals allow-same-origin";

  let content: React.ReactNode;
  if (active.kind === "none") {
    content = (
      <div className="flex h-full flex-col items-center justify-center gap-3 p-6 text-center">
        <GlobeIcon width={28} height={28} className="text-ink-600" />
        <p className="text-sm text-ink-400">No preview open</p>
        <p className="max-w-xs text-xs text-ink-500">
          Open an <code className="text-ink-300">.html</code> or{" "}
          <code className="text-ink-300">.md</code> file to preview, or enter a URL above.
          Loopback URLs (<code className="text-ink-300">localhost</code>) are proxied for remote access.
        </p>
      </div>
    );
  } else if (active.kind === "url") {
    const sand = proxiedUrlSrc ? proxiedSandbox : externalSandbox;
    content = (
      <div className="relative h-full">
        <iframe
          key={reloadKey}
          ref={iframeRef}
          src={iframeSrc}
          title="Preview"
          className="h-full w-full border-0 bg-white"
          referrerPolicy="no-referrer"
          sandbox={sand}
          onLoad={onIframeLoad}
        />
        {inspecting && (
          <p className="pointer-events-none absolute top-2 left-1/2 -translate-x-1/2 rounded bg-ink-950/85 px-2 py-1 text-[10px] text-ink-200">
            Click an element to add it to chat · Esc to cancel
          </p>
        )}
        {!proxiedUrlSrc && (
          <p className="pointer-events-none absolute bottom-2 left-1/2 -translate-x-1/2 rounded bg-ink-950/80 px-2 py-1 text-[10px] text-ink-500">
            Blank? The site may block embedding — use “open in new tab”.
          </p>
        )}
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
      content = (
        <div className="relative h-full">
          <iframe
            key={reloadKey}
            ref={isHtml ? iframeRef : undefined}
            src={fileUrl}
            title="Preview"
            className="h-full w-full border-0 bg-white"
            sandbox={fileHtmlSandbox}
            onLoad={isHtml ? onIframeLoad : undefined}
          />
          {isHtml && inspecting && (
            <p className="pointer-events-none absolute top-2 left-1/2 -translate-x-1/2 rounded bg-ink-950/85 px-2 py-1 text-[10px] text-ink-200">
              Click an element to add it to chat · Esc to cancel
            </p>
          )}
        </div>
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
      content = <EmptyHint text={`Cannot preview “.${ext}” files.`} />;
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
