"use client";

// WorkState — a compact ambient status panel rendered from `work_state` events.
//
// The core keeps a rolling, signal-derived summary of the current task (goal,
// what's done / in progress / next, recently touched files, last activity) and
// emits it so the UI can show live context alongside the conversation. Only
// rendered when there's meaningful content (a goal or active todos), so an idle
// session shows nothing. Placed above the message list so it stays visible while
// the conversation scrolls beneath it.

import type { WorkState as WorkStateData } from "@/lib/types";

interface Props {
  ws: WorkStateData;
}

function Chip({ children, tone }: { children: React.ReactNode; tone?: "active" | "next" }) {
  const cls =
    tone === "active"
      ? "border-accent/40 bg-accent/10 text-accent-soft"
      : "border-ink-700 bg-ink-900/70 text-ink-400";
  return (
    <li
      className={`flex max-w-[220px] items-center gap-1 truncate rounded-md border px-1.5 py-0.5 text-[11px] leading-none ${cls}`}
      title={typeof children === "string" ? children : undefined}
    >
      <span className="truncate">{children}</span>
    </li>
  );
}

export function WorkStatePanel({ ws }: Props) {
  const hasContent =
    ws.goal ||
    ws.in_progress.length > 0 ||
    ws.next.length > 0 ||
    ws.recent_files.length > 0;
  if (!hasContent) return null;

  return (
    <div className="border-b border-ink-800/80 bg-ink-950/60 px-4 py-1.5 backdrop-blur sm:px-6">
      <div className="mx-auto flex max-w-3xl flex-wrap items-center gap-x-4 gap-y-1.5">
        {ws.goal && (
          <div className="flex min-w-0 items-center gap-1.5">
            <span className="shrink-0 text-[10px] font-semibold uppercase tracking-wide text-ink-600">
              goal
            </span>
            <span className="truncate text-[12px] font-medium text-ink-200" title={ws.goal}>
              {ws.goal}
            </span>
          </div>
        )}
        {ws.in_progress.length > 0 && (
          <div className="flex min-w-0 flex-wrap items-center gap-1">
            <span className="shrink-0 text-[10px] font-semibold uppercase tracking-wide text-ink-600">
              doing
            </span>
            <ul className="flex flex-wrap gap-1">
              {ws.in_progress.slice(0, 4).map((s, i) => (
                <Chip key={i} tone="active">
                  {s}
                </Chip>
              ))}
              {ws.in_progress.length > 4 && (
                <li className="text-[11px] text-ink-600">+{ws.in_progress.length - 4}</li>
              )}
            </ul>
          </div>
        )}
        {ws.next.length > 0 && (
          <div className="flex min-w-0 flex-wrap items-center gap-1">
            <span className="shrink-0 text-[10px] font-semibold uppercase tracking-wide text-ink-600">
              next
            </span>
            <ul className="flex flex-wrap gap-1">
              {ws.next.slice(0, 4).map((s, i) => (
                <Chip key={i}>{s}</Chip>
              ))}
              {ws.next.length > 4 && (
                <li className="text-[11px] text-ink-600">+{ws.next.length - 4}</li>
              )}
            </ul>
          </div>
        )}
        {ws.recent_files.length > 0 && (
          <div className="flex min-w-0 items-center gap-1.5">
            <span className="shrink-0 text-[10px] font-semibold uppercase tracking-wide text-ink-600">
              files
            </span>
            <span className="truncate font-mono text-[11px] text-ink-500" title={ws.recent_files.join(", ")}>
              {ws.recent_files.slice(0, 4).join(", ")}
            </span>
          </div>
        )}
        {ws.last_activity && (
          <span
            className="ml-auto hidden shrink-0 truncate text-[10px] text-ink-600 lg:inline"
            title={ws.last_activity}
          >
            {ws.last_activity}
          </span>
        )}
      </div>
    </div>
  );
}
