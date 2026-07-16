"use client";

// HelpModal — keybindings + slash-command reference. Opened via /help or the
// sidebar Help action. Uses the shared command catalog so it stays in sync with
// the flyout.

import { COMMANDS } from "@/lib/commands";
import { useOutsideClose, mergeRefs } from "@/lib/use-outside-close";
import { useBodyScrollLock } from "@/lib/use-body-scroll-lock";
import { useFocusTrap } from "@/lib/use-focus-trap";
import { XIcon } from "./icons";

const KEYBINDS: Array<[string, string]> = [
  ["Enter", "Send (or queue follow-up while streaming)"],
  ["Ctrl + Enter", "Steer the in-flight turn"],
  ["Shift + Enter", "Newline"],
  ["Esc", "Clear queued follow-up, else stop turn / close flyout"],
  ["/", "Open command menu"],
  ["!", "Run bash (!cmd · !!cmd no context)"],
  ["@", "Mention a file"],
  ["↑ / ↓", "Navigate flyout"],
  ["Tab / ↵", "Confirm flyout selection"],
  ["Double-click session", "Rename session"],
];

export function HelpModal({ onClose }: { onClose: () => void }) {
  const closeRef = useOutsideClose(onClose);
  const trapRef = useFocusTrap<HTMLDivElement>();
  useBodyScrollLock();
  return (
    <div className="modal-backdrop">
      <div
        ref={mergeRefs(closeRef, trapRef)}
        className="modal-sheet max-w-lg"
        role="dialog"
        aria-modal="true"
        aria-labelledby="help-modal-title"
      >
        <div className="flex items-center justify-between border-b border-ink-800/80 px-5 py-3.5">
          <h2 id="help-modal-title" className="text-[15px] font-semibold text-ink-100">Help & Keybindings</h2>
          <button
            onClick={onClose}
            className="rounded-md p-1 text-ink-500 hover:bg-ink-800 hover:text-ink-100"
            aria-label="Close"
          >
            <XIcon width={16} height={16} />
          </button>
        </div>
        <div className="overflow-y-auto px-5 py-4">
          {/* Keybindings */}
          <div className="mb-5">
            <div className="mb-2 text-[11px] font-medium uppercase tracking-wider text-ink-500">
              Keyboard
            </div>
            <div className="grid grid-cols-1 gap-1.5 sm:grid-cols-2">
              {KEYBINDS.map(([key, desc]) => (
                <div key={key} className="flex items-center gap-2 text-[12px]">
                  <kbd className="shrink-0 rounded bg-ink-800 px-1.5 py-0.5 font-mono text-[10px] text-ink-300">
                    {key}
                  </kbd>
                  <span className="text-ink-400">{desc}</span>
                </div>
              ))}
            </div>
          </div>
          {/* Commands */}
          <div>
            <div className="mb-2 text-[11px] font-medium uppercase tracking-wider text-ink-500">
              Slash Commands
            </div>
            <div className="space-y-0.5">
              {COMMANDS.map((c) => (
                <div key={c.label} className="flex items-baseline gap-2 text-[12px]">
                  <span className="shrink-0 font-mono text-accent-soft">{c.label}</span>
                  <span className="text-ink-500">{c.desc}</span>
                </div>
              ))}
            </div>
          </div>
        </div>
      </div>
    </div>
  );
}
