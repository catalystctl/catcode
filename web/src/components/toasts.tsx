"use client";

// Toasts — a small stack of transient notifications (compaction notices,
// retries, errors). Auto-dismiss after a few seconds. The dismiss timer keys on
// the toast id (NOT the onDismiss callback) so it survives parent re-renders
// during streaming.

import { useEffect } from "react";
import type { Toast } from "@/lib/types";
import { XIcon } from "./icons";

const TTL = 5000;

export function Toasts({
  toasts,
  onDismiss,
  docked,
}: {
  toasts: Toast[];
  onDismiss: (id: string) => void;
  /** When true, anchor inside `.chat-panel` instead of the viewport. */
  docked?: boolean;
}) {
  return (
    <div
      role="status"
      aria-live="polite"
      className={
        docked
          ? "chat-toasts pointer-events-none absolute bottom-[4.75rem] left-2 right-2 z-40 flex max-w-sm flex-col gap-2 sm:left-auto sm:right-2 sm:w-72"
          : "chat-toasts pointer-events-none fixed bottom-[calc(6.5rem+env(safe-area-inset-bottom))] left-3 right-3 z-40 flex max-w-sm flex-col gap-2 sm:bottom-24 sm:left-auto sm:right-4 sm:w-80"
      }
    >
      {toasts.map((t) => (
        <ToastItem key={t.id} toast={t} onDismiss={onDismiss} />
      ))}
    </div>
  );
}

function ToastItem({ toast, onDismiss }: { toast: Toast; onDismiss: (id: string) => void }) {
  useEffect(() => {
    const t = setTimeout(() => onDismiss(toast.id), TTL);
    return () => clearTimeout(t);
  }, [toast.id, onDismiss]);

  const cfg =
    toast.kind === "error"
      ? { bar: "border-l-danger", dot: "bg-danger" }
      : toast.kind === "success"
        ? { bar: "border-l-success", dot: "bg-success" }
        : { bar: "border-l-accent", dot: "bg-accent" };

  return (
    <div
      className={`pointer-events-auto flex items-center gap-2 rounded-sm border border-ink-700 border-l-2 ${cfg.bar} bg-ink-900 py-1.5 pl-2 pr-1.5 shadow-elev-2 animate-fade-in`}
    >
      <span className={`h-1.5 w-1.5 shrink-0 rounded-none ${cfg.dot}`} aria-hidden />
      <p className="flex-1 break-words text-[11px] leading-snug text-ink-200 sm:break-normal">
        {toast.message}
      </p>
      <button
        onClick={() => onDismiss(toast.id)}
        aria-label="Dismiss notification"
        className="shrink-0 rounded-sm p-1 text-ink-500 transition-colors hover:bg-ink-800 hover:text-ink-100"
      >
        <XIcon width={12} height={12} />
      </button>
    </div>
  );
}
