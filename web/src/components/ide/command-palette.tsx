"use client";

import { useEffect, useMemo, useRef, useState } from "react";
import { SearchIcon } from "@/components/icons";
import { useFocusTrap } from "@/lib/use-focus-trap";

export interface PaletteItem {
  id: string;
  label: string;
  detail?: string;
  group: "Commands" | "Files" | "Panels" | "Chats" | "Projects" | "Models";
  keywords?: string;
  run: () => void;
}

export function CommandPalette({ open, items, onClose, onQueryChange }: {
  open: boolean;
  items: PaletteItem[];
  onClose: () => void;
  onQueryChange?: (query: string) => void;
}) {
  const [query, setQuery] = useState("");
  const [selected, setSelected] = useState(0);
  const inputRef = useRef<HTMLInputElement>(null);
  const trapRef = useFocusTrap<HTMLDivElement>(open);
  const results = useMemo(() => {
    const needle = query.trim().toLowerCase();
    if (!needle) return items;
    return items.filter((item) =>
      `${item.label} ${item.detail ?? ""} ${item.group} ${item.keywords ?? ""}`.toLowerCase().includes(needle),
    );
  }, [items, query]);

  useEffect(() => {
    if (!open) return;
    setQuery("");
    setSelected(0);
    requestAnimationFrame(() => inputRef.current?.focus());
  }, [open]);
  useEffect(() => {
    if (open) onQueryChange?.(query);
  }, [onQueryChange, open, query]);
  useEffect(() => setSelected((value) => Math.min(value, Math.max(0, results.length - 1))), [results.length]);
  if (!open) return null;

  const choose = (item: PaletteItem | undefined) => {
    if (!item) return;
    onClose();
    item.run();
  };
  return (
    <div className="fixed inset-0 z-[90] flex justify-center bg-black/55 px-4 pt-[12vh] backdrop-blur-[2px]" onMouseDown={onClose}>
      <div
        ref={trapRef}
        role="dialog"
        aria-modal="true"
        aria-label="Command palette"
        className="flex h-fit max-h-[min(34rem,72vh)] w-full max-w-2xl flex-col overflow-hidden rounded-xl border border-ink-700 bg-ink-925 shadow-2xl"
        onMouseDown={(event) => event.stopPropagation()}
      >
        <label className="flex h-12 shrink-0 items-center gap-3 border-b border-ink-800 px-4">
          <SearchIcon width={18} height={18} className="text-accent-soft" />
          <input
            ref={inputRef}
            value={query}
            onChange={(event) => { setQuery(event.target.value); setSelected(0); }}
            onKeyDown={(event) => {
              if (event.key === "Escape") onClose();
              if (event.key === "ArrowDown") { event.preventDefault(); setSelected((v) => Math.min(v + 1, results.length - 1)); }
              if (event.key === "ArrowUp") { event.preventDefault(); setSelected((v) => Math.max(v - 1, 0)); }
              if (event.key === "Enter") { event.preventDefault(); choose(results[selected]); }
            }}
            placeholder="Search commands, files, chats, projects, and models…"
            role="combobox"
            aria-autocomplete="list"
            aria-expanded={true}
            aria-controls="command-palette-listbox"
            aria-activedescendant={results[selected] ? `palette-option-${results[selected].id}` : undefined}
            className="min-w-0 flex-1 bg-transparent text-sm text-ink-100 outline-none placeholder:text-ink-600"
          />
          <kbd className="rounded border border-ink-700 bg-ink-850 px-1.5 py-0.5 text-[10px] text-ink-500">Esc</kbd>
        </label>
        <div className="overflow-y-auto p-2" role="listbox" id="command-palette-listbox">
          {results.length ? results.map((item, index) => (
            <button
              key={item.id}
              id={`palette-option-${item.id}`}
              type="button"
              role="option"
              aria-selected={selected === index}
              onMouseEnter={() => setSelected(index)}
              onClick={() => choose(item)}
              className={`flex w-full items-center gap-3 rounded-lg px-3 py-2 text-left ${selected === index ? "bg-accent/15 text-ink-100" : "text-ink-300 hover:bg-ink-850"}`}
            >
              <span className="min-w-0 flex-1">
                <span className="block truncate text-sm">{item.label}</span>
                {item.detail ? <span className="block truncate text-[11px] text-ink-500">{item.detail}</span> : null}
              </span>
              <span className="shrink-0 text-[10px] uppercase tracking-wider text-ink-600">{item.group}</span>
            </button>
          )) : <div className="px-3 py-10 text-center text-sm text-ink-600">No matching actions</div>}
        </div>
      </div>
    </div>
  );
}
