"use client";

// MemoryPanel — list/create/forget persisted workspace + global memories.
// Each memory shows its name (title), description (subtitle), the full content
// (expandable), a scope badge (workspace/global), and its type tag. Memories
// are injected into the agent's system prompt across sessions.

import { useState } from "react";
import type { MemoryEntry } from "@/lib/types";
import { useOutsideClose, mergeRefs } from "@/lib/use-outside-close";
import { useFocusTrap } from "@/lib/use-focus-trap";
import { AppDialogHost, useAppDialog } from "./app-dialog";
import {
  BrainIcon,
  PlusIcon,
  TrashIcon,
  XIcon,
  ChevronDown,
  GlobeIcon,
  FolderIcon,
  RefreshIcon,
} from "./icons";

interface Props {
  memories: MemoryEntry[];
  onSave: (text: string, tags?: string[], scope?: "workspace" | "global") => void;
  onForget: (id: string) => void;
  onRefresh?: () => void;
  onClose: () => void;
}

function scopeBadge(scope?: string) {
  if (scope === "global") {
    return { label: "global", Icon: GlobeIcon, cls: "bg-accent/10 text-accent-soft" };
  }
  return { label: "workspace", Icon: FolderIcon, cls: "bg-ink-800 text-ink-400" };
}

function MemoryCard({
  m,
  onForget,
}: {
  m: MemoryEntry;
  onForget: (id: string) => void | Promise<void>;
}) {
  const [expanded, setExpanded] = useState(false);
  const title = m.name || m.text;
  const content = m.content ?? m.text ?? "";
  const desc = m.description;
  const { label, Icon, cls } = scopeBadge(m.scope);
  const hasLongContent = content.length > 160;

  return (
    <div className="group rounded-lg border border-ink-800 bg-ink-925/40 px-3 py-2.5">
      {/* Title row */}
      <div className="flex items-start gap-2">
        <div className="min-w-0 flex-1">
          <div className="flex items-center gap-1.5">
            <span className="truncate text-[13px] font-semibold text-ink-100">{title}</span>
            {m.type && (
              <span className="rounded bg-ink-800 px-1.5 py-0.5 text-[9px] uppercase tracking-wide text-ink-400">
                {m.type}
              </span>
            )}
          </div>
          {desc && (
            <p className="mt-0.5 truncate text-[11px] text-ink-500">{desc}</p>
          )}
        </div>
        <button
          onClick={() => void onForget(m.id)}
          className="shrink-0 rounded-md p-1 text-ink-600 opacity-100 transition-opacity hover:bg-danger/10 hover:text-danger sm:opacity-0 sm:group-hover:opacity-100"
          title="Forget"
          aria-label={`Forget ${title}`}
        >
          <TrashIcon width={13} height={13} />
        </button>
      </div>

      {/* Content */}
      {content && (
        <div className="mt-1.5">
          <pre
            className={`whitespace-pre-wrap break-words rounded-md border border-ink-800/60 bg-ink-950/50 p-2 font-mono text-[11.5px] leading-relaxed text-ink-300 ${
              expanded || !hasLongContent ? "" : "max-h-20 overflow-hidden"
            }`}
          >
            <code>{content}</code>
          </pre>
          {hasLongContent && (
            <button
              onClick={() => setExpanded((e) => !e)}
              className="mt-1 flex items-center gap-1 text-[11px] text-accent-soft hover:text-accent"
            >
              <ChevronDown
                width={12}
                height={12}
                className={`transition-transform ${expanded ? "rotate-180" : ""}`}
              />
              {expanded ? "Show less" : "Show more"}
            </button>
          )}
        </div>
      )}

      {/* Footer: scope + tags */}
      <div className="mt-1.5 flex flex-wrap items-center gap-1">
        <span
          className={`inline-flex items-center gap-1 rounded px-1.5 py-0.5 text-[9px] font-medium ${cls}`}
        >
          <Icon width={9} height={9} />
          {label}
        </span>
        {m.tags &&
          m.tags
            .filter((t) => t !== m.type)
            .map((t) => (
              <span key={t} className="rounded bg-ink-800 px-1.5 py-0.5 text-[9px] text-ink-400">
                {t}
              </span>
            ))}
      </div>
    </div>
  );
}

export function MemoryPanel({ memories, onSave, onForget, onRefresh, onClose }: Props) {
  const [text, setText] = useState("");
  const [tags, setTags] = useState("");
  const [scope, setScope] = useState<"workspace" | "global">("workspace");
  const { confirm, dialog } = useAppDialog();
  const closeRef = useOutsideClose(onClose);
  const trapRef = useFocusTrap<HTMLDivElement>();

  const save = () => {
    const t = text.trim();
    if (!t) return;
    const tagList = tags
      .split(",")
      .map((s) => s.trim())
      .filter(Boolean);
    onSave(t, tagList.length ? tagList : undefined, scope);
    setText("");
    setTags("");
  };

  const forget = async (id: string) => {
    const entry = memories.find((m) => m.id === id);
    const title = entry?.name || entry?.text || id;
    const ok = await confirm({
      title: "Forget memory",
      message: `Forget "${title}"? This removes it from future sessions.`,
      confirmLabel: "Forget",
      danger: true,
    });
    if (ok) onForget(id);
  };

  return (
    <>
      <AppDialogHost dialog={dialog} />
      <div className="modal-backdrop">
        <div
          ref={mergeRefs(closeRef, trapRef)}
          className="modal-sheet max-w-xl"
          role="dialog"
          aria-modal="true"
          aria-label="Memories"
        >
          <div className="flex items-center justify-between border-b border-ink-800/80 px-4 py-3">
            <div className="flex items-center gap-2">
              <BrainIcon width={15} height={15} className="text-accent-soft" />
              <span className="text-[13px] font-semibold text-ink-100">Memories</span>
              {memories.length > 0 && (
                <span className="rounded-full bg-ink-800 px-2 py-0.5 text-[10px] text-ink-400">
                  {memories.length}
                </span>
              )}
            </div>
            <div className="flex items-center gap-1">
              {onRefresh && (
                <button
                  onClick={onRefresh}
                  className="rounded-md p-1 text-ink-500 hover:bg-ink-800 hover:text-ink-100"
                  title="Re-inject memories into the system prompt"
                  aria-label="Refresh memory injection"
                >
                  <RefreshIcon width={15} height={15} />
                </button>
              )}
              <button
                onClick={onClose}
                className="rounded-md p-1 text-ink-500 hover:bg-ink-800 hover:text-ink-100"
              >
                <XIcon width={16} height={16} />
              </button>
            </div>
          </div>

          <div className="flex min-h-0 flex-1 flex-col overflow-y-auto p-4">
            {/* New memory */}
            <div className="mb-3 rounded-xl border border-ink-800 bg-ink-925/40 p-3">
              <textarea
                rows={3}
                value={text}
                onChange={(e) => setText(e.target.value)}
                placeholder="Save a note the agent will remember across sessions…"
                className="w-full resize-none rounded-lg border border-ink-700 bg-ink-950 px-3 py-2 text-[13px] text-ink-100 placeholder:text-ink-600 focus:border-accent/50 focus:outline-none"
              />
              <div className="mt-2 flex flex-wrap items-center gap-2">
                <div className="flex rounded-lg border border-ink-700 bg-ink-950 p-0.5">
                  {(["workspace", "global"] as const).map((s) => (
                    <button
                      key={s}
                      type="button"
                      onClick={() => setScope(s)}
                      className={`rounded-md px-2.5 py-1 text-[11px] font-medium capitalize transition-colors ${
                        scope === s
                          ? "bg-ink-800 text-ink-100"
                          : "text-ink-500 hover:text-ink-300"
                      }`}
                    >
                      {s}
                    </button>
                  ))}
                </div>
                <input
                  value={tags}
                  onChange={(e) => setTags(e.target.value)}
                  placeholder="tags, comma-separated"
                  className="min-w-[8rem] flex-1 rounded-lg border border-ink-700 bg-ink-950 px-3 py-1.5 font-mono text-[12px] text-ink-200 placeholder:text-ink-600 focus:border-accent/50 focus:outline-none"
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
              <div className="px-3 py-8 text-center text-[12px] text-ink-600">
                No memories yet. Save one above.
              </div>
            ) : (
              <div className="space-y-2">
                {memories.map((m) => (
                  <MemoryCard key={m.id} m={m} onForget={forget} />
                ))}
              </div>
            )}
          </div>
        </div>
      </div>
    </>
  );
}
