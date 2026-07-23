"use client";

// WorkState — a collapsible IDE status band rendered from `work_state` events.
//
// The core keeps a rolling, signal-derived summary of the current task (goal,
// what's done / in progress / next, recently touched files) and emits it so the
// UI can show live context alongside the conversation. Only rendered when
// there's meaningful content (a goal or active todos), so an idle session shows
// nothing.
//
// Collapsible: collapsed (the default) shows the compact single-row flat band
// with truncated values and capped counts — the existing at-a-glance idiom.
// Expanded reveals the full goal text and every done/doing/next item without
// truncation or caps. The choice persists in localStorage so it survives reloads
// and session switches.
//
// Design intent: a flat status band (bg-ink-925, hairline border-b) with mono
// uppercase segment labels (GOAL / DONE / DOING / NEXT) separated by hairline
// vertical rules — the ActivityBar/PanelHeader idiom, not chips or pills. Raw
// `last_activity` (a bare tool-call string) is intentionally not surfaced. After
// a goal completes, completed steps stay in `done` so the feed does not go empty
// when in_progress/next clear.

import { useEffect, useState } from "react";
import type { WorkState as WorkStateData } from "@/lib/types";
import { ChevronDown, ChevronRight } from "./icons";

const LS_KEY = "catalyst:work-state:expanded";

interface Props {
  ws: WorkStateData;
  /** Narrow dock / IdeShell copilot — align width with transcript, denser padding. */
  compact?: boolean;
}

function Sep() {
  return <span className="h-3 w-px shrink-0 bg-ink-700" aria-hidden />;
}

function Label({ children }: { children: React.ReactNode }) {
  return (
    <span className="shrink-0 font-mono text-[10px] uppercase tracking-wider text-ink-500">
      {children}
    </span>
  );
}

function Value({
  children,
  title,
}: {
  children: React.ReactNode;
  title?: string;
}) {
  return (
    <span
      className="max-w-[200px] truncate text-[11px] leading-none text-ink-300"
      title={title ?? (typeof children === "string" ? children : undefined)}
    >
      {children}
    </span>
  );
}

function Toggle({
  expanded,
  onClick,
}: {
  expanded: boolean;
  onClick: () => void;
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      className="flex h-6 w-6 shrink-0 items-center justify-center rounded-sm text-ink-400 hover:bg-ink-800 hover:text-ink-100 focus:outline-none focus-visible:ring-1 focus-visible:ring-accent/60"
      aria-label={expanded ? "Collapse status band" : "Expand status band"}
      aria-expanded={expanded}
      title={expanded ? "Collapse" : "Expand"}
    >
      {expanded ? (
        <ChevronDown width={14} height={14} />
      ) : (
        <ChevronRight width={14} height={14} />
      )}
    </button>
  );
}

/** One labeled section in the expanded view — label pinned left, items wrap. */
function Section({
  label,
  items,
  dot,
  tone,
}: {
  label: string;
  items: string[];
  dot: string;
  tone: string;
}) {
  return (
    <div className="flex items-start gap-2">
      <span className={`mt-1 h-1.5 w-1.5 shrink-0 rounded-none ${dot}`} aria-hidden />
      <Label>{label}</Label>
      <ul className="flex min-w-0 flex-1 flex-wrap items-start gap-x-1.5 gap-y-0.5">
        {items.map((s, i) => (
          <li
            key={`${label}-${i}`}
            className="flex items-center gap-1.5 text-[11px] leading-snug text-ink-300"
          >
            <span className={tone}>{s}</span>
            {i < items.length - 1 && (
              <span className="text-ink-700" aria-hidden>
                ·
              </span>
            )}
          </li>
        ))}
      </ul>
    </div>
  );
}

export function WorkStatePanel({ ws, compact }: Props) {
  const [expanded, setExpanded] = useState(false);

  // Adopt the persisted preference after mount so first paint is deterministic
  // (collapsed). The band only renders once `work_state` arrives over SSE —
  // always after this effect — so there is no visible flash.
  useEffect(() => {
    try {
      setExpanded(window.localStorage.getItem(LS_KEY) === "1");
    } catch {
      /* ignore */
    }
  }, []);

  const toggle = () => {
    const next = !expanded;
    setExpanded(next);
    try {
      window.localStorage.setItem(LS_KEY, next ? "1" : "0");
    } catch {
      /* ignore */
    }
  };

  // Keep the feed after a goal completes: completed steps live in `done`, and
  // in_progress/next go empty — without counting `done`, the strip vanishes.
  const hasContent =
    !!ws.goal ||
    ws.done.length > 0 ||
    ws.in_progress.length > 0 ||
    ws.next.length > 0 ||
    ws.recent_files.length > 0;
  if (!hasContent) return null;

  const goalComplete =
    ws.done.length > 0 &&
    ws.in_progress.length === 0 &&
    ws.next.length === 0;
  const doneCap = goalComplete ? 6 : 3;

  const showGoal = !!ws.goal;
  const showDone = ws.done.length > 0;
  const showDoing = ws.in_progress.length > 0;
  const showNext = ws.next.length > 0;
  const showFiles = ws.recent_files.length > 0;

  return (
    <div className="w-full border-b border-ink-800 bg-ink-925">
      {expanded ? (
        // Expanded — full detail: goal wrapped (no truncation), every item shown.
        <div
          className={`mx-auto w-full ${compact ? "max-w-none px-2 py-2" : "max-w-3xl px-3 py-2.5"}`}
        >
          <div className="flex items-start gap-2">
            <span
              className={`mt-1 h-1.5 w-1.5 shrink-0 rounded-none ${goalComplete ? "bg-success" : "bg-accent"}`}
              aria-hidden
            />
            <div className="min-w-0 flex-1">
              {showGoal ? (
                <>
                  <div className="flex items-center gap-1.5">
                    <Label>goal</Label>
                    {goalComplete && (
                      <span className="font-mono text-[10px] uppercase tracking-wider text-success">
                        done
                      </span>
                    )}
                  </div>
                  <p className="mt-0.5 break-words text-[12px] leading-snug text-ink-200">
                    {ws.goal}
                  </p>
                </>
              ) : (
                <Label>work state</Label>
              )}
            </div>
            <Toggle expanded onClick={toggle} />
          </div>

          {(showDone || showDoing || showNext) && (
            <div className="mt-2 space-y-1.5">
              {showDone && (
                <Section label="done" items={ws.done} dot="bg-ink-600" tone="text-ink-400" />
              )}
              {showDoing && (
                <Section label="doing" items={ws.in_progress} dot="bg-accent" tone="text-ink-100" />
              )}
              {showNext && (
                <Section label="next" items={ws.next} dot="bg-ink-700" tone="text-ink-400" />
              )}
            </div>
          )}

          {showFiles && (
            <div className="mt-2 flex items-center gap-1.5">
              <Label>files</Label>
              <span
                className="min-w-0 truncate font-mono text-[10px] text-ink-600"
                title={ws.recent_files.join(", ")}
              >
                {ws.recent_files.join(" · ")}
              </span>
            </div>
          )}
        </div>
      ) : (
        // Collapsed — compact single-row flat band (truncated values, capped counts).
        <div
          className={`mx-auto flex w-full flex-wrap items-center gap-x-2 gap-y-1 ${compact ? "max-w-none px-2 py-1" : "max-w-3xl px-3 py-1.5"}`}
        >
          {/* goal — accent/success status square, then the truncated goal text. */}
          {showGoal && (
            <div className="flex min-w-0 items-center gap-1.5">
              <span
                className={`h-1.5 w-1.5 shrink-0 rounded-none ${goalComplete ? "bg-success" : "bg-accent"}`}
                aria-hidden
              />
              <Label>goal</Label>
              <Value title={ws.goal}>{ws.goal}</Value>
              {goalComplete && (
                <span className="shrink-0 font-mono text-[10px] uppercase tracking-wider text-success">
                  done
                </span>
              )}
            </div>
          )}

          {/* done — completed steps stay visible after the goal finishes. */}
          {showDone && (
            <>
              {showGoal && <Sep />}
              <div className="flex min-w-0 items-center gap-1.5">
                <Label>done</Label>
                <ul className="flex min-w-0 flex-wrap items-center gap-1.5">
                  {ws.done.slice(0, doneCap).map((s, i) => (
                    <li key={`done-${i}`} className="flex items-center">
                      <Value>{s}</Value>
                    </li>
                  ))}
                  {ws.done.length > doneCap && (
                    <li className="self-center font-mono text-[10px] text-ink-600">
                      +{ws.done.length - doneCap}
                    </li>
                  )}
                </ul>
              </div>
            </>
          )}

          {/* doing — live progress. */}
          {showDoing && (
            <>
              {(showGoal || showDone) && <Sep />}
              <div className="flex min-w-0 items-center gap-1.5">
                <Label>doing</Label>
                <ul className="flex min-w-0 flex-wrap items-center gap-1.5">
                  {ws.in_progress.slice(0, 4).map((s, i) => (
                    <li key={`doing-${i}`} className="flex items-center">
                      <Value>{s}</Value>
                    </li>
                  ))}
                  {ws.in_progress.length > 4 && (
                    <li className="self-center font-mono text-[10px] text-ink-600">
                      +{ws.in_progress.length - 4}
                    </li>
                  )}
                </ul>
              </div>
            </>
          )}

          {/* next — upcoming steps. */}
          {showNext && (
            <>
              {(showGoal || showDone || showDoing) && <Sep />}
              <div className="flex min-w-0 items-center gap-1.5">
                <Label>next</Label>
                <ul className="flex min-w-0 flex-wrap items-center gap-1.5">
                  {ws.next.slice(0, 4).map((s, i) => (
                    <li key={`next-${i}`} className="flex items-center">
                      <Value>{s}</Value>
                    </li>
                  ))}
                  {ws.next.length > 4 && (
                    <li className="self-center font-mono text-[10px] text-ink-600">
                      +{ws.next.length - 4}
                    </li>
                  )}
                </ul>
              </div>
            </>
          )}

          {/* Right-aligned group: files caption + expand toggle. */}
          <div className="ml-auto flex shrink-0 items-center gap-1.5">
            {showFiles && (
              <span
                className="hidden min-w-0 truncate font-mono text-[10px] text-ink-600 md:inline"
                title={ws.recent_files.join(", ")}
              >
                {ws.recent_files.slice(0, 4).join(" · ")}
              </span>
            )}
            <Toggle expanded={false} onClick={toggle} />
          </div>
        </div>
      )}
    </div>
  );
}
