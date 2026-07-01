"use client";

// MemoryPanel — list/create/forget persisted workspace memories.
// Memories are injected into the agent's system prompt across sessions.

import { useEffect, useRef, useState } from "react";
import type { MemoryEntry } from "@/lib/types";
import { BrainIcon, PlusIcon, TrashIcon, XIcon } from "./icons";

interface Props {
  memories: MemoryEntry[];
  onSave: (text: string, tags?: string[]) => void;
  onForget: (id: string) => void;
  onClose: () => void;
}

function useOutsideClose(onClose: () => void) {
  const ref = useRef<HTMLDivElement>(null);
  useEffect(() => {
    const h = (e: MouseEvent) => {
      if (ref.current && !ref.current.contains(e.target as Node)) onClose();
    };
    const k = (e: KeyboardEvent) => {
      if (e.key === "Escape") onClose();
    };
    document.addEventListener("mousedown", h);
    document.addEventListener("keydown", k);
    return () => {
      document.removeEventListener("mousedown", h);
      document.removeEventListener("keydown", k);
    };
  }, [onClose]);
  return ref;
}

export function MemoryPanel({ memories, onSave, onForget, onClose }: Props) {
  const [text, setText] = useState("");
  const [tags, setTags] = useState("");
  const ref = useOutsideClose(onClose);

  const save = () => {
    const t = text.trim();
    if (!t) return;
    const tagList = tags
      .split(",")
      .map((s) => s.trim())
      .filter(Boolean);
    onSave(t, tagList.length ? tagList : undefined);
    setText("");
    setTags("");
  };

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/60 p-4 backdrop-blur-sm">
      <div
        ref={ref}
        className="flex max-h-[80vh] w-full max-w-lg flex-col rounded-2xl border border-ink-700 bg-ink-900 shadow-2xl animate-fade-in"
      >
        <div className="flex items-center justify-between border-b border-ink-800/80 px-4 py-3">
          <div className="flex items-center gap-2">
            <BrainIcon width={15} height={15} className="text-accent-soft" />
            <span className="text-[13px] font-semibold text-ink-100">Memories</span>
          </div>
          <button
            onClick={onClose}
            className="rounded-md p-1 text-ink-500 hover:bg-ink-800 hover:text-ink-100"
          >
            <XIcon width={16} height={16} />
          </button>
        </div>

        <div className="flex-1 overflow-y-auto p-4">
          {/* New memory */}
          <div className="mb-3 rounded-xl border border-ink-800 bg-ink-925/40 p-3">
            <textarea
              rows={2}
              value={text}
              onChange={(e) => setText(e.target.value)}
              placeholder="Save a note the agent will remember across sessions…"
              className="w-full resize-none rounded-lg border border-ink-700 bg-ink-950 px-3 py-2 text-[13px] text-ink-100 placeholder:text-ink-600 focus:border-accent/50 focus:outline-none"
            />
            <div className="mt-2 flex items-center gap-2">
              <input
                value={tags}
                onChange={(e) => setTags(e.target.value)}
                placeholder="tags, comma-separated"
                className="flex-1 rounded-lg border border-ink-700 bg-ink-950 px-3 py-1.5 font-mono text-[12px] text-ink-200 placeholder:text-ink-600 focus:border-accent/50 focus:outline-none"
              />
              <button
                onClick={save}
                disabled={!text.trim()}
                className="flex items-center gap-1.5 rounded-lg bg-accent px-3 py-1.5 text-[12px] font-semibold text-white transition-colors hover:bg-accent-soft disabled:cursor-not-allowed disabled:bg-ink-800 disabled:text-ink-500"
              >
                <PlusIcon width={13} height={13} /> Save
              </button>
            </div>
          </div>

          {/* List */}
          {memories.length === 0 ? (
            <div className="px-3 py-6 text-center text-[12px] text-ink-600">
              No memories yet. Save one above.
            </div>
          ) : (
            <div className="space-y-2">
              {memories.map((m) => (
                <div
                  key={m.id}
                  className="group rounded-lg border border-ink-800 bg-ink-925/40 px-3 py-2"
                >
                  <div className="flex items-start gap-2">
                    <p className="flex-1 whitespace-pre-wrap break-words text-[13px] text-ink-200">
                      {m.text}
                    </p>
                    <button
                      onClick={() => {
                        if (window.confirm("Forget this memory?")) onForget(m.id);
                      }}
                      className="shrink-0 rounded-md p-1 text-ink-600 opacity-0 transition-opacity hover:bg-rose-500/10 hover:text-rose-400 group-hover:opacity-100"
                      title="Forget"
                    >
                      <TrashIcon width={13} height={13} />
                    </button>
                  </div>
                  {m.tags && m.tags.length > 0 && (
                    <div className="mt-1.5 flex flex-wrap gap-1">
                      {m.tags.map((t) => (
                        <span
                          key={t}
                          className="rounded bg-ink-800 px-1.5 py-0.5 text-[10px] text-ink-400"
                        >
                          {t}
                        </span>
                      ))}
                    </div>
                  )}
                </div>
              ))}
            </div>
          )}
        </div>
      </div>
    </div>
  );
}
