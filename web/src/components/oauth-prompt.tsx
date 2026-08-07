"use client";

// OauthPrompt — surfaces an `oauth_prompt` event from the core.
//
// When a provider login needs the user to visit an authorize URL (and, for the
// device/no-browser flow, paste back a code or final callback URL), the core
// emits `oauth_prompt`. Without handling it the login silently stalls — the
// URL/code never reach the user. This banner shows the URL as a clickable link,
// the device code (when given, with a copy button), and a paste box that
// submits the `oauth_code` command to finish the manual flow (mirrors the TUI's
// /oauth-code).

import { useEffect, useRef, useState } from "react";
import type { OauthPrompt } from "@/lib/types";
import { SendIcon, XIcon, CopyIcon } from "./icons";

function LinkIcon({ width = 16, height = 16, className }: { width?: number; height?: number; className?: string }) {
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
      className={className}
    >
      <path d="M10 13a5 5 0 0 0 7.54.54l3-3a5 5 0 0 0-7.07-7.07l-1.72 1.71" />
      <path d="M14 11a5 5 0 0 0-7.54-.54l-3 3a5 5 0 0 0 7.07 7.07l1.71-1.71" />
    </svg>
  );
}

interface Props {
  prompt: OauthPrompt;
  onSubmit: (code: string) => void;
  onDismiss: () => void;
}

export function OauthPromptBanner({ prompt, onSubmit, onDismiss }: Props) {
  const [text, setText] = useState("");
  const [copied, setCopied] = useState(false);
  const ref = useRef<HTMLTextAreaElement>(null);

  useEffect(() => {
    ref.current?.focus();
  }, []);

  const send = () => {
    const t = text.trim();
    if (!t) return;
    onSubmit(t);
    setText("");
  };

  const onKey = (e: React.KeyboardEvent<HTMLTextAreaElement>) => {
    if (e.key === "Enter" && !e.shiftKey && !e.nativeEvent.isComposing) {
      e.preventDefault();
      send();
    } else if (e.key === "Escape") {
      e.preventDefault();
      onDismiss();
    }
  };

  const copyCode = () => {
    if (!prompt.code) return;
    navigator.clipboard?.writeText(prompt.code).then(
      () => {
        setCopied(true);
        setTimeout(() => setCopied(false), 1500);
      },
      () => {},
    );
  };

  return (
    <div className="my-3 overflow-hidden rounded-sm border border-ink-700 border-l-2 border-l-info bg-ink-925">
      <div className="flex items-center gap-2 border-b border-ink-800 px-4 py-2.5">
        <LinkIcon width={14} height={14} className="shrink-0 text-info" />
        <span className="text-[10px] font-mono uppercase tracking-wider text-ink-400">
          OAuth login required
        </span>
        <button
          onClick={onDismiss}
          className="ml-auto rounded-sm p-1 text-ink-500 transition-colors hover:bg-ink-800 hover:text-ink-100"
          aria-label="Dismiss"
        >
          <XIcon width={14} height={14} />
        </button>
      </div>
      <div className="px-4 py-3">
        {prompt.message && (
          <p className="mb-2 text-[12px] leading-relaxed text-ink-200">{prompt.message}</p>
        )}
        {prompt.url && (
          <a
            href={prompt.url}
            target="_blank"
            rel="noopener noreferrer"
            className="mb-3 block break-all rounded-sm border border-ink-800 bg-ink-950 p-2.5 font-mono text-[11px] leading-relaxed text-accent-soft transition-colors hover:bg-ink-900"
            title="Open the OAuth authorize URL"
          >
            {prompt.url}
          </a>
        )}
        {prompt.code && (
          <div className="mb-3 flex items-center gap-2 rounded-sm border border-ink-800 bg-ink-950 p-2.5">
            <span className="font-mono text-[10px] uppercase tracking-wider text-ink-500">code</span>
            <code className="select-all font-mono text-[12px] font-medium text-ink-100">{prompt.code}</code>
            <button
              onClick={copyCode}
              className="ml-auto flex items-center gap-1 rounded-sm px-1.5 py-1 font-mono text-[10px] uppercase tracking-wider text-ink-400 transition-colors hover:bg-ink-800 hover:text-ink-100"
              title="Copy code"
            >
              <CopyIcon width={12} height={12} />
              {copied ? "copied" : "copy"}
            </button>
          </div>
        )}
        <textarea
          ref={ref}
          rows={2}
          value={text}
          onChange={(e) => setText(e.target.value)}
          onKeyDown={onKey}
          placeholder="Paste the code or final localhost callback URL…"
          className="mb-3 w-full resize-none rounded-sm border border-ink-700 bg-ink-950 px-3 py-2 text-[12px] leading-relaxed text-ink-100 placeholder:text-ink-500 transition-colors focus:border-accent/50 focus:outline-none"
        />
        <div className="flex flex-wrap items-center gap-2">
          <button
            onClick={send}
            className="flex items-center gap-1.5 rounded-sm bg-accent px-2.5 py-1 text-[11px] font-medium text-white transition-colors hover:bg-accent-soft"
          >
            <SendIcon width={13} height={13} /> Submit code
          </button>
          <button
            onClick={onDismiss}
            className="flex items-center gap-1.5 rounded-sm border border-ink-700 px-2.5 py-1 text-[11px] text-ink-300 transition-colors hover:bg-ink-800"
          >
            <XIcon width={13} height={13} /> Dismiss
          </button>
          <span className="ml-auto hidden font-mono text-[10px] text-ink-500 sm:inline">
            <kbd className="rounded-sm border border-ink-800 bg-ink-950 px-1 py-0.5 font-mono text-[10px]">Enter</kbd> submit ·{" "}
            <kbd className="rounded-sm border border-ink-800 bg-ink-950 px-1 py-0.5 font-mono text-[10px]">Esc</kbd> dismiss
          </span>
        </div>
      </div>
    </div>
  );
}
