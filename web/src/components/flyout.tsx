"use client";

// Flyout — a generic positioned dropdown list with keyboard navigation support.
// Used by the Composer for both slash-command suggestions and @-mention file
// suggestions. The parent owns the items, selectedIndex, and key handling; this
// component just renders the list and reports clicks.

import { useEffect, useRef } from "react";
import type { ReactNode } from "react";

export interface FlyoutItem {
  id: string;
  /** Primary label (e.g. "/reset" or "src/lib/types.ts"). */
  label: string;
  /** Secondary description (e.g. "wipe conversation"). */
  desc?: string;
  /** Optional leading icon/node. */
  icon?: ReactNode;
  /** Optional trailing badge (e.g. "dir"). */
  badge?: string;
}

interface Props {
  items: FlyoutItem[];
  selectedIndex: number;
  onSelect: (index: number) => void;
  onHover: (index: number) => void;
  emptyHint?: string;
}

export function Flyout({ items, selectedIndex, onSelect, onHover, emptyHint }: Props) {
  const listRef = useRef<HTMLDivElement>(null);

  // Scroll the selected item into view when the index changes.
  useEffect(() => {
    const el = listRef.current?.querySelector<HTMLElement>(`[data-idx="${selectedIndex}"]`);
    el?.scrollIntoView({ block: "nearest" });
  }, [selectedIndex]);

  if (items.length === 0) {
    return (
      <div className="absolute bottom-full left-0 right-0 z-40 mb-2 max-h-64 overflow-auto rounded-xl border border-ink-700 bg-ink-900 p-2 text-[12px] text-ink-500 shadow-2xl shadow-black/40 animate-fade-in">
        {emptyHint ?? "No matches"}
      </div>
    );
  }

  return (
    <div
      ref={listRef}
      className="absolute bottom-full left-0 right-0 z-40 mb-2 max-h-64 overflow-auto rounded-xl border border-ink-700 bg-ink-900 p-1 shadow-2xl shadow-black/40 animate-fade-in"
      role="listbox"
    >
      {items.map((item, i) => (
        <button
          key={item.id}
          data-idx={i}
          role="option"
          aria-selected={i === selectedIndex}
          onMouseDown={(e) => {
            // Prevent the textarea from losing focus before the click registers.
            e.preventDefault();
            onSelect(i);
          }}
          onMouseEnter={() => onHover(i)}
          className={`flex w-full items-center gap-2.5 rounded-lg px-2.5 py-1.5 text-left transition-colors ${
            i === selectedIndex ? "bg-accent/15 text-ink-100" : "text-ink-300 hover:bg-ink-850"
          }`}
        >
          {item.icon && <span className="shrink-0 text-ink-400">{item.icon}</span>}
          <div className="min-w-0 flex-1">
            <div className="truncate font-mono text-[12px] font-medium text-ink-100">
              {item.label}
            </div>
            {item.desc && (
              <div className="truncate text-[10px] text-ink-500">{item.desc}</div>
            )}
          </div>
          {item.badge && (
            <span className="shrink-0 rounded bg-ink-800 px-1.5 py-0.5 text-[9px] uppercase tracking-wide text-ink-400">
              {item.badge}
            </span>
          )}
        </button>
      ))}
    </div>
  );
}
