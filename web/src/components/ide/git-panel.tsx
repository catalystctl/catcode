"use client";

// Source Control panel for the IDE shell. User-driven (not an agent turn): it
// talks directly to /api/git. Owns its fetch lifecycle (mount + 10s poll +
// window-focus refresh) and mirrors results into the IDE state slice via
// ide.setGitStatus. Consumes { workspace, ide } from IdeContext.
//
// Layout: branch + sync header, commit box, then changed files grouped into
// STAGED / CHANGES / UNTRACKED. Clicking a tracked file shows its unified diff
// (reusing <Diff/>); clicking an untracked file opens it in the editor.

import { useCallback, useEffect, useMemo, useState } from "react";
import { useIdeContext } from "@/lib/ide-context";
import type { GitStatus, GitStatusEntry } from "@/lib/types";
import { Diff } from "@/components/diff";
import {
  GitBranchIcon,
  PlusIcon,
  MinusIcon,
  TrashIcon,
  RefreshIcon,
  CheckIcon,
} from "@/components/icons";

function statusLetter(s: GitStatusEntry["status"]): string {
  switch (s) {
    case "modified":
      return "M";
    case "added":
      return "A";
    case "deleted":
      return "D";
    case "renamed":
      return "R";
    case "conflicted":
      return "U";
    case "untracked":
      return "U";
    default:
      return "M";
  }
}

function statusColor(s: GitStatusEntry["status"]): string {
  switch (s) {
    case "modified":
      return "text-amber-300";
    case "added":
    case "untracked":
      return "text-emerald-400";
    case "deleted":
    case "conflicted":
      return "text-red-400";
    case "renamed":
      return "text-sky-400";
    default:
      return "text-amber-300";
  }
}

function groupEntries(status: GitStatus | null) {
  const entries = status?.entries ?? [];
  return {
    staged: entries.filter((e) => e.staged && e.status !== "untracked"),
    unstaged: entries.filter((e) => !e.staged && e.status !== "untracked"),
    untracked: entries.filter((e) => e.status === "untracked"),
  };
}

export function GitPanel({ compact }: { compact?: boolean }) {
  const { workspace, ide } = useIdeContext();
  const status = ide.state.gitStatus;

  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [message, setMessage] = useState("");
  const [selected, setSelected] = useState<string | null>(null);
  const [diff, setDiff] = useState("");
  const [diffLoading, setDiffLoading] = useState(false);
  const [showSwitch, setShowSwitch] = useState(false);
  const [branchInput, setBranchInput] = useState("");

  const refresh = useCallback(
    async (silent = false) => {
      if (!silent) setLoading(true);
      try {
        const res = await fetch(
          `/api/git?workspace=${encodeURIComponent(workspace)}`,
        );
        const data = await res.json().catch(() => ({}));
        if (res.ok) {
          ide.setGitStatus(data as GitStatus);
          setError(null);
        } else {
          setError(
            typeof data.error === "string" ? data.error : "failed to load git status",
          );
        }
      } catch (e) {
        setError(e instanceof Error ? e.message : "network error");
      } finally {
        if (!silent) setLoading(false);
      }
    },
    [workspace, ide],
  );

  // Mount refresh + 10s poll while the document is visible + refresh on focus.
  // Background polls are silent (no loading flicker); the manual refresh button
  // calls refresh() (non-silent) to show the spinner state.
  useEffect(() => {
    refresh();
    const id = setInterval(() => {
      if (document.visibilityState === "visible") refresh(true);
    }, 10000);
    const onFocus = () => refresh(true);
    window.addEventListener("focus", onFocus);
    return () => {
      clearInterval(id);
      window.removeEventListener("focus", onFocus);
    };
  }, [refresh]);

  const postAction = useCallback(
    async (
      action: string,
      extra: Record<string, unknown> = {},
    ): Promise<boolean> => {
      try {
        setBusy(true);
        const res = await fetch("/api/git", {
          method: "POST",
          headers: { "Content-Type": "application/json" },
          body: JSON.stringify({ action, workspace, ...extra }),
        });
        const data = await res.json().catch(() => ({}));
        if (!res.ok) {
          setError(
            typeof data.error === "string" ? data.error : `${action} failed`,
          );
          return false;
        }
        setError(null);
        if (data && data.status) ide.setGitStatus(data.status as GitStatus);
        // An action may have changed the selected file's diff; drop the stale view.
        setSelected(null);
        setDiff("");
        return true;
      } catch (e) {
        setError(e instanceof Error ? e.message : "network error");
        return false;
      } finally {
        setBusy(false);
      }
    },
    [workspace, ide],
  );

  const fetchDiff = useCallback(
    async (entry: GitStatusEntry) => {
      if (entry.status === "untracked") {
        ide.openFile(entry.path);
        return;
      }
      setSelected(entry.path);
      setDiff("");
      setDiffLoading(true);
      try {
        const res = await fetch(
          `/api/git?workspace=${encodeURIComponent(workspace)}&diff=${encodeURIComponent(entry.path)}&staged=${entry.staged ? 1 : 0}`,
        );
        const data = await res.json().catch(() => ({}));
        if (res.ok) setDiff(typeof data.diff === "string" ? data.diff : "");
        else setDiff("");
      } catch {
        setDiff("");
      } finally {
        setDiffLoading(false);
      }
    },
    [workspace, ide],
  );

  const commit = useCallback(async () => {
    if (!message.trim()) return;
    const ok = await postAction("commit", { message });
    if (ok) setMessage("");
  }, [message, postAction]);

  const doCheckout = useCallback(async () => {
    if (!branchInput.trim()) return;
    const ok = await postAction("checkout", { branch: branchInput.trim() });
    if (ok) {
      setShowSwitch(false);
      setBranchInput("");
    }
  }, [branchInput, postAction]);

  const groups = useMemo(() => groupEntries(status), [status]);
  const totalChanges = (status?.entries ?? []).length;

  // ── compact (status-bar) mode ────────────────────────────────────────────
  if (compact) {
    if (!status || status.bare) {
      return (
        <span className="flex items-center gap-1 text-[12px] text-ink-500">
          <GitBranchIcon width={12} height={12} />
          <span>no repo</span>
        </span>
      );
    }
    return (
      <span className="flex items-center gap-1.5 text-[12px] text-ink-400">
        <GitBranchIcon width={12} height={12} />
        <span className="text-ink-200">{status.branch}</span>
        {status.ahead > 0 && <span className="text-emerald-400">↑{status.ahead}</span>}
        {status.behind > 0 && <span className="text-amber-300">↓{status.behind}</span>}
        {totalChanges > 0 && (
          <span className="text-ink-500">• {totalChanges} change{totalChanges === 1 ? "" : "s"}</span>
        )}
      </span>
    );
  }

  // ── full panel ────────────────────────────────────────────────────────────
  const canCommit = groups.staged.length > 0 && message.trim().length > 0 && !busy;

  return (
    <div className="flex h-full flex-col text-ink-200">
      {/* Header */}
      <div className="flex items-center justify-between border-b border-ink-800 px-3 py-2">
        <span className="text-[11px] font-semibold uppercase tracking-wider text-ink-400">
          Source Control
        </span>
        <button
          onClick={() => refresh()}
          disabled={loading}
          title="Refresh"
          className="rounded p-1 text-ink-400 hover:bg-ink-800 hover:text-ink-100 disabled:opacity-50"
        >
          <RefreshIcon width={14} height={14} />
        </button>
      </div>

      <div className="flex-1 overflow-y-auto">
        {error && (
          <div className="m-2 rounded border border-red-400/30 bg-red-400/10 px-2 py-1.5 text-[12px] text-red-300">
            {error}
          </div>
        )}

        {!status && loading && (
          <div className="px-3 py-4 text-[12px] text-ink-500">Loading git status…</div>
        )}

        {status && status.bare && (
          <div className="px-3 py-4 text-center">
            <p className="mb-3 text-[12px] text-ink-400">
              This workspace is not a git repository.
            </p>
            <button
              onClick={() => postAction("init")}
              disabled={busy}
              className="rounded bg-accent px-3 py-1.5 text-[12px] font-medium text-ink-950 hover:bg-accent-soft disabled:opacity-50"
            >
              Initialize Repository
            </button>
          </div>
        )}

        {status && !status.bare && (
          <>
            {/* Branch + sync row */}
            <div className="flex items-center gap-2 border-b border-ink-800 px-3 py-2 text-[12px]">
              <GitBranchIcon width={13} height={13} className="text-ink-400" />
              <span className="truncate font-medium text-ink-100">{status.branch}</span>
              {status.ahead > 0 && (
                <span className="text-emerald-400">↑{status.ahead}</span>
              )}
              {status.behind > 0 && (
                <span className="text-amber-300">↓{status.behind}</span>
              )}
              <div className="ml-auto flex items-center gap-1">
                <SyncBtn label="Pull" onClick={() => postAction("pull")} disabled={busy} />
                <SyncBtn label="Push" onClick={() => postAction("push")} disabled={busy} />
                <SyncBtn label="Fetch" onClick={() => postAction("fetch")} disabled={busy} />
                <button
                  onClick={() => setShowSwitch((v) => !v)}
                  title="Switch branch"
                  className="rounded px-1.5 py-0.5 text-ink-400 hover:bg-ink-800 hover:text-ink-100"
                >
                  ⇄
                </button>
              </div>
            </div>

            {showSwitch && (
              <div className="flex items-center gap-1 border-b border-ink-800 px-3 py-2">
                <input
                  value={branchInput}
                  onChange={(e) => setBranchInput(e.target.value)}
                  onKeyDown={(e) => {
                    if (e.key === "Enter") doCheckout();
                  }}
                  placeholder="branch name"
                  className="min-w-0 flex-1 rounded border border-ink-700 bg-ink-950 px-2 py-1 text-[12px] text-ink-100 outline-none focus:border-accent"
                />
                <button
                  onClick={doCheckout}
                  disabled={busy || !branchInput.trim()}
                  className="rounded bg-accent px-2 py-1 text-[12px] font-medium text-ink-950 hover:bg-accent-soft disabled:opacity-50"
                >
                  Switch
                </button>
              </div>
            )}

            {/* Commit box */}
            <div className="border-b border-ink-800 p-2">
              <textarea
                value={message}
                onChange={(e) => setMessage(e.target.value)}
                onKeyDown={(e) => {
                  if ((e.metaKey || e.ctrlKey) && e.key === "Enter" && canCommit) {
                    e.preventDefault();
                    commit();
                  }
                }}
                placeholder="Commit message (Ctrl+Enter to commit)"
                rows={2}
                className="w-full resize-none rounded border border-ink-700 bg-ink-950 px-2 py-1.5 text-[12px] text-ink-100 outline-none placeholder:text-ink-600 focus:border-accent"
              />
              <button
                onClick={commit}
                disabled={!canCommit}
                className="mt-1.5 flex w-full items-center justify-center gap-1.5 rounded bg-accent px-2 py-1.5 text-[12px] font-medium text-ink-950 hover:bg-accent-soft disabled:opacity-40"
              >
                <CheckIcon width={13} height={13} />
                Commit{groups.staged.length > 0 ? ` (${groups.staged.length})` : ""}
              </button>
            </div>

            {/* Changes groups */}
            <Group
              title="Staged Changes"
              count={groups.staged.length}
              actionLabel="Unstage All"
              onAction={() => postAction("unstageAll")}
              busy={busy}
            >
              {groups.staged.map((e) => (
                <Row
                  key={`s-${e.path}`}
                  entry={e}
                  active={selected === e.path}
                  onClick={() => fetchDiff(e)}
                >
                  <RowBtn
                    title="Unstage"
                    disabled={busy}
                    onClick={() => postAction("unstage", { path: e.path })}
                  >
                    <MinusIcon width={13} height={13} />
                  </RowBtn>
                </Row>
              ))}
            </Group>

            <Group
              title="Changes"
              count={groups.unstaged.length}
              actionLabel="Stage All"
              onAction={() => postAction("stageAll")}
              busy={busy}
            >
              {groups.unstaged.map((e) => (
                <Row
                  key={`c-${e.path}`}
                  entry={e}
                  active={selected === e.path}
                  onClick={() => fetchDiff(e)}
                >
                  <RowBtn
                    title="Stage"
                    disabled={busy}
                    onClick={() => postAction("stage", { path: e.path })}
                  >
                    <PlusIcon width={13} height={13} />
                  </RowBtn>
                  <RowBtn
                    title="Discard changes"
                    disabled={busy}
                    danger
                    onClick={() => {
                      if (
                        window.confirm(
                          `Discard changes to ${e.path}? This cannot be undone.`,
                        )
                      ) {
                        postAction("discard", { path: e.path });
                      }
                    }}
                  >
                    <TrashIcon width={13} height={13} />
                  </RowBtn>
                </Row>
              ))}
            </Group>

            <Group
              title="Untracked"
              count={groups.untracked.length}
              actionLabel="Stage All"
              onAction={() => postAction("stageAll")}
              busy={busy}
            >
              {groups.untracked.map((e) => (
                <Row
                  key={`u-${e.path}`}
                  entry={e}
                  active={selected === e.path}
                  onClick={() => fetchDiff(e)}
                >
                  <RowBtn
                    title="Stage"
                    disabled={busy}
                    onClick={() => postAction("stage", { path: e.path })}
                  >
                    <PlusIcon width={13} height={13} />
                  </RowBtn>
                </Row>
              ))}
            </Group>

            {totalChanges === 0 && (
              <div className="px-3 py-6 text-center text-[12px] text-ink-500">
                No changes — working tree clean.
              </div>
            )}

            {/* Diff view for the selected tracked file */}
            {selected && (
              <div className="border-t border-ink-800">
                <div className="flex items-center justify-between px-3 py-1.5">
                  <span className="truncate text-[11px] text-ink-400" title={selected}>
                    {selected}
                  </span>
                  <button
                    onClick={() => {
                      setSelected(null);
                      setDiff("");
                    }}
                    className="rounded px-1.5 py-0.5 text-[11px] text-ink-500 hover:bg-ink-800 hover:text-ink-100"
                  >
                    close
                  </button>
                </div>
                <div className="max-h-72 overflow-auto px-2 pb-2">
                  {diffLoading ? (
                    <div className="px-1 py-2 text-[12px] text-ink-500">Loading diff…</div>
                  ) : diff ? (
                    <Diff diff={diff} />
                  ) : (
                    <div className="px-1 py-2 text-[12px] text-ink-500">
                      No textual diff available.
                    </div>
                  )}
                </div>
              </div>
            )}
          </>
        )}
      </div>
    </div>
  );
}

function SyncBtn({
  label,
  onClick,
  disabled,
}: {
  label: string;
  onClick: () => void;
  disabled?: boolean;
}) {
  return (
    <button
      onClick={onClick}
      disabled={disabled}
      className="rounded px-1.5 py-0.5 text-[11px] text-ink-400 hover:bg-ink-800 hover:text-ink-100 disabled:opacity-50"
    >
      {label}
    </button>
  );
}

function Group({
  title,
  count,
  actionLabel,
  onAction,
  busy,
  children,
}: {
  title: string;
  count: number;
  actionLabel: string;
  onAction: () => void;
  busy?: boolean;
  children?: React.ReactNode;
}) {
  if (count === 0) return null;
  return (
    <div className="border-b border-ink-800">
      <div className="flex items-center justify-between px-3 py-1.5">
        <span className="text-[11px] font-semibold uppercase tracking-wider text-ink-400">
          {title} <span className="text-ink-600">({count})</span>
        </span>
        <button
          onClick={onAction}
          disabled={busy}
          className="text-[11px] text-ink-500 hover:text-ink-100 disabled:opacity-50"
        >
          {actionLabel}
        </button>
      </div>
      <div className="pb-1">{children}</div>
    </div>
  );
}

function Row({
  entry,
  active,
  onClick,
  children,
}: {
  entry: GitStatusEntry;
  active: boolean;
  onClick: () => void;
  children?: React.ReactNode;
}) {
  return (
    <div
      className={`group flex items-center gap-1.5 px-3 py-1 text-[12px] hover:bg-ink-850 ${
        active ? "bg-ink-850" : ""
      }`}
    >
      <span className={`w-3 shrink-0 text-center font-mono ${statusColor(entry.status)}`}>
        {statusLetter(entry.status)}
      </span>
      <button
        onClick={onClick}
        title={entry.path}
        className="min-w-0 flex-1 truncate text-left text-ink-200 hover:text-ink-100"
      >
        {entry.path}
        {entry.oldPath ? <span className="text-ink-600"> ← {entry.oldPath}</span> : null}
      </button>
      <div className="flex items-center gap-0.5 opacity-0 transition-opacity group-hover:opacity-100">
        {children}
      </div>
    </div>
  );
}

function RowBtn({
  title,
  onClick,
  disabled,
  danger,
  children,
}: {
  title: string;
  onClick: () => void;
  disabled?: boolean;
  danger?: boolean;
  children: React.ReactNode;
}) {
  return (
    <button
      title={title}
      onClick={(e) => {
        e.stopPropagation();
        onClick();
      }}
      disabled={disabled}
      className={`rounded p-0.5 text-ink-400 hover:bg-ink-700 disabled:opacity-40 ${
        danger ? "hover:text-red-400" : "hover:text-ink-100"
      }`}
    >
      {children}
    </button>
  );
}
