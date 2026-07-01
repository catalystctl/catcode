"use client";

// Composer — the prompt input. Auto-growing textarea, Enter to send,
// Shift+Enter for newline, and a few TUI-style slash commands (/reset,
// /compact, /new, /abort, /stats, /sessions). Shows Send or Abort depending
// on whether the agent is streaming.

import { useEffect, useRef, useState } from "react";
import { SendIcon, StopIcon, BoltIcon } from "./icons";
import { ImageAttach, fileToDataUrl } from "./attach";

interface Props {
  streaming: boolean;
  connected: boolean;
  canSend: boolean;
  thinkingLevel: string;
  modelLabel: string;
  images: string[];
  onAddImage: (url: string) => void;
  onRemoveImage: (i: number) => void;
  onPrompt: (text: string, images?: string[]) => void;
  onSteer: (text: string) => void;
  onAbort: () => void;
  onCommand: (name: string) => void;
}

const SLASH = new Set([
  "/reset", "/compact", "/new", "/abort", "/stats", "/sessions",
  "/undo", "/clear", "/memory", "/plugins", "/settings", "/subagents",
]);

export function Composer({
  streaming,
  connected,
  canSend,
  thinkingLevel,
  modelLabel,
  images,
  onAddImage,
  onRemoveImage,
  onPrompt,
  onSteer,
  onAbort,
  onCommand,
}: Props) {
  const [text, setText] = useState("");
  const ref = useRef<HTMLTextAreaElement>(null);

  // Auto-grow.
  useEffect(() => {
    const el = ref.current;
    if (!el) return;
    el.style.height = "auto";
    el.style.height = Math.min(el.scrollHeight, 240) + "px";
  }, [text]);

  const submit = () => {
    const t = text.trim();
    if (!t && images.length === 0) return;
    if (t.startsWith("/") && SLASH.has(t)) {
      onCommand(t.slice(1));
      setText("");
      return;
    }
    const imgs = images.length ? images : undefined;
    if (streaming) {
      onSteer(t);
    } else {
      onPrompt(t, imgs);
    }
    setText("");
  };

  const onPaste = (e: React.ClipboardEvent<HTMLTextAreaElement>) => {
    const items = e.clipboardData?.items;
    if (!items) return;
    let added = false;
    for (const it of items) {
      if (it.type.startsWith("image/")) {
        const file = it.getAsFile();
        if (file) {
          added = true;
          fileToDataUrl(file).then(onAddImage).catch(() => {});
        }
      }
    }
    if (added) e.preventDefault();
  };

  const onKey = (e: React.KeyboardEvent<HTMLTextAreaElement>) => {
    if (e.key === "Enter" && !e.shiftKey && !e.nativeEvent.isComposing) {
      e.preventDefault();
      submit();
    }
  };

  const disabled = !connected || !canSend;

  return (
    <div className="border-t border-ink-800/80 bg-ink-950/80 px-4 pb-4 pt-2 backdrop-blur sm:px-6">
      <div className="mx-auto max-w-3xl">
        <div className="relative flex items-end gap-2 rounded-2xl border border-ink-700/70 bg-ink-900/80 p-2 shadow-lg shadow-black/20 transition-colors focus-within:border-accent/50 focus-within:shadow-glow">
          <ImageAttach images={images} onAdd={onAddImage} onRemove={onRemoveImage} />
          <textarea
            ref={ref}
            rows={1}
            value={text}
            aria-label="Message the agent"
            disabled={!connected}
            onChange={(e) => setText(e.target.value)}
            onKeyDown={onKey}
            onPaste={onPaste}
            placeholder={
              connected
                ? streaming
                  ? "Redirect the agent… (Enter to steer)"
                  : "Message the agent…  (/reset /compact /new)"
                : "Connecting to umans-core…"
            }
            className="max-h-60 flex-1 resize-none bg-transparent px-2 py-1.5 text-[14px] leading-relaxed text-ink-100 placeholder:text-ink-500 focus:outline-none disabled:opacity-50"
          />
          {streaming ? (
            <button
              onClick={onAbort}
              className="flex h-9 shrink-0 items-center gap-1.5 rounded-xl border border-rose-500/40 bg-rose-500/10 px-3.5 text-[13px] font-medium text-rose-300 transition-colors hover:bg-rose-500/20"
            >
              <StopIcon width={14} height={14} /> Stop
            </button>
          ) : (
            <button
              onClick={submit}
              disabled={disabled || (!text.trim() && images.length === 0)}
              className="flex h-9 shrink-0 items-center gap-1.5 rounded-xl bg-accent px-3.5 text-[13px] font-semibold text-white transition-all hover:bg-accent-soft disabled:cursor-not-allowed disabled:bg-ink-800 disabled:text-ink-500"
            >
              <SendIcon width={14} height={14} /> Send
            </button>
          )}
        </div>
        <div className="mt-1.5 flex items-center justify-between px-1 text-[11px] text-ink-500">
          <span className="flex items-center gap-1.5">
            <BoltIcon width={11} height={11} className="text-accent-soft" />
            <span className="font-mono">{modelLabel}</span>
            <span className="text-ink-600">·</span>
            <span>think: {thinkingLevel}</span>
          </span>
          <span className="hidden sm:inline">
            <kbd className="rounded bg-ink-800 px-1 py-0.5 font-mono text-[10px]">Enter</kbd> send ·{" "}
            <kbd className="rounded bg-ink-800 px-1 py-0.5 font-mono text-[10px]">Shift+↵</kbd> newline
          </span>
        </div>
      </div>
    </div>
  );
}
