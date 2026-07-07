"use client";

// WorkState — a calm ambient status line rendered from `work_state` events.
//
// The core keeps a rolling, signal-derived summary of the current task (goal,
// what's done / in progress / next, recently touched files) and emits it so the
// UI can show live context alongside the conversation. Only rendered when
// there's meaningful content (a goal or active todos), so an idle session shows
// nothing.
//
// Design intent: this is NOT a second chrome bar. It deliberately drops the
// boxed/bordered/backdrop surface the real Header uses, so it reads as ambient
// text belonging to the content area — a status line, not a toolbar. The goal
// is the hero (brightest, largest); doing/next demote to quiet chips; raw
// `last_activity` (a bare tool-call string) is intentionally not surfaced.

import type { WorkState as WorkStateData } from "@/lib/types";

interface Props {
  ws: WorkStateData;
}

function Chip({ children, tone }: { children: React.ReactNode; tone?: "active" | "next" }) {
  // Tinted badges, no borders — calmer than the old outlined chips and on-system
  // with the settings panel's `bg-accent/15` accent badges.
  const cls =
    tone === "active"
      ? "bg-accent/10 text-accent-soft"
      : "bg-ink-800/50 text-ink-500";
  return (
    <li
      className={`flex max-w-[200px] items-center gap-1 truncate rounded px-1.5 py-0.5 text-[11px] leading-none ${cls}`}
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
    <div className="mx-auto max-w-3xl px-4 sm:px-6">
      {/* A single faint hairline anchors the line as a status strip without the
          heavy boxed surface that made it read as a second header. */}
      <div className="flex flex-wrap items-center gap-x-3 gap-y-1.5 border-b border-ink-900/60 pb-2.5 pt-3">
        {/* goal — the hero. One accent dot, no label. */}
        {ws.goal && (
          <div className="flex min-w-0 items-center gap-2">
            <span className="h-1.5 w-1.5 shrink-0 rounded-full bg-accent/80" aria-hidden />
            <span className="truncate text-[13px] text-ink-200" title={ws.goal}>
              {ws.goal}
            </span>
          </div>
        )}

        {/* doing — live progress, accent-tinted chips. */}
        {ws.in_progress.length > 0 && (
          <ul className="flex min-w-0 flex-wrap items-center gap-1">
            {ws.in_progress.slice(0, 4).map((s, i) => (
              <Chip key={i} tone="active">
                {s}
              </Chip>
            ))}
            {ws.in_progress.length > 4 && (
              <li className="self-center text-[11px] text-ink-600">+{ws.in_progress.length - 4}</li>
            )}
          </ul>
        )}

        {/* next — upcoming, muted chips behind a quiet arrow. */}
        {ws.next.length > 0 && (
          <div className="flex min-w-0 flex-wrap items-center gap-1">
            <span className="select-none text-ink-700" aria-hidden>
              →
            </span>
            <ul className="flex flex-wrap items-center gap-1">
              {ws.next.slice(0, 4).map((s, i) => (
                <Chip key={i}>{s}</Chip>
              ))}
              {ws.next.length > 4 && (
                <li className="self-center text-[11px] text-ink-600">+{ws.next.length - 4}</li>
              )}
            </ul>
          </div>
        )}

        {/* files — demoted, right-aligned mono caption; hidden on small screens. */}
        {ws.recent_files.length > 0 && (
          <span
            className="ml-auto hidden min-w-0 shrink-0 truncate font-mono text-[10px] text-ink-600 md:inline"
            title={ws.recent_files.join(", ")}
          >
            {ws.recent_files.slice(0, 4).join(" · ")}
          </span>
        )}
      </div>
    </div>
  );
}
