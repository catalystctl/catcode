"use client";

// ToolCall — a collapsible card for a single tool invocation: name + args
// (syntax-highlighted JSON) + the execution result (mono, scrollable, with an
// optional diff pane). Shows a spinner while awaiting the result.

import { useState } from "react";
import type { UIToolCall } from "@/lib/types";
import { isDangerousTool, prettyArgs, toolIcon, truncate } from "@/lib/format";
import { ChevronRight, CheckIcon, XIcon, CopyIcon } from "./icons";
import { Diff } from "./diff";

export function ToolCallCard({ tc }: { tc: UIToolCall }) {
  const [open, setOpen] = useState(false);
  const [copied, setCopied] = useState(false);
  const danger = isDangerousTool(tc.name);
  const running = !tc.result;
  const ok = tc.result?.ok;
  const unknown = tc.result?.unknown;

  const copy = () => {
    navigator.clipboard?.writeText(tc.result?.output ?? "").then(
      () => {
        setCopied(true);
        setTimeout(() => setCopied(false), 1400);
      },
      () => {},
    );
  };

  return (
    <div className="my-1.5 overflow-hidden rounded-lg border border-ink-800 bg-ink-925/40">
      <button
        onClick={() => setOpen((o) => !o)}
        className="flex w-full items-center gap-2 px-3 py-2 text-left transition-colors hover:bg-ink-850/60"
      >
        <ChevronRight
          width={14}
          height={14}
          className={`shrink-0 text-ink-500 transition-transform ${open ? "rotate-90" : ""}`}
        />
        <span className="shrink-0 text-sm">{toolIcon(tc.name)}</span>
        <span className="font-mono text-[13px] font-medium text-ink-100">{tc.name || "tool"}</span>
        {danger && (
          <span className="rounded bg-warning/10 px-1.5 py-0.5 text-[10px] font-medium uppercase tracking-wide text-warning">
            destructive
          </span>
        )}
        <span className="ml-1 truncate font-mono text-[11px] text-ink-500">
          {truncate(tc.argString || JSON.stringify(tc.args), 64)}
        </span>
        <span className="ml-auto flex items-center gap-1.5">
          {running ? (
            <span className="flex items-center gap-1 text-[11px] text-accent-soft">
              <span className="h-1.5 w-1.5 animate-pulse rounded-full bg-accent-soft" />
              running
            </span>
          ) : unknown ? (
            <span className="flex items-center gap-1 text-[11px] text-ink-500">
              <span className="h-1.5 w-1.5 rounded-full bg-ink-600" />
              loaded
            </span>
          ) : ok ? (
            <span className="flex items-center gap-1 text-[11px] text-success">
              <CheckIcon width={12} height={12} /> ok
            </span>
          ) : (
            <span className="flex items-center gap-1 text-[11px] text-danger">
              <XIcon width={12} height={12} /> error
            </span>
          )}
        </span>
      </button>

      {open && (
        <div className="border-t border-ink-800/70 bg-ink-950">
          {tc.argString && (
            <div className="px-3 py-2">
              <div className="mb-1 font-mono text-[10px] uppercase tracking-wider text-ink-500">arguments</div>
              <pre className="overflow-x-auto text-[12px] leading-relaxed text-ink-200">
                <code>{prettyArgs(tc.args)}</code>
              </pre>
            </div>
          )}
          {tc.result && (
            <div className="border-t border-ink-800/70 px-3 py-2">
              <div className="mb-1 flex items-center justify-between">
                <span className="font-mono text-[10px] uppercase tracking-wider text-ink-500">result</span>
                <button
                  onClick={copy}
                  className="flex items-center gap-1 rounded px-1.5 py-0.5 text-[10px] text-ink-500 hover:bg-ink-800 hover:text-ink-100"
                >
                  {copied ? <CheckIcon width={11} height={11} /> : <CopyIcon width={11} height={11} />}
                  {copied ? "Copied" : "Copy"}
                </button>
              </div>
              {tc.result.diff && <Diff diff={tc.result.diff} />}
              <pre
                className={`mt-1 max-h-80 overflow-auto whitespace-pre-wrap break-words text-[12px] leading-relaxed ${
                  ok || unknown ? "text-ink-200" : "text-danger"
                }`}
              >
                {tc.result.output || "(no output)"}
              </pre>
            </div>
          )}
        </div>
      )}
    </div>
  );
}
