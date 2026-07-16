"use client";

// Thinking — a collapsible reasoning block. While the assistant is streaming
// and has produced thinking but no text yet, show an animated "thinking" state.

import { useEffect, useRef, useState } from "react";
import { BrainIcon, ChevronRight } from "./icons";

export function Thinking({ text, active }: { text: string; active?: boolean }) {
  const [open, setOpen] = useState(false);
  const autoOpenedRef = useRef(false);
  const userToggledRef = useRef(false);
  const wasActiveRef = useRef(!!active);
  // Default open once when reasoning text arrives during an active stream.
  useEffect(() => {
    if (active && text && !autoOpenedRef.current) {
      setOpen(true);
      autoOpenedRef.current = true;
    }
  }, [active, text]);
  // Collapse when the stream ends unless the user manually opened/kept it open.
  useEffect(() => {
    if (wasActiveRef.current && !active && !userToggledRef.current) {
      setOpen(false);
    }
    wasActiveRef.current = !!active;
  }, [active]);

  if (!text && !active) return null;
  const showShimmer = active && !text;

  return (
    <div className="my-1.5">
      <button
        onClick={() => {
          userToggledRef.current = true;
          setOpen((o) => !o);
        }}
        aria-expanded={open}
        className="flex items-center gap-1.5 rounded-md px-1 py-0.5 -ml-1 text-[11px] text-ink-500 transition-colors hover:bg-ink-900/60 hover:text-ink-300"
      >
        <ChevronRight
          width={11}
          height={11}
          className={`shrink-0 transition-transform duration-150 ${open ? "rotate-90" : ""}`}
        />
        <BrainIcon width={12} height={12} className={showShimmer ? "text-accent-soft" : "text-ink-500"} />
        {showShimmer ? (
          <span className="flex items-center gap-1 text-accent-soft">
            thinking
            <span className="inline-flex gap-0.5" aria-hidden="true">
              <span className="h-1 w-1 animate-bounce rounded-full bg-accent-soft [animation-delay:-0.3s]" />
              <span className="h-1 w-1 animate-bounce rounded-full bg-accent-soft [animation-delay:-0.15s]" />
              <span className="h-1 w-1 animate-bounce rounded-full bg-accent-soft" />
            </span>
          </span>
        ) : (
          <span>reasoning{text ? ` · ${text.length.toLocaleString()}` : ""}</span>
        )}
      </button>
      {open && text && (
        <div className="mt-1 ml-4 max-h-80 overflow-auto border-l border-ink-800/60 pl-3 py-1.5">
          <p className="whitespace-pre-wrap break-words font-mono text-[11px] italic leading-relaxed text-ink-400">
            {text}
          </p>
        </div>
      )}
    </div>
  );
}
