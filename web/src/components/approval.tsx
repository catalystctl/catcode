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
      className={`my-3 overflow-hidden rounded-xl border ${
        danger ? "border-warning/30 bg-warning/[0.04]" : "border-accent/20 bg-accent/[0.03]"
      }`}
    >
      <div
        className={`flex items-center gap-2 border-b px-4 py-2.5 ${
          danger ? "border-warning/20" : "border-accent/15"
        }`}
      >
        <ShieldIcon width={15} height={15} className={danger ? "text-warning" : "text-accent-soft"} />
        <span className="text-sm font-semibold text-ink-100">Approval required</span>
        <span className="ml-auto flex items-center gap-1.5 rounded-full bg-ink-850 px-2 py-0.5">
          <span className="text-xs">{toolIcon(approval.tool)}</span>
          <span className="font-mono text-[12px] font-medium text-ink-200">{approval.tool}</span>
        </span>
      </div>
      <div className="px-4 py-3">
        {args && (
          <pre className="mb-3 max-h-56 overflow-auto rounded-lg border border-ink-800 bg-ink-950 p-2.5 text-[12px] leading-relaxed text-ink-200">
            <code>{args}</code>
          </pre>
        )}
        {approval.diff && <Diff diff={approval.diff} className="mb-3" />}
        <div className="flex flex-wrap items-center gap-2">
          <button
            onClick={() => onApprove("yes")}
            className="flex items-center gap-1.5 rounded-lg bg-success/90 px-3.5 py-1.5 text-[13px] font-medium text-ink-950 transition-colors hover:bg-success"
          >
            <CheckIcon width={14} height={14} /> Approve once
          </button>
          <button
            onClick={() => onApprove("always")}
            className="flex items-center gap-1.5 rounded-lg border border-success/30 bg-success/10 px-3.5 py-1.5 text-[13px] font-medium text-success transition-colors hover:bg-success/20"
          >
            <ShieldIcon width={13} height={13} /> Always allow {approval.tool}
          </button>
          <button
            onClick={() => onApprove("allow_session")}
            className="flex items-center gap-1.5 rounded-lg border border-accent/30 bg-accent/10 px-3.5 py-1.5 text-[13px] font-medium text-accent-soft transition-colors hover:bg-accent/20"
          >
            Allow for session
          </button>
          <button
            onClick={onDeny}
            className="flex items-center gap-1.5 rounded-lg border border-ink-700 px-3.5 py-1.5 text-[13px] font-medium text-ink-300 transition-colors hover:border-danger/40 hover:bg-danger/10 hover:text-danger"
          >
            <XIcon width={14} height={14} /> Deny
          </button>
        </div>
      </div>
    </div>
  );
}
