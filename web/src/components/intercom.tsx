"use client";

// Intercom — the subagent supervisor surface.
//
// (1) IntercomPrompt: when a subagent calls contact_supervisor({reason:
// "need_decision"}), the core emits an `intercom_message` event and blocks
// waiting for `intercom_reply`. This banner surfaces the ask + a reply box so
// the orchestrator (the user) can unblock the child. Without it the subagent
// hangs forever — this is the P0 correctness path.
// (2) SubagentPanel: a modal log of recent intercom/subagent activity.

import { useEffect, useRef, useState } from "react";
import type { IntercomEntry, IntercomPrompt } from "@/lib/types";
import { relativeTime } from "@/lib/format";
import { DotIcon, SendIcon, XIcon } from "./icons";
import { useOutsideClose, mergeRefs } from "@/lib/use-outside-close";
import { useFocusTrap } from "@/lib/use-focus-trap";

function QuestionIcon(props: { width?: number; height?: number; className?: string }) {
  const { width = 16, height = 16, className } = props;
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
      <circle cx="12" cy="12" r="10" />
      <path d="M9.1 9a3 3 0 0 1 5.8 1c0 2-3 3-3 3" />
      <path d="M12 17h.01" />
    </svg>
  );
}

interface PromptProps {
  prompt: IntercomPrompt;
  onReply: (reply: string) => void;
  onDismiss: () => void;
}

export function IntercomPrompt({ prompt, onReply, onDismiss }: PromptProps) {
  const [text, setText] = useState("");
  const ref = useRef<HTMLTextAreaElement>(null);

  useEffect(() => {
    ref.current?.focus();
  }, []);

  const send = () => {
    const t = text.trim();
    // An empty "Send reply" used to dispatch an empty reply to the subagent
    // (mirroring the TUI's fixed "Enter does not reply" bug). No-op instead —
    // the user can type a reply or press Skip to defer to the subagent.
    if (!t) return;
    onReply(t);
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

  return (
    <div className="my-3 overflow-hidden rounded-xl border border-warning/30 bg-warning/[0.04]">
      <div className="flex items-center gap-2 border-b border-warning/20 px-4 py-2.5">
        <QuestionIcon width={15} height={15} className="text-warning" />
        <span className="text-sm font-semibold text-ink-100">Subagent needs a decision</span>
        <span className="ml-auto flex items-center gap-1.5 rounded-full bg-ink-850 px-2 py-0.5">
          <span className="text-xs">↳</span>
          <span className="font-mono text-[12px] font-medium text-ink-200">{prompt.from}</span>
        </span>
      </div>
      <div className="px-4 py-3">
        <pre className="mb-3 max-h-40 overflow-auto whitespace-pre-wrap break-words rounded-lg border border-ink-800 bg-ink-950 p-2.5 text-[12px] leading-relaxed text-ink-200">
          <code>{prompt.message}</code>
        </pre>
        <textarea
          ref={ref}
          rows={2}
          value={text}
          onChange={(e) => setText(e.target.value)}
          onKeyDown={onKey}
          placeholder="Reply to the subagent…"
          className="mb-3 w-full resize-none rounded-lg border border-ink-700 bg-ink-950 px-3 py-2 text-[13px] leading-relaxed text-ink-100 placeholder:text-ink-500 focus:border-accent/50 focus:outline-none focus:shadow-glow"
        />
        <div className="flex flex-wrap items-center gap-2">
          <button
            onClick={send}
            disabled={!text.trim()}
            className="flex items-center gap-1.5 rounded-lg bg-accent px-3.5 py-1.5 text-[13px] font-semibold text-white transition-colors hover:bg-accent-soft disabled:cursor-not-allowed disabled:bg-ink-800 disabled:text-ink-500"
          >
            <SendIcon width={14} height={14} /> Send reply
          </button>
          <button
            onClick={onDismiss}
            className="flex items-center gap-1.5 rounded-lg border border-ink-700 px-3.5 py-1.5 text-[13px] font-medium text-ink-300 transition-colors hover:border-danger/40 hover:bg-danger/10 hover:text-danger"
          >
            <XIcon width={14} height={14} /> Skip
          </button>
          <span className="ml-auto hidden text-[11px] text-ink-600 sm:inline">
            <kbd className="rounded bg-ink-800 px-1 py-0.5 font-mono text-[10px]">Enter</kbd> reply ·{" "}
            <kbd className="rounded bg-ink-800 px-1 py-0.5 font-mono text-[10px]">Esc</kbd> skip
          </span>
        </div>
      </div>
    </div>
  );
}

interface PanelProps {
  log: IntercomEntry[];
  onClose: () => void;
}

const KIND_COLOR: Record<IntercomEntry["kind"], string> = {
  ask: "text-warning",
  reply: "text-success",
  status: "text-ink-500",
};

export function SubagentPanel({ log, onClose }: PanelProps) {
  const closeRef = useOutsideClose(onClose);
  const trapRef = useFocusTrap<HTMLDivElement>();
  const entries = [...log].sort((a, b) => b.ts - a.ts);

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/60 p-4 backdrop-blur-sm">
      <div
        ref={mergeRefs(closeRef, trapRef)}
        className="flex max-h-[80vh] w-full max-w-lg flex-col rounded-2xl border border-ink-700 bg-ink-900 shadow-2xl animate-fade-in"
        role="dialog"
        aria-modal="true"
        aria-label="Subagent activity"
      >
        <div className="flex items-center justify-between border-b border-ink-800/80 px-4 py-3">
          <span className="text-[15px] font-semibold text-ink-100">Subagent activity</span>
          <button
            onClick={onClose}
            className="rounded-md p-1 text-ink-500 hover:bg-ink-800 hover:text-ink-100"
            aria-label="Close"
          >
            <XIcon width={16} height={16} />
          </button>
        </div>
        <div className="flex-1 overflow-y-auto p-2">
          {entries.length === 0 ? (
            <div className="px-3 py-10 text-center text-[12px] text-ink-600">
              No subagent activity yet.
            </div>
          ) : (
            <ul className="space-y-1">
              {entries.map((e) => (
                <li
                  key={e.id}
                  className="rounded-lg px-2.5 py-2 transition-colors hover:bg-ink-850/60"
                >
                  <div className="flex items-center gap-2">
                    <DotIcon className={KIND_COLOR[e.kind]} />
                    {e.from && (
                      <span className="font-mono text-[11px] text-ink-400">{e.from}</span>
                    )}
                    {e.to && (
                      <span className="font-mono text-[11px] text-ink-600">→ {e.to}</span>
                    )}
                    <span className="ml-auto text-[10px] text-ink-600">
                      {relativeTime(e.ts)}
                    </span>
                  </div>
                  <div className="mt-0.5 pl-4 text-[12px] leading-relaxed text-ink-200">
                    {e.message}
                  </div>
                </li>
              ))}
            </ul>
          )}
        </div>
      </div>
    </div>
  );
}
