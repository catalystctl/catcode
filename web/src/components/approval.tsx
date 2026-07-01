"use client";

// Approval — the human-in-the-loop gate banner. Destructive tools are styled
// amber/rose; read-only tools are neutral. Yes / No / Always (escalate this
// tool kind for the rest of the session).

import { useMemo } from "react";
import type { ApprovalRequest } from "@/lib/types";
import { isDangerousTool, toolIcon } from "@/lib/format";
import { CheckIcon, XIcon, ShieldIcon } from "./icons";

interface Props {
  approval: ApprovalRequest;
  onApprove: (decision: "yes" | "no" | "always") => void;
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

  return (
    <div
      className={`my-3 overflow-hidden rounded-xl border ${
        danger ? "border-amber-500/30 bg-amber-500/[0.04]" : "border-accent/20 bg-accent/[0.03]"
      }`}
    >
      <div
        className={`flex items-center gap-2 border-b px-4 py-2.5 ${
          danger ? "border-amber-500/20" : "border-accent/15"
        }`}
      >
        <ShieldIcon width={15} height={15} className={danger ? "text-amber-300" : "text-accent-soft"} />
        <span className="text-sm font-semibold text-ink-100">Approval required</span>
        <span className="ml-auto flex items-center gap-1.5 rounded-full bg-ink-850 px-2 py-0.5">
          <span className="text-xs">{toolIcon(approval.tool)}</span>
          <span className="font-mono text-[12px] font-medium text-ink-200">{approval.tool}</span>
        </span>
      </div>
      <div className="px-4 py-3">
        {args && (
          <pre className="mb-3 max-h-56 overflow-auto rounded-lg border border-ink-800 bg-[#08080a] p-2.5 text-[12px] leading-relaxed text-ink-200">
            <code>{args}</code>
          </pre>
        )}
        {approval.diff && (
          <pre className="mb-3 max-h-56 overflow-auto rounded-lg border border-ink-800 bg-[#08080a] p-2.5 text-[12px] leading-relaxed">
            {approval.diff.split("\n").map((l, i) => {
              const cls = l.startsWith("+") && !l.startsWith("+++") ? "diff-line-add" : l.startsWith("-") && !l.startsWith("---") ? "diff-line-del" : l.startsWith("@@") ? "diff-line-hunk" : "";
              return <div key={i} className={`${cls} px-1`}>{l || " "}</div>;
            })}
          </pre>
        )}
        <div className="flex flex-wrap items-center gap-2">
          <button
            onClick={() => onApprove("yes")}
            className="flex items-center gap-1.5 rounded-lg bg-emerald-500/90 px-3.5 py-1.5 text-[13px] font-medium text-emerald-950 transition-colors hover:bg-emerald-400"
          >
            <CheckIcon width={14} height={14} /> Approve once
          </button>
          <button
            onClick={() => onApprove("always")}
            className="flex items-center gap-1.5 rounded-lg border border-emerald-500/30 bg-emerald-500/10 px-3.5 py-1.5 text-[13px] font-medium text-emerald-300 transition-colors hover:bg-emerald-500/20"
          >
            <ShieldIcon width={13} height={13} /> Always allow {approval.tool}
          </button>
          <button
            onClick={() => onApprove("no")}
            className="flex items-center gap-1.5 rounded-lg border border-ink-700 px-3.5 py-1.5 text-[13px] font-medium text-ink-300 transition-colors hover:border-rose-500/40 hover:bg-rose-500/10 hover:text-rose-300"
          >
            <XIcon width={14} height={14} /> Deny
          </button>
        </div>
      </div>
    </div>
  );
}
