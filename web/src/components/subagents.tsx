"use client";

// SubagentsPanel — the live subagent supervisor surface.
//
// Replaces the old flat intercom log with two views:
//  (1) a runs list: every subagent run (single / parallel-batch / chain), each
//      card showing agent, task, state, elapsed, tokens, current phase/tool —
//      with a pulsing indicator while running.
//  (2) a drill-in live chat: click a run to see what it's doing — its task,
//      each assistant turn's text, and every tool call (name + args) with its
//      result. This is the per-run equivalent of the main chat, assembled from
//      the core's run_id-tagged subagent_message / subagent_tool_call /
//      subagent_tool_result events.
//
// The blocking `need_decision` ask still uses the inline IntercomPrompt banner
// (intercom.tsx); this panel is for observation, not reply.

import { useEffect, useState } from "react";
import type { AgentInfo, SubagentChatItem, SubagentRunView } from "@/lib/types";
import {
  formatMs,
  formatTokens,
  relativeTime,
  truncate,
  prettyArgs,
  toolIcon,
  isDangerousTool,
} from "@/lib/format";
import {
  XIcon,
  ChevronRight,
  DotIcon,
  CheckIcon,
  WarningIcon,
  RefreshIcon,
} from "./icons";
import { useOutsideClose, mergeRefs } from "@/lib/use-outside-close";
import { useFocusTrap } from "@/lib/use-focus-trap";

/** Fallback builtins if the core hasn't emitted an `agents` event yet. */
const FALLBACK_AGENTS: AgentInfo[] = [
  { name: "scout", description: "Fast code exploration", source: "builtin" },
  { name: "reviewer", description: "Evidence-backed review", source: "builtin" },
  { name: "worker", description: "Implementation writer", source: "builtin" },
  { name: "oracle", description: "Decision consistency", source: "builtin" },
  { name: "planner", description: "Implementation planning", source: "builtin" },
  { name: "researcher", description: "Deep research", source: "builtin" },
  { name: "context-builder", description: "Gather handoff context", source: "builtin" },
  { name: "delegate", description: "General delegated work", source: "builtin" },
];

/** @deprecated Prefer live `availableAgents` from core. Kept for external imports. */
export const AVAILABLE_AGENTS = FALLBACK_AGENTS.map((a) => a.name);

const STATE_STYLE: Record<string, { dot: string; text: string; label: string }> = {
  running: { dot: "bg-warning", text: "text-warning", label: "running" },
  completed: { dot: "bg-success", text: "text-success", label: "done" },
  failed: { dot: "bg-danger", text: "text-danger", label: "failed" },
  paused: { dot: "bg-ink-500", text: "text-ink-400", label: "paused" },
};

function StateBadge({ state }: { state: string }) {
  const s = STATE_STYLE[state] ?? STATE_STYLE.paused;
  const pulse = state === "running";
  return (
    <span className={`inline-flex items-center gap-1.5 ${s.text}`}>
      <span className={`relative flex h-2 w-2`}>
        {pulse && (
          <span
            className={`absolute inline-flex h-full w-full animate-ping rounded-full ${s.dot} opacity-60`}
          />
        )}
        <span className={`relative inline-flex h-2 w-2 rounded-full ${s.dot}`} />
      </span>
      <span className="text-[11px] font-medium">{s.label}</span>
    </span>
  );
}

function RunCard({ run, onClick }: { run: SubagentRunView; onClick: () => void }) {
  const isContainer = run.mode === "parallel" || run.mode === "chain";
  const title = run.agent ?? (isContainer ? run.mode : "subagent");
  const phaseHint =
    run.state === "running" && run.phase
      ? run.tool
        ? `${run.phase} · ${run.tool}`
        : run.phase
      : run.summary
        ? truncate(run.summary, 90)
        : run.state === "running"
          ? "working…"
          : "finished";
  return (
    <button
      onClick={onClick}
      className="group w-full rounded-xl border border-ink-800 bg-ink-900/40 px-3.5 py-3 text-left transition-all hover:border-accent/40 hover:bg-ink-850"
    >
      <div className="flex items-center gap-2">
        <span className="font-mono text-[12px] font-semibold text-accent-soft">
          {isContainer ? "❯" : "↳"} {title}
        </span>
        {run.mode !== "single" && (
          <span className="rounded bg-ink-850 px-1.5 py-0.5 text-[10px] uppercase tracking-wide text-ink-400">
            {run.mode}
          </span>
        )}
        <span className="ml-auto">
          <StateBadge state={run.state} />
        </span>
      </div>
      <div className="mt-1 line-clamp-2 text-[12px] leading-relaxed text-ink-300">
        {run.task || (isContainer ? `${run.agents.length} agent(s)` : "—")}
      </div>
      <div className="mt-2 flex flex-wrap items-center gap-x-3 gap-y-1 text-[11px] text-ink-500">
        <span>{formatMs(run.elapsedMs)}</span>
        <span className="text-ink-700">·</span>
        <span>↑{formatTokens(run.tokensOut)}</span>
        <span className="text-ink-700">·</span>
        <span>{run.toolCount} tool{run.toolCount === 1 ? "" : "s"}</span>
        <span className="ml-auto truncate text-ink-600">{phaseHint}</span>
      </div>
    </button>
  );
}

function ChatMessage({ item }: { item: SubagentChatItem }) {
  const isUser = item.role === "user";
  return (
    <div className={isUser ? "" : "mt-3"}>
      <div className="mb-1 flex items-center gap-1.5">
        <span
          className={`text-[11px] font-semibold uppercase tracking-wide ${
            isUser ? "text-accent-soft" : "text-ink-400"
          }`}
        >
          {isUser ? "Task" : "Assistant"}
        </span>
      </div>
      <pre className="whitespace-pre-wrap break-words rounded-lg border border-ink-800 bg-ink-950/60 p-2.5 text-[12.5px] leading-relaxed text-ink-200">
        <code>{item.content}</code>
      </pre>
    </div>
  );
}

function ToolBlock({ item }: { item: SubagentChatItem }) {
  const [open, setOpen] = useState(false);
  const name = item.name ?? "";
  const dangerous = isDangerousTool(name);
  const pending = item.result === undefined;
  const argStr = item.args ? prettyArgs(item.args) : "";
  const showArgs = argStr && argStr !== "{}";
  const out = item.result ?? "";

  return (
    <div className="mt-3 rounded-lg border border-ink-800 bg-ink-950/40">
      <button
        onClick={() => showArgs && setOpen((o) => !o)}
        className={`flex w-full items-center gap-2 px-3 py-2 text-left ${
          showArgs ? "hover:bg-ink-850/60" : "cursor-default"
        }`}
      >
        <span className="text-[13px]">{toolIcon(name)}</span>
        <span className="font-mono text-[12px] font-medium text-ink-200">{name || "tool"}</span>
        {dangerous && (
          <span className="rounded bg-danger/10 px-1.5 py-0.5 text-[10px] font-medium text-danger">
            destructive
          </span>
        )}
        <span className="ml-auto flex items-center gap-2">
          {pending ? (
            <span className="flex items-center gap-1 text-[11px] text-warning">
              <DotIcon className="text-warning" /> running
            </span>
          ) : item.ok ? (
            <span className="flex items-center gap-1 text-[11px] text-success">
              <CheckIcon width={12} height={12} /> ok
            </span>
          ) : (
            <span className="flex items-center gap-1 text-[11px] text-danger">
              <WarningIcon width={12} height={12} /> error
            </span>
          )}
          {showArgs && (
            <ChevronRight
              width={14}
              height={14}
              className={`text-ink-500 transition-transform ${open ? "rotate-90" : ""}`}
            />
          )}
        </span>
      </button>
      {showArgs && open && (
        <pre className="mx-3 mb-2 max-h-48 overflow-auto whitespace-pre-wrap break-words rounded border border-ink-800 bg-ink-950 p-2 text-[11.5px] leading-relaxed text-ink-300">
          <code>{argStr}</code>
        </pre>
      )}
      {out && (
        <pre className="mx-3 mb-3 mt-1 max-h-64 overflow-auto whitespace-pre-wrap break-words rounded border border-ink-800 bg-ink-950 p-2 text-[11.5px] leading-relaxed text-ink-400">
          <code>{out}</code>
        </pre>
      )}
    </div>
  );
}

function RunDetail({ run, onBack }: { run: SubagentRunView; onBack: () => void }) {
  const title = run.agent ?? run.mode;
  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <div className="flex items-center gap-2 border-b border-ink-800/80 px-4 py-3">
        <button
          onClick={onBack}
          className="flex items-center gap-1 rounded-md px-1.5 py-0.5 text-[13px] text-ink-300 transition-colors hover:bg-ink-800 hover:text-ink-100"
        >
          <ChevronRight width={14} height={14} className="rotate-180" />
          Back
        </button>
        <span className="font-mono text-[13px] font-semibold text-accent-soft">↳ {title}</span>
        <span className="ml-auto">
          <StateBadge state={run.state} />
        </span>
      </div>
      <div className="border-b border-ink-800/60 px-4 py-2.5">
        <div className="flex flex-wrap items-center gap-x-3 gap-y-1 text-[11px] text-ink-500">
          <span>{formatMs(run.elapsedMs)}</span>
          <span className="text-ink-700">·</span>
          <span>↑{formatTokens(run.tokensOut)} ↓{formatTokens(run.tokensIn)}</span>
          <span className="text-ink-700">·</span>
          <span>{run.toolCount} tool{run.toolCount === 1 ? "" : "s"}</span>
          <span className="text-ink-700">·</span>
          <span>{relativeTime(run.startedAt)}</span>
        </div>
        {run.task && (
          <p className="mt-1.5 line-clamp-2 text-[12px] leading-relaxed text-ink-300">{run.task}</p>
        )}
      </div>
      <div className="min-h-0 flex-1 overflow-y-auto px-4 py-3">
        {run.items.length === 0 ? (
          <div className="px-3 py-10 text-center text-[12px] text-ink-600">
            {run.mode === "parallel" || run.mode === "chain" ? (
              <>
                This is a container run ({run.mode}). Its child runs appear in the list —
                each has its own live chat.
              </>
            ) : run.state === "running" ? (
              "Waiting for the subagent's first response…"
            ) : (
              "No transcript captured for this run."
            )}
          </div>
        ) : (
          <div className="space-y-0">
            {run.items.map((item) =>
              item.kind === "message" ? (
                <ChatMessage key={item.id} item={item} />
              ) : (
                <ToolBlock key={item.id} item={item} />
              ),
            )}
            {run.state === "running" && (
              <div className="mt-3 flex items-center gap-1.5 text-[11px] text-warning">
                <DotIcon className="text-warning" /> working…
              </div>
            )}
          </div>
        )}
      </div>
    </div>
  );
}

interface PanelProps {
  runs: Record<string, SubagentRunView>;
  /** Discoverable agents from core `agents` events (builtin + user + project). */
  agents?: AgentInfo[];
  onRefreshAgents?: () => void;
  onClose: () => void;
}

export function SubagentsPanel({ runs, agents, onRefreshAgents, onClose }: PanelProps) {
  const closeRef = useOutsideClose(onClose);
  const trapRef = useFocusTrap<HTMLDivElement>();
  const [selectedId, setSelectedId] = useState<string | null>(null);

  useEffect(() => {
    onRefreshAgents?.();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const list = Object.values(runs).sort((a, b) => {
    const ar = a.state === "running" ? 1 : 0;
    const br = b.state === "running" ? 1 : 0;
    if (ar !== br) return br - ar;
    return (b.startedAt || 0) - (a.startedAt || 0);
  });
  const runningCount = list.filter((r) => r.state === "running").length;
  const selected = selectedId ? runs[selectedId] : null;
  const agentList = agents && agents.length > 0 ? agents : FALLBACK_AGENTS;

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/60 p-4 backdrop-blur-sm">
      <div
        ref={mergeRefs(closeRef, trapRef)}
        className="flex max-h-[85vh] w-full max-w-2xl flex-col rounded-2xl border border-ink-700 bg-ink-900 shadow-2xl animate-fade-in"
        role="dialog"
        aria-modal="true"
        aria-label="Subagents"
      >
        <div className="flex items-center justify-between border-b border-ink-800/80 px-4 py-3">
          <div className="flex items-center gap-2">
            <span className="text-[15px] font-semibold text-ink-100">Subagents</span>
            {runningCount > 0 && (
              <span className="flex items-center gap-1.5 rounded-full bg-warning/10 px-2 py-0.5 text-[11px] font-medium text-warning">
                <DotIcon className="text-warning" /> {runningCount} running
              </span>
            )}
          </div>
          <div className="flex items-center gap-1">
            {onRefreshAgents && (
              <button
                onClick={onRefreshAgents}
                className="rounded-md p-1 text-ink-500 hover:bg-ink-800 hover:text-ink-100"
                aria-label="Refresh agents"
                title="Refresh available agents"
              >
                <RefreshIcon width={15} height={15} />
              </button>
            )}
            <button
              onClick={onClose}
              className="rounded-md p-1 text-ink-500 hover:bg-ink-800 hover:text-ink-100"
              aria-label="Close"
            >
              <XIcon width={16} height={16} />
            </button>
          </div>
        </div>

        {selected ? (
          <RunDetail run={selected} onBack={() => setSelectedId(null)} />
        ) : (
          <div className="flex-1 overflow-y-auto p-3">
            <div className="mb-3 rounded-xl border border-ink-800 bg-ink-950/50 px-3 py-2.5">
              <div className="mb-1.5 text-[11px] font-medium uppercase tracking-wide text-ink-500">
                Available agents
              </div>
              <div className="flex flex-wrap gap-1.5">
                {agentList.map((a) => (
                  <code
                    key={a.name}
                    title={a.description || a.source}
                    className="rounded-md bg-ink-850 px-1.5 py-0.5 font-mono text-[11px] text-accent-soft"
                  >
                    {a.name}
                  </code>
                ))}
              </div>

              <p className="mt-2 text-[11px] text-ink-600">
                Use <code className="font-mono text-ink-500">/run</code>,{" "}
                <code className="font-mono text-ink-500">/parallel</code>, or{" "}
                <code className="font-mono text-ink-500">/chain</code> — or the{" "}
                <code className="font-mono text-ink-500">subagent</code> tool.
              </p>
            </div>
            {list.length === 0 ? (
              <div className="px-3 py-8 text-center text-[12px] text-ink-600">
                No runs yet. Delegated work will appear here live.
              </div>
            ) : (
              <div className="space-y-2">
                {list.map((r) => (
                  <RunCard key={r.id} run={r} onClick={() => setSelectedId(r.id)} />
                ))}
              </div>
            )}
          </div>
        )}
      </div>
    </div>
  );
}
