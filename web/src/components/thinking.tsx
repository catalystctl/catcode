"use client";

// Thinking — a collapsible reasoning block. While the assistant is streaming
// and has produced thinking but no text yet, show an animated "thinking" state.

import { useEffect, useRef, useState } from "react";
import { BrainIcon, ChevronRight } from "./icons";

export function Thinking({ text, active }: { text: string; active?: boolean }) {
  const [open, setOpen] = useState(false);
  const autoOpenedRef = useRef(false);
  // Default open once when reasoning text arrives during an active stream.
  useEffect(() => {
    if (active && text && !autoOpenedRef.current) {
      setOpen(true);
      autoOpenedRef.current = true;
    }
  }, [active, text]);

  if (!text && !active) return null;
  const showShimmer = active && !text;

  return (
    <div className="my-2">
      <button
        onClick={() => setOpen((o) => !o)}
        aria-expanded={open}
        className="flex items-center gap-2 text-[12px] text-ink-400 transition-colors hover:text-ink-200"
      >
        <ChevronRight
          width={12}
          height={12}
          className={`transition-transform ${open ? "rotate-90" : ""}`}
        />
        <BrainIcon width={13} height={13} className={showShimmer ? "text-accent-soft" : ""} />
        {showShimmer ? (
          <span className="flex items-center gap-1">
            thinking
            <span className="inline-flex gap-0.5">
              <span className="h-1 w-1 animate-bounce rounded-full bg-accent-soft [animation-delay:-0.3s]" />
              <span className="h-1 w-1 animate-bounce rounded-full bg-accent-soft [animation-delay:-0.15s]" />
              <span className="h-1 w-1 animate-bounce rounded-full bg-accent-soft" />
            </span>
          </span>
        ) : (
          <span>reasoning {text ? `· ${text.length.toLocaleString()} chars` : ""}</span>
        )}
      </button>
      {open && text && (
        <div className="mt-1.5 ml-5 max-h-96 overflow-auto rounded-lg border border-ink-800/70 bg-ink-925/50 p-3">
          <p className="whitespace-pre-wrap break-words font-mono text-[12px] italic leading-relaxed text-ink-300">
            {text}
          </p>
        </div>
      )}
    </div>
  );
}
