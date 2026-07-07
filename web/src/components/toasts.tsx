"use client";

// Toasts — a small stack of transient notifications (compaction notices,
// retries, errors). Auto-dismiss after a few seconds. The dismiss timer keys on
// the toast id (NOT the onDismiss callback) so it survives parent re-renders
// during streaming.

import { useEffect } from "react";
import type { Toast } from "@/lib/types";
import { CheckIcon, XIcon, WarningIcon } from "./icons";

const TTL = 5000;

export function Toasts({ toasts, onDismiss }: { toasts: Toast[]; onDismiss: (id: string) => void }) {
  return (
    <div
      role="status"
      aria-live="polite"
      className="pointer-events-none fixed bottom-24 right-4 z-40 flex w-80 max-w-[calc(100vw-2rem)] flex-col gap-2"
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
      ? { icon: <XIcon width={14} height={14} />, ring: "border-danger/30", tint: "bg-danger/10", text: "text-danger" }
      : toast.kind === "success"
        ? { icon: <CheckIcon width={14} height={14} />, ring: "border-success/30", tint: "bg-success/10", text: "text-success" }
        : { icon: <WarningIcon width={14} height={14} />, ring: "border-accent/20", tint: "bg-accent/[0.06]", text: "text-accent-soft" };

  return (
    <div
      className={`pointer-events-auto flex items-start gap-2 rounded-xl border ${cfg.ring} ${cfg.tint} px-3 py-2.5 backdrop-blur animate-fade-in`}
    >
      <span className={`mt-0.5 shrink-0 ${cfg.text}`}>{cfg.icon}</span>
      <p className="flex-1 text-[12px] leading-snug text-ink-200">{toast.message}</p>
      <button onClick={() => onDismiss(toast.id)} className="shrink-0 text-ink-500 hover:text-ink-100">
        <XIcon width={13} height={13} />
      </button>
    </div>
  );
}
