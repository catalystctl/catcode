"use client";

// Approval — the human-in-the-loop gate banner. Destructive tools are styled
// amber/rose; read-only tools are neutral. Yes / No / Always (escalate this
// tool kind for the rest of the session). Esc denies (TUI parity); outside
// click does NOT deny — the user may be reading a diff / using the sidebar.

import { useCallback, useEffect, useMemo } from "react";
import type { ApprovalRequest, ApproveDecision } from "@/lib/types";
import { isDangerousTool, toolIcon } from "@/lib/format";
import { useFocusTrap } from "@/lib/use-focus-trap";
import { CheckIcon, XIcon, ShieldIcon } from "./icons";
import { Diff } from "./diff";

interface Props {
  approval: ApprovalRequest;
  onApprove: (decision: ApproveDecision, opts?: { pattern?: string }) => void;
}

export function Approval({ approval, onApprove }: Props) {
  const danger = isDangerousTool(approval.tool);
  const args = useMemo(() => {
    try {
      return JSON.stringify(JSON.parse(approval.args), null, 2);
    } catch {
      return approval.args;
    }
  }, [approval.args]);

  const onDeny = useCallback(() => onApprove("no"), [onApprove]);
  const trapRef = useFocusTrap<HTMLDivElement>();

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key !== "Escape") return;
      e.preventDefault();
      e.stopImmediatePropagation();
      onDeny();
    };
    window.addEventListener("keydown", onKey, true);
    return () => window.removeEventListener("keydown", onKey, true);
  }, [onDeny]);

  return (
    <div
      ref={trapRef}
      role="alertdialog"
      aria-modal="true"
      aria-label={`Approve ${approval.tool}`}
      className={`chat-msg-enter my-3 overflow-hidden rounded-sm border border-ink-700 border-l-2 bg-ink-925 ${
        danger ? "border-l-warning" : "border-l-accent"
      }`}
    >
      <div className="flex items-center gap-2.5 border-b border-ink-800 px-4 py-2.5">
        <span
          className={`flex h-7 w-7 shrink-0 items-center justify-center rounded-sm border border-ink-800 bg-ink-900 ${
            danger ? "text-warning" : "text-accent-soft"
          }`}
        >
          <ShieldIcon width={14} height={14} />
        </span>
        <div className="min-w-0 flex-1">
          <div className="text-[10px] font-mono uppercase tracking-wider text-ink-400">
            Approval required
          </div>
          <div className="text-[12px] text-ink-200">
            {danger ? "This action may modify your system." : "Review before continuing."}
          </div>
        </div>
        <span className="flex min-w-0 items-center gap-1.5 rounded-sm border border-ink-800 bg-ink-950 px-1.5 py-0.5">
          <span className="text-xs leading-none">{toolIcon(approval.tool)}</span>
          <span className="truncate font-mono text-[11px] text-ink-200">{approval.tool}</span>
        </span>
      </div>
      <div className="px-3 py-3 sm:px-4">
        {args && (
          <pre className="mb-3 max-h-56 overflow-auto rounded-sm border border-ink-800 bg-ink-950 p-2.5 font-mono text-[11px] leading-relaxed text-ink-300">
            <code>{args}</code>
          </pre>
        )}
        {approval.diff && <Diff diff={approval.diff} className="mb-3" />}
        {/* 2×2 dense grid keeps HITL actions readable in ~320px docks */}
        <div className="approval-actions grid grid-cols-2 gap-1.5">
          <button
            onClick={() => onApprove("yes")}
            title="Approve once"
            className="flex items-center justify-center gap-1.5 rounded-sm bg-accent px-2.5 py-1 text-[11px] font-medium text-white transition-colors hover:bg-accent-soft"
          >
            <CheckIcon width={13} height={13} className="shrink-0" />
            <span className="truncate">Once</span>
          </button>
          <button
            onClick={() => onApprove("always")}
            title={`Always allow ${approval.tool}`}
            className="flex items-center justify-center gap-1.5 rounded-sm border border-ink-700 px-2.5 py-1 text-[11px] text-ink-300 transition-colors hover:bg-ink-800"
          >
            <ShieldIcon width={12} height={12} className="shrink-0" />
            <span className="truncate">Always</span>
          </button>
          <button
            onClick={() => onApprove("allow_session")}
            title="Allow for session"
            className="flex items-center justify-center gap-1.5 rounded-sm border border-ink-700 px-2.5 py-1 text-[11px] text-ink-300 transition-colors hover:bg-ink-800"
          >
            <span className="truncate">Session</span>
          </button>
          <button
            onClick={onDeny}
            title="Deny (Esc)"
            className="flex items-center justify-center gap-1.5 rounded-sm bg-danger/90 px-2.5 py-1 text-[11px] font-medium text-white transition-colors hover:bg-danger"
          >
            <XIcon width={13} height={13} className="shrink-0" />
            <span className="truncate">Deny</span>
          </button>
        </div>
      </div>
    </div>
  );
}
