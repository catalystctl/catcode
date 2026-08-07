"use client";

// Thinking — a flat collapsible reasoning block: a left-border rail with a mono
// header label. While the assistant is streaming and has produced thinking but
// no text yet, show an animated "thinking" state.

import { useEffect, useRef, useState } from "react";
import { ChevronRight } from "./icons";

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
    <div className="my-1.5 border-l-2 border-ink-700 pl-2">
      <button
        onClick={() => {
          userToggledRef.current = true;
          setOpen((o) => !o);
        }}
        aria-expanded={open}
        className="flex items-center gap-1.5 py-1 font-mono text-[10px] uppercase tracking-wider text-ink-500 transition-colors hover:text-ink-300"
      >
        <ChevronRight
          width={11}
          height={11}
          className={`shrink-0 transition-transform duration-150 ${open ? "rotate-90" : ""}`}
        />
        {showShimmer ? (
          <span className="shimmer-text">Thinking…</span>
        ) : active ? (
          <span className="text-accent-soft">reasoning</span>
        ) : (
          <span>reasoning{text ? ` · ${text.length.toLocaleString()} chars` : ""}</span>
        )}
      </button>
      {open && text && (
        <div className="mt-1 max-h-80 overflow-auto pr-1">
          <p className="whitespace-pre-wrap break-words font-mono text-[12px] leading-relaxed text-ink-400">
            {text}
          </p>
        </div>
      )}
    </div>
  );
}
