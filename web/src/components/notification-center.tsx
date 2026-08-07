"use client";

// NotificationCenter — the in-app half of the cross-session notification system.
//
// A bell in the header with an unread badge; clicking opens a dropdown feed of
// background-session events (a session finished its turn, or now needs the user).
// Each item navigates to that session (cross-project) on click. This is the
// PERSISTENT complement to the transient toasts — items survive until read or
// dismissed, so a background session's "needs approval" can't vanish in 5s.
//
// The tab badge + OS desktop notifications are driven separately in useAgent
// from the same feed; this component only renders the feed itself.

import { useId, useRef, useState } from "react";
import type { NotificationItem } from "@/lib/types";
import { relativeTime, basename } from "@/lib/format";
import { attentionLabel } from "@/lib/notifications";
import { useOutsideClose } from "@/lib/use-outside-close";
import { BellIcon, XIcon, CheckDoubleIcon, TrashIcon } from "./icons";

interface Props {
  notifications: NotificationItem[];
  /** The workspace the client is currently viewing (to mark same-project items). */
  currentWorkspace?: string;
  onOpen: (n: NotificationItem) => void;
  onDismiss: (id: string) => void;
  onMarkAllRead: () => void;
  onClear: () => void;
}

export function NotificationCenter(props: Props) {
  const [open, setOpen] = useState(false);
  const triggerRef = useRef<HTMLButtonElement>(null);
  const popoverId = useId();
  const headingId = useId();
  const ref = useOutsideClose(() => setOpen(false), open);

  const sorted = [...props.notifications].sort((a, b) => b.ts - a.ts);
  const unread = sorted.filter((n) => !n.read);
  const unreadCount = unread.length;
  const hasAttention = unread.some((n) => n.kind === "attention");

  return (
    <div className="relative" ref={ref}>
      <button
        ref={triggerRef}
        type="button"
        onClick={() => setOpen((o) => !o)}
        aria-label={`Notifications${unreadCount > 0 ? ` (${unreadCount} unread)` : ""}`}
        aria-haspopup="dialog"
        aria-expanded={open}
        aria-controls={popoverId}
        title="Notifications"
        className={`focus-ring relative flex h-11 w-11 items-center justify-center rounded-sm transition-colors hover:bg-ink-800 hover:text-ink-100 ${
          unreadCount > 0
            ? hasAttention
              ? "text-danger"
              : "text-accent-soft"
            : "text-ink-400"
        }`}
      >
        <BellIcon width={14} height={14} />
        {unreadCount > 0 && (
          <span
            aria-hidden="true"
            className={`absolute right-1 top-1 flex h-3.5 min-w-3.5 items-center justify-center rounded-sm px-0.5 text-[8px] font-bold leading-none text-white ${
              hasAttention ? "bg-danger" : "bg-accent"
            }`}
          >
            {unreadCount > 9 ? "9+" : unreadCount}
          </span>
        )}
      </button>

      {open && (
        <div
          id={popoverId}
          role="dialog"
          aria-modal="false"
          aria-labelledby={headingId}
          tabIndex={-1}
          onKeyDown={(event) => {
            if (event.key === "Escape") {
              event.preventDefault();
              setOpen(false);
              triggerRef.current?.focus();
            }
          }}
          className="absolute right-0 z-30 mt-1 flex max-h-[min(70vh,30rem)] w-[min(20rem,calc(100vw-1rem))] flex-col overflow-hidden rounded-sm border border-ink-700 bg-ink-900 shadow-elev-2 outline-none animate-fade-in sm:w-80"
        >
          <div className="flex min-h-11 items-center justify-between border-b border-ink-800 px-2">
            <span id={headingId} className="text-[10px] font-mono uppercase tracking-wider text-ink-500">
              Notifications
            </span>
            <div className="flex items-center gap-1">
              {sorted.length > 0 && unreadCount > 0 && (
                <button
                  type="button"
                  onClick={props.onMarkAllRead}
                  className="focus-ring flex min-h-11 items-center gap-1 rounded-sm px-2 text-[10px] font-mono text-ink-400 transition-colors hover:bg-ink-800 hover:text-ink-100"
                  title="Mark all read"
                >
                  <CheckDoubleIcon width={11} height={11} /> Read all
                </button>
              )}
              {sorted.length > 0 && (
                <button
                  type="button"
                  onClick={props.onClear}
                  className="focus-ring flex min-h-11 items-center gap-1 rounded-sm px-2 text-[10px] font-mono text-ink-400 transition-colors hover:bg-ink-800 hover:text-ink-100"
                  title="Clear all"
                >
                  <TrashIcon width={11} height={11} /> Clear
                </button>
              )}
            </div>
          </div>

          <div className="min-h-0 flex-1 overflow-y-auto">
            {sorted.length === 0 ? (
              <div className="px-3 py-6 text-center text-[11px] text-ink-500">
                No notifications.
                <div className="mt-1 text-[10px] text-ink-600">
                  You&apos;ll be notified here when another session finishes or needs your attention.
                </div>
              </div>
            ) : (
              <ul className="divide-y divide-ink-800">
                {sorted.map((n) => {
                  const attention = n.kind === "attention";
                  const sameProject =
                    !!props.currentWorkspace && n.workspace === props.currentWorkspace;
                  return (
                    <li key={n.id} className="group relative">
                      <button
                        type="button"
                        onClick={() => {
                          props.onDismiss(n.id);
                          props.onOpen(n);
                          setOpen(false);
                        }}
                        className={`focus-ring flex min-h-11 w-full items-start gap-2 py-1.5 pl-2 pr-12 text-left transition-colors hover:bg-ink-800 ${
                          n.read ? "text-ink-500" : ""
                        }`}
                      >
                        <span
                          className={`mt-1 h-1.5 w-1.5 shrink-0 rounded-none ${
                            n.read
                              ? "bg-ink-600"
                              : attention
                                ? "bg-danger"
                                : "bg-accent"
                          }`}
                          aria-hidden="true"
                        />
                        <div className="min-w-0 flex-1">
                          <div
                            className={`truncate text-[11px] font-medium leading-4 ${
                              n.read ? "text-ink-400" : "text-ink-100"
                            }`}
                            title={n.title}
                          >
                            {n.title || "Session"}
                          </div>
                          <div className="mt-0.5 flex items-center gap-1.5 font-mono text-[10px] leading-4 text-ink-500">
                            <span
                              className={
                                n.read
                                  ? "text-ink-500"
                                  : attention
                                    ? "text-danger"
                                    : "text-accent-soft"
                              }
                            >
                              {attention ? attentionLabel(n.attentionKind) : "finished"}
                            </span>
                            <span>·</span>
                            <span>{relativeTime(n.ts)}</span>
                            {!sameProject && n.workspace && (
                              <>
                                <span>·</span>
                                <span className="truncate">{basename(n.workspace) || "project"}</span>
                              </>
                            )}
                          </div>
                        </div>
                      </button>
                      <button
                        type="button"
                        onClick={() => props.onDismiss(n.id)}
                        className="focus-ring absolute right-0 top-0 flex h-11 w-11 items-center justify-center rounded-sm text-ink-500 transition-colors hover:bg-ink-700 hover:text-ink-100"
                        aria-label={`Dismiss ${n.title || "session"} notification`}
                      >
                        <XIcon width={11} height={11} />
                      </button>
                    </li>
                  );
                })}
              </ul>
            )}
          </div>
        </div>
      )}
    </div>
  );
}
