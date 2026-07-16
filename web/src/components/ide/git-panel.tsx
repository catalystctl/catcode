"use client";

import { useCallback, useEffect, useMemo, useState, type ReactNode } from "react";
import { Diff } from "@/components/diff";
import { AppDialogHost, useAppDialog } from "@/components/app-dialog";
import {
  CheckIcon,
  CopyIcon,
  FileIcon,
  GitBranchIcon,
  MinusIcon,
  PlusIcon,
  RefreshIcon,
  TrashIcon,
  XIcon,
} from "@/components/icons";
import { useIdeContext } from "@/lib/ide-context";
import type {
  GitBranch,
  GitCommit,
  GitOperation,
  GitRemote,
  GitStash,
  GitStatus,
  GitStatusEntry,
  GitTag,
} from "@/lib/types";

type Tab = "changes" | "history" | "branches" | "stashes" | "repository";
type Action = (action: string, extra?: Record<string, unknown>) => Promise<boolean>;
type ConfirmFn = ReturnType<typeof useAppDialog>["confirm"];
type PromptFn = ReturnType<typeof useAppDialog>["prompt"];

const inputClass =
  "min-w-0 rounded-lg border border-ink-700 bg-ink-950 px-2.5 py-1.5 text-[12px] text-ink-100 outline-none placeholder:text-ink-600 focus:border-accent/50";
const buttonClass =
  "rounded-lg border border-ink-700 px-2.5 py-1 text-[11px] font-medium text-ink-300 transition-colors hover:border-ink-600 hover:bg-ink-800 hover:text-ink-100 disabled:opacity-40";
const primaryClass =
  "rounded-lg bg-accent px-2.5 py-1.5 text-[11px] font-semibold text-white transition-colors hover:bg-accent-soft disabled:opacity-40";

function statusLetter(status: GitStatusEntry["status"]) {
  return (
    {
      modified: "M",
      added: "A",
      deleted: "D",
      renamed: "R",
      conflicted: "!",
      untracked: "U",
    } as const
  )[status];
}

function statusColor(status: GitStatusEntry["status"]) {
  if (status === "added" || status === "untracked") return "text-emerald-400";
  if (status === "deleted" || status === "conflicted") return "text-red-400";
  if (status === "renamed") return "text-sky-400";
  return "text-amber-300";
}

function groupEntries(status: GitStatus | null) {
  const entries = status?.entries ?? [];
  return {
    conflicted: entries.filter((entry) => entry.status === "conflicted"),
    staged: entries.filter((entry) => entry.staged && entry.status !== "untracked" && entry.status !== "conflicted"),
    unstaged: entries.filter(
      (entry) => !entry.staged && entry.status !== "untracked" && entry.status !== "conflicted",
    ),
    untracked: entries.filter((entry) => entry.status === "untracked"),
  };
}

function actionLabel(action: string) {
  return action
    .replace(/([A-Z])/g, " $1")
    .replace(/^./, (c) => c.toUpperCase())
    .trim();
}

function formatDate(timestamp: number) {
  return timestamp
    ? new Intl.DateTimeFormat(undefined, { dateStyle: "medium", timeStyle: "short" }).format(timestamp)
    : "unknown date";
}

async function copyText(text: string) {
  try {
    await navigator.clipboard.writeText(text);
    return true;
  } catch {
    return false;
  }
}

export function GitPanel({ compact }: { compact?: boolean }) {
  const { workspace, ide } = useIdeContext();
  const { confirm, prompt, dialog } = useAppDialog();
  const status = ide.state.gitStatus;
  const [tab, setTab] = useState<Tab>("changes");
  const [loading, setLoading] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [result, setResult] = useState<string | null>(null);
  const [selected, setSelected] = useState<string | null>(null);
  const [diff, setDiff] = useState("");
  const [diffLoading, setDiffLoading] = useState(false);
  const [previewTitle, setPreviewTitle] = useState<string | null>(null);

  const refresh = useCallback(
    async (silent = false) => {
      if (!silent) setLoading(true);
      try {
        const response = await fetch(`/api/git?workspace=${encodeURIComponent(workspace)}`);
        const data = await response.json().catch(() => ({}));
        if (!response.ok) {
          throw new Error(typeof data.error === "string" ? data.error : "Failed to load Git repository");
        }
        ide.setGitStatus(data as GitStatus);
        setError(null);
      } catch (cause) {
        setError(cause instanceof Error ? cause.message : "Network error");
      } finally {
        if (!silent) setLoading(false);
      }
    },
    [ide, workspace],
  );

  useEffect(() => {
    void refresh();
    const interval = window.setInterval(
      () => document.visibilityState === "visible" && void refresh(true),
      10000,
    );
    const onFocus = () => void refresh(true);
    window.addEventListener("focus", onFocus);
    return () => {
      window.clearInterval(interval);
      window.removeEventListener("focus", onFocus);
    };
  }, [refresh]);

  useEffect(() => {
    if (!result) return;
    const t = window.setTimeout(() => setResult(null), 3500);
    return () => window.clearTimeout(t);
  }, [result]);

  const postAction = useCallback<Action>(
    async (action, extra = {}) => {
      setBusy(true);
      setResult(null);
      try {
        const response = await fetch("/api/git", {
          method: "POST",
          headers: { "Content-Type": "application/json" },
          body: JSON.stringify({ action, workspace, ...extra }),
        });
        const data = await response.json().catch(() => ({}));
        if (!response.ok) {
          throw new Error(typeof data.error === "string" ? data.error : `${action} failed`);
        }
        if (data.status) ide.setGitStatus(data.status as GitStatus);
        setError(null);
        setResult(`${actionLabel(action)} completed`);
        setSelected(null);
        setDiff("");
        setPreviewTitle(null);
        return true;
      } catch (cause) {
        setError(cause instanceof Error ? cause.message : "Network error");
        return false;
      } finally {
        setBusy(false);
      }
    },
    [ide, workspace],
  );

  const fetchDiff = useCallback(
    async (entry: GitStatusEntry) => {
      setSelected(entry.path);
      setPreviewTitle(entry.path);
      setDiff("");
      setDiffLoading(true);
      try {
        if (entry.status === "untracked") {
          setDiff("");
          return;
        }
        const response = await fetch(
          `/api/git?workspace=${encodeURIComponent(workspace)}&diff=${encodeURIComponent(entry.path)}&staged=${entry.staged ? 1 : 0}`,
        );
        const data = await response.json().catch(() => ({}));
        if (!response.ok) throw new Error("Diff unavailable");
        setDiff(typeof data.diff === "string" ? data.diff : "");
      } catch {
        setDiff("");
      } finally {
        setDiffLoading(false);
      }
    },
    [workspace],
  );

  const fetchCommit = useCallback(
    async (oid: string, label: string) => {
      setSelected(oid);
      setPreviewTitle(label);
      setDiff("");
      setDiffLoading(true);
      setTab("history");
      try {
        const response = await fetch(
          `/api/git?workspace=${encodeURIComponent(workspace)}&commit=${encodeURIComponent(oid)}`,
        );
        const data = await response.json().catch(() => ({}));
        if (!response.ok) throw new Error(typeof data.error === "string" ? data.error : "Commit unavailable");
        setDiff(typeof data.patch === "string" ? data.patch : "");
      } catch (cause) {
        setError(cause instanceof Error ? cause.message : "Failed to load commit");
        setDiff("");
      } finally {
        setDiffLoading(false);
      }
    },
    [workspace],
  );

  const fetchStash = useCallback(
    async (ref: string) => {
      setSelected(ref);
      setPreviewTitle(ref);
      setDiff("");
      setDiffLoading(true);
      setTab("stashes");
      try {
        const response = await fetch(
          `/api/git?workspace=${encodeURIComponent(workspace)}&stash=${encodeURIComponent(ref)}`,
        );
        const data = await response.json().catch(() => ({}));
        if (!response.ok) throw new Error(typeof data.error === "string" ? data.error : "Stash unavailable");
        setDiff(typeof data.patch === "string" ? data.patch : "");
      } catch (cause) {
        setError(cause instanceof Error ? cause.message : "Failed to load stash");
        setDiff("");
      } finally {
        setDiffLoading(false);
      }
    },
    [workspace],
  );

  const totalChanges = status?.entries.length ?? 0;
  const stashCount = status?.stashes?.length ?? 0;
  const branchCount = status?.branches?.length ?? 0;
  const operations = status?.operations ?? [];

  if (compact) {
    if (!status || status.bare) {
      return (
        <span className="flex items-center gap-1 text-[12px] text-ink-500">
          <GitBranchIcon width={12} height={12} />
          no repo
        </span>
      );
    }
    return (
      <span className="flex items-center gap-1.5 text-[12px] text-ink-400">
        <GitBranchIcon width={12} height={12} />
        <span className="text-ink-200">{status.branch}</span>
        {status.ahead > 0 && <span className="text-emerald-400">↑{status.ahead}</span>}
        {status.behind > 0 && <span className="text-amber-300">↓{status.behind}</span>}
        {totalChanges > 0 && <span className="text-ink-500">• {totalChanges}</span>}
        {operations.length > 0 && <span className="text-red-400">• {operations[0]}</span>}
      </span>
    );
  }

  const tabs: Array<{ id: Tab; label: string; count?: number }> = [
    { id: "changes", label: "Changes", count: totalChanges || undefined },
    { id: "history", label: "History" },
    { id: "branches", label: "Branches", count: branchCount || undefined },
    { id: "stashes", label: "Stashes", count: stashCount || undefined },
    { id: "repository", label: "Repo" },
  ];

  return (
    <div className="flex h-full flex-col text-ink-200">
      <AppDialogHost dialog={dialog} />

      <div className="flex items-center justify-between border-b border-ink-800 px-3 py-2">
        <div className="min-w-0">
          <div className="text-[11px] font-semibold uppercase tracking-wider text-ink-400">
            Source Control
          </div>
          {status?.head && (
            <div className="mt-0.5 truncate font-mono text-[10px] text-ink-600" title={status.head.message}>
              {status.head.oid} · {status.head.message}
            </div>
          )}
        </div>
        <button
          type="button"
          onClick={() => void refresh()}
          disabled={loading || busy}
          title="Refresh all Git data"
          className="rounded-md p-1.5 text-ink-400 hover:bg-ink-800 hover:text-ink-100 disabled:opacity-40"
        >
          <RefreshIcon width={14} height={14} className={loading ? "animate-spin" : ""} />
        </button>
      </div>

      {error && (
        <Notice danger onClose={() => setError(null)}>
          {error}
        </Notice>
      )}
      {result && <Notice onClose={() => setResult(null)}>{result}</Notice>}

      {!status && loading && (
        <div className="px-3 py-4 text-[12px] text-ink-500">Loading repository…</div>
      )}

      {status?.bare && (
        <div className="flex flex-1 flex-col items-center justify-center gap-3 p-6 text-center">
          <div className="flex h-12 w-12 items-center justify-center rounded-xl bg-ink-850 text-ink-500">
            <GitBranchIcon width={22} height={22} />
          </div>
          <div>
            <p className="text-[13px] font-medium text-ink-100">Not a Git repository</p>
            <p className="mt-1 text-[12px] text-ink-500">Initialize to start tracking changes here.</p>
          </div>
          <button type="button" className={primaryClass} disabled={busy} onClick={() => void postAction("init")}>
            Initialize Repository
          </button>
        </div>
      )}

      {status && !status.bare && (
        <>
          <RepositoryHeader status={status} busy={busy} action={postAction} />

          {operations.length > 0 && (
            <OperationsBanner operations={operations} busy={busy} action={postAction} />
          )}

          <div className="flex overflow-x-auto border-b border-ink-800 px-1" role="tablist">
            {tabs.map((item) => (
              <button
                key={item.id}
                type="button"
                role="tab"
                aria-selected={tab === item.id}
                onClick={() => setTab(item.id)}
                className={`shrink-0 border-b-2 px-2.5 py-2 text-[10px] font-medium capitalize transition-colors ${
                  tab === item.id
                    ? "border-accent text-ink-100"
                    : "border-transparent text-ink-500 hover:text-ink-200"
                }`}
              >
                {item.label}
                {item.count != null ? (
                  <span className="ml-1 rounded bg-ink-800 px-1 py-0.5 text-[9px] text-ink-400">
                    {item.count}
                  </span>
                ) : null}
              </button>
            ))}
          </div>

          <div className="flex min-h-0 flex-1 flex-col">
            <div className="min-h-0 flex-1 overflow-y-auto">
              {tab === "changes" && (
                <Changes
                  status={status}
                  busy={busy}
                  action={postAction}
                  selected={selected}
                  fetchDiff={fetchDiff}
                  openFile={(path) => ide.openFile(path)}
                  confirm={confirm}
                />
              )}
              {tab === "history" && (
                <History
                  commits={status.commits ?? []}
                  busy={busy}
                  action={postAction}
                  confirm={confirm}
                  prompt={prompt}
                  onShow={(commit) => void fetchCommit(commit.oid, commit.shortOid)}
                  selected={selected}
                />
              )}
              {tab === "branches" && (
                <Branches
                  current={status.branch}
                  branches={status.branches ?? []}
                  busy={busy}
                  action={postAction}
                  confirm={confirm}
                  prompt={prompt}
                />
              )}
              {tab === "stashes" && (
                <Stashes
                  stashes={status.stashes ?? []}
                  busy={busy}
                  action={postAction}
                  confirm={confirm}
                  prompt={prompt}
                  onShow={(stash) => void fetchStash(stash.ref)}
                  selected={selected}
                />
              )}
              {tab === "repository" && (
                <Repository
                  status={status}
                  busy={busy}
                  action={postAction}
                  confirm={confirm}
                  prompt={prompt}
                />
              )}
            </div>

            {selected && previewTitle && (tab === "changes" || tab === "history" || tab === "stashes") && (
              <PreviewPane
                title={previewTitle}
                loading={diffLoading}
                diff={diff}
                onClose={() => {
                  setSelected(null);
                  setDiff("");
                  setPreviewTitle(null);
                }}
                onOpen={tab === "changes" ? () => ide.openFile(selected) : undefined}
              />
            )}
          </div>
        </>
      )}
    </div>
  );
}

function PreviewPane({
  title,
  loading,
  diff,
  onClose,
  onOpen,
}: {
  title: string;
  loading: boolean;
  diff: string;
  onClose: () => void;
  onOpen?: () => void;
}) {
  return (
    <div className="flex max-h-[42%] min-h-[8rem] flex-col border-t border-ink-800 bg-ink-950/40">
      <div className="flex items-center gap-2 border-b border-ink-800/80 px-3 py-1.5">
        <span className="min-w-0 flex-1 truncate font-mono text-[11px] text-ink-400">{title}</span>
        {onOpen && (
          <button type="button" className={buttonClass} onClick={onOpen}>
            Open
          </button>
        )}
        <button type="button" className={buttonClass} onClick={onClose} aria-label="Close preview">
          <XIcon width={12} height={12} />
        </button>
      </div>
      <div className="min-h-0 flex-1 overflow-auto px-2 py-2">
        {loading ? (
          <Empty>Loading…</Empty>
        ) : diff ? (
          <Diff diff={diff} />
        ) : (
          <Empty>No textual diff available. Open the file to inspect it.</Empty>
        )}
      </div>
    </div>
  );
}

function OperationsBanner({
  operations,
  busy,
  action,
}: {
  operations: GitOperation[];
  busy: boolean;
  action: Action;
}) {
  return (
    <div className="border-b border-warning/30 bg-warning/10 px-3 py-2">
      <div className="text-[11px] font-semibold text-warning">
        In progress: {operations.join(", ")}
      </div>
      <div className="mt-1.5 flex flex-wrap gap-1">
        {operations.map((operation) => (
          <span key={operation} className="flex gap-1">
            <button
              type="button"
              className={primaryClass}
              disabled={busy}
              onClick={() => void action("continue", { operation })}
            >
              Continue {operation}
            </button>
            <button
              type="button"
              className={buttonClass}
              disabled={busy}
              onClick={() => void action("abort", { operation })}
            >
              Abort
            </button>
          </span>
        ))}
      </div>
    </div>
  );
}

function RepositoryHeader({
  status,
  busy,
  action,
}: {
  status: GitStatus;
  busy: boolean;
  action: Action;
}) {
  const [pullMode, setPullMode] = useState("--no-rebase");
  return (
    <div className="border-b border-ink-800 px-3 py-2.5 text-[12px]">
      <div className="flex flex-wrap items-center gap-2">
        <span className="flex h-6 w-6 items-center justify-center rounded-md bg-accent/15 text-accent-soft">
          <GitBranchIcon width={13} height={13} />
        </span>
        <span className="min-w-0 truncate font-medium text-ink-100">{status.branch}</span>
        {status.ahead > 0 && <span className="rounded bg-emerald-400/10 px-1.5 py-0.5 text-[10px] text-emerald-400">↑{status.ahead}</span>}
        {status.behind > 0 && <span className="rounded bg-amber-300/10 px-1.5 py-0.5 text-[10px] text-amber-300">↓{status.behind}</span>}
        <span className="ml-auto truncate text-[10px] text-ink-600" title={status.upstream ?? "No upstream"}>
          {status.upstream ?? "no upstream"}
        </span>
      </div>
      <div className="mt-2 flex flex-wrap gap-1">
        <button type="button" className={buttonClass} disabled={busy} onClick={() => void action("fetch", { prune: true })}>
          Fetch
        </button>
        <button
          type="button"
          className={buttonClass}
          disabled={busy}
          onClick={() => void action("pull", { mode: pullMode })}
        >
          Pull{status.behind > 0 ? ` ${status.behind}` : ""}
        </button>
        <select
          value={pullMode}
          onChange={(event) => setPullMode(event.target.value)}
          className={`${inputClass} w-auto py-1`}
          aria-label="Pull strategy"
        >
          <option value="--no-rebase">merge</option>
          <option value="--rebase">rebase</option>
          <option value="--ff-only">ff-only</option>
        </select>
        <button type="button" className={buttonClass} disabled={busy} onClick={() => void action("push")}>
          Push{status.ahead > 0 ? ` ${status.ahead}` : ""}
        </button>
      </div>
    </div>
  );
}

function Changes({
  status,
  busy,
  action,
  selected,
  fetchDiff,
  openFile,
  confirm,
}: {
  status: GitStatus;
  busy: boolean;
  action: Action;
  selected: string | null;
  fetchDiff: (entry: GitStatusEntry) => void;
  openFile: (path: string) => void;
  confirm: ConfirmFn;
}) {
  const groups = useMemo(() => groupEntries(status), [status]);
  const [message, setMessage] = useState("");
  const [amend, setAmend] = useState(false);
  const [signoff, setSignoff] = useState(false);
  const [noVerify, setNoVerify] = useState(false);

  const commit = async (andPush = false) => {
    const ok = await action("commit", { message, amend, signoff, noVerify });
    if (!ok) return;
    setMessage("");
    if (andPush) await action("push");
  };

  const canCommit =
    (Boolean(message.trim()) || amend) && (groups.staged.length > 0 || amend) && !busy;

  return (
    <>
      <div className="border-b border-ink-800 p-3">
        <textarea
          className={`${inputClass} w-full resize-y`}
          rows={3}
          value={message}
          onChange={(event) => setMessage(event.target.value)}
          onKeyDown={(event) => {
            if ((event.ctrlKey || event.metaKey) && event.key === "Enter" && canCommit) {
              event.preventDefault();
              void commit(false);
            }
          }}
          placeholder={
            amend
              ? "Optional new message (blank keeps previous)"
              : "Commit message (Ctrl/Cmd+Enter)"
          }
        />
        <div className="my-2 flex flex-wrap gap-3">
          <Check label="Amend" value={amend} set={setAmend} />
          <Check label="Sign-off" value={signoff} set={setSignoff} />
          <Check label="Skip hooks" value={noVerify} set={setNoVerify} />
        </div>
        <div className="flex gap-1.5">
          <button
            type="button"
            className={`${primaryClass} flex flex-1 items-center justify-center gap-1`}
            disabled={!canCommit}
            onClick={() => void commit(false)}
          >
            <CheckIcon width={13} height={13} />
            {amend ? "Amend" : `Commit (${groups.staged.length})`}
          </button>
          <button
            type="button"
            className={buttonClass}
            disabled={!canCommit}
            onClick={() => void commit(true)}
            title="Commit then push"
          >
            & Push
          </button>
        </div>
      </div>

      <div className="flex flex-wrap gap-1 border-b border-ink-800 p-2">
        <button
          type="button"
          className={buttonClass}
          disabled={busy || status.entries.length === 0}
          onClick={() => void action("stashPush", { includeUntracked: true })}
        >
          Stash all
        </button>
        <button
          type="button"
          className={buttonClass}
          disabled={busy || groups.unstaged.length === 0}
          onClick={async () => {
            if (await confirm({ title: "Discard tracked changes?", message: "Discard every tracked worktree change? This cannot be undone.", confirmLabel: "Discard", danger: true })) {
              await action("discardAll");
            }
          }}
        >
          Discard tracked
        </button>
        <button
          type="button"
          className={`${buttonClass} hover:text-red-400`}
          disabled={busy || groups.untracked.length === 0}
          onClick={async () => {
            if (await confirm({ title: "Clean untracked files?", message: "Permanently delete all untracked files and folders?", confirmLabel: "Clean", danger: true })) {
              await action("clean");
            }
          }}
        >
          Clean untracked
        </button>
      </div>

      <ChangeGroup
        title="Merge Conflicts"
        entries={groups.conflicted}
        actionLabel="Stage all"
        onAction={() => void action("stageAll")}
        busy={busy}
        accent="danger"
      >
        {(entry) => (
          <FileRow
            key={`x-${entry.path}`}
            entry={entry}
            active={selected === entry.path}
            onClick={() => void fetchDiff(entry)}
          >
            <IconButton title="Open" disabled={busy} onClick={() => openFile(entry.path)}>
              <FileIcon width={13} height={13} />
            </IconButton>
            <IconButton title="Mark resolved (stage)" disabled={busy} onClick={() => void action("stage", { path: entry.path })}>
              <PlusIcon width={13} height={13} />
            </IconButton>
          </FileRow>
        )}
      </ChangeGroup>

      <ChangeGroup
        title="Staged Changes"
        entries={groups.staged}
        actionLabel="Unstage all"
        onAction={() => void action("unstageAll")}
        busy={busy}
      >
        {(entry) => (
          <FileRow
            key={`s-${entry.path}`}
            entry={entry}
            active={selected === entry.path}
            onClick={() => void fetchDiff(entry)}
          >
            <IconButton title="Open" disabled={busy || entry.status === "deleted"} onClick={() => openFile(entry.path)}>
              <FileIcon width={13} height={13} />
            </IconButton>
            <IconButton title="Unstage" disabled={busy} onClick={() => void action("unstage", { path: entry.path })}>
              <MinusIcon width={13} height={13} />
            </IconButton>
          </FileRow>
        )}
      </ChangeGroup>

      <ChangeGroup
        title="Changes"
        entries={groups.unstaged}
        actionLabel="Stage all"
        onAction={() => void action("stageAll")}
        busy={busy}
      >
        {(entry) => (
          <FileRow
            key={`c-${entry.path}`}
            entry={entry}
            active={selected === entry.path}
            onClick={() => void fetchDiff(entry)}
          >
            <IconButton title="Open" disabled={busy || entry.status === "deleted"} onClick={() => openFile(entry.path)}>
              <FileIcon width={13} height={13} />
            </IconButton>
            <IconButton title="Stage" disabled={busy} onClick={() => void action("stage", { path: entry.path })}>
              <PlusIcon width={13} height={13} />
            </IconButton>
            <IconButton
              title="Discard"
              danger
              disabled={busy}
              onClick={async () => {
                if (await confirm({ title: "Discard changes?", message: `Discard changes to ${entry.path}?`, confirmLabel: "Discard", danger: true })) {
                  await action("discard", { path: entry.path });
                }
              }}
            >
              <TrashIcon width={13} height={13} />
            </IconButton>
          </FileRow>
        )}
      </ChangeGroup>

      <ChangeGroup
        title="Untracked"
        entries={groups.untracked}
        actionLabel="Stage all"
        onAction={() => void action("stageAll")}
        busy={busy}
      >
        {(entry) => (
          <FileRow
            key={`u-${entry.path}`}
            entry={entry}
            active={selected === entry.path}
            onClick={() => void fetchDiff(entry)}
          >
            <IconButton title="Open" disabled={busy} onClick={() => openFile(entry.path)}>
              <FileIcon width={13} height={13} />
            </IconButton>
            <IconButton title="Stage" disabled={busy} onClick={() => void action("stage", { path: entry.path })}>
              <PlusIcon width={13} height={13} />
            </IconButton>
            <IconButton
              title="Add to .gitignore"
              disabled={busy}
              onClick={() => void action("ignore", { path: entry.path })}
            >
              <MinusIcon width={13} height={13} />
            </IconButton>
          </FileRow>
        )}
      </ChangeGroup>

      {status.entries.length === 0 && <Empty>Working tree clean.</Empty>}
    </>
  );
}

function History({
  commits,
  busy,
  action,
  confirm,
  prompt,
  onShow,
  selected,
}: {
  commits: GitCommit[];
  busy: boolean;
  action: Action;
  confirm: ConfirmFn;
  prompt: PromptFn;
  onShow: (commit: GitCommit) => void;
  selected: string | null;
}) {
  const [query, setQuery] = useState("");
  const visible = commits.filter((commit) =>
    `${commit.subject} ${commit.author} ${commit.oid} ${commit.refs.join(" ")}`
      .toLowerCase()
      .includes(query.toLowerCase()),
  );

  const reset = async (commit: GitCommit) => {
    const mode = await prompt({
      title: "Reset to commit",
      message: `Reset HEAD to ${commit.shortOid}. Choose mode: soft, mixed, or hard.`,
      defaultValue: "mixed",
      placeholder: "soft | mixed | hard",
      confirmLabel: "Reset",
      required: true,
    });
    if (!mode) return;
    const normalized = mode.trim().toLowerCase();
    if (!["soft", "mixed", "hard"].includes(normalized)) return;
    if (normalized === "hard") {
      const ok = await confirm({
        title: "Hard reset?",
        message: "Hard reset discards local changes permanently. Continue?",
        confirmLabel: "Hard reset",
        danger: true,
      });
      if (!ok) return;
    }
    await action("reset", { ref: commit.oid, mode: normalized });
  };

  return (
    <>
      <div className="border-b border-ink-800 p-2">
        <input
          className={`${inputClass} w-full`}
          value={query}
          onChange={(event) => setQuery(event.target.value)}
          placeholder="Filter by message, author, SHA, or ref"
        />
      </div>
      {visible.map((commit) => (
        <div
          key={commit.oid}
          className={`group border-b border-ink-800/70 px-3 py-2 ${selected === commit.oid ? "bg-ink-850/80" : ""}`}
        >
          <button type="button" className="flex w-full items-start gap-2 text-left" onClick={() => onShow(commit)}>
            <span className="font-mono text-[10px] text-accent-soft">{commit.shortOid}</span>
            <span className="min-w-0 flex-1">
              <span className="block truncate text-[12px] text-ink-100" title={commit.subject}>
                {commit.subject}
              </span>
              <span className="mt-0.5 block truncate text-[10px] text-ink-500">
                {commit.author} · {formatDate(commit.ts)}
              </span>
              {commit.refs.length > 0 && (
                <span className="mt-1 flex flex-wrap gap-1">
                  {commit.refs.map((ref) => (
                    <span key={ref} className="rounded bg-ink-800 px-1 text-[9px] text-sky-300">
                      {ref}
                    </span>
                  ))}
                </span>
              )}
            </span>
          </button>
          <div className="mt-1.5 flex flex-wrap gap-1 opacity-80 group-hover:opacity-100">
            <button type="button" className={buttonClass} disabled={busy} onClick={() => onShow(commit)}>
              View
            </button>
            <button
              type="button"
              className={buttonClass}
              disabled={busy}
              onClick={() => void copyText(commit.oid).then(() => undefined)}
              title="Copy full SHA"
            >
              <CopyIcon width={12} height={12} />
            </button>
            <button
              type="button"
              className={buttonClass}
              disabled={busy}
              onClick={() => void action("checkout", { branch: commit.oid })}
            >
              Checkout
            </button>
            <button
              type="button"
              className={buttonClass}
              disabled={busy}
              onClick={() => void action("cherryPick", { ref: commit.oid })}
            >
              Cherry-pick
            </button>
            <button
              type="button"
              className={buttonClass}
              disabled={busy}
              onClick={async () => {
                if (await confirm({ title: "Revert commit?", message: `Create a revert commit for ${commit.shortOid}?`, confirmLabel: "Revert" })) {
                  await action("revert", { ref: commit.oid });
                }
              }}
            >
              Revert
            </button>
            <button type="button" className={buttonClass} disabled={busy} onClick={() => void reset(commit)}>
              Reset…
            </button>
          </div>
        </div>
      ))}
      {visible.length === 0 && <Empty>No commits match.</Empty>}
    </>
  );
}

function Branches({
  current,
  branches,
  busy,
  action,
  confirm,
  prompt,
}: {
  current: string;
  branches: GitBranch[];
  busy: boolean;
  action: Action;
  confirm: ConfirmFn;
  prompt: PromptFn;
}) {
  const [name, setName] = useState("");
  const [startPoint, setStartPoint] = useState("");
  const [checkout, setCheckout] = useState(true);
  const [filter, setFilter] = useState("");
  const q = filter.trim().toLowerCase();
  const local = branches.filter((branch) => !branch.remote && (!q || branch.name.toLowerCase().includes(q)));
  const remote = branches.filter((branch) => branch.remote && (!q || branch.name.toLowerCase().includes(q)));

  const create = async () => {
    if (
      await action("createBranch", {
        branch: name,
        startPoint: startPoint || undefined,
        checkout,
      })
    ) {
      setName("");
      setStartPoint("");
    }
  };

  return (
    <>
      <Section title="Create branch">
        <div className="grid grid-cols-2 gap-1.5">
          <input
            className={inputClass}
            value={name}
            onChange={(event) => setName(event.target.value)}
            placeholder="branch name"
          />
          <input
            className={inputClass}
            value={startPoint}
            onChange={(event) => setStartPoint(event.target.value)}
            placeholder="start point (HEAD)"
          />
        </div>
        <div className="mt-2 flex items-center justify-between gap-2">
          <Check label="Switch after creation" value={checkout} set={setCheckout} />
          <button type="button" className={primaryClass} disabled={busy || !name.trim()} onClick={() => void create()}>
            Create
          </button>
        </div>
      </Section>
      <div className="border-b border-ink-800 px-3 py-2">
        <input
          className={`${inputClass} w-full`}
          value={filter}
          onChange={(event) => setFilter(event.target.value)}
          placeholder="Filter branches…"
        />
      </div>
      <BranchList
        title="Local branches"
        branches={local}
        current={current}
        busy={busy}
        action={action}
        confirm={confirm}
        prompt={prompt}
      />
      <BranchList
        title="Remote branches"
        branches={remote}
        current={current}
        busy={busy}
        action={action}
        confirm={confirm}
        prompt={prompt}
      />
    </>
  );
}

function BranchList({
  title,
  branches,
  current,
  busy,
  action,
  confirm,
  prompt,
}: {
  title: string;
  branches: GitBranch[];
  current: string;
  busy: boolean;
  action: Action;
  confirm: ConfirmFn;
  prompt: PromptFn;
}) {
  return (
    <Section title={`${title} (${branches.length})`}>
      {branches.length === 0 && <Empty>No branches.</Empty>}
      {branches.map((branch) => (
        <div key={`${branch.remote}-${branch.name}`} className="group border-t border-ink-800/70 py-2 first:border-0">
          <div className="flex items-center gap-2">
            <GitBranchIcon
              width={12}
              height={12}
              className={branch.current ? "text-accent" : "text-ink-500"}
            />
            <span
              className={`min-w-0 flex-1 truncate text-[12px] ${
                branch.current ? "font-medium text-ink-100" : "text-ink-300"
              }`}
            >
              {branch.name}
            </span>
            <span className="font-mono text-[9px] text-ink-600">{branch.oid}</span>
          </div>
          {branch.upstream && (
            <div className="ml-5 truncate text-[10px] text-ink-600">
              ↳ {branch.upstream} {branch.ahead ? `↑${branch.ahead}` : ""}{" "}
              {branch.behind ? `↓${branch.behind}` : ""}
            </div>
          )}
          <div className="ml-5 mt-1.5 flex flex-wrap gap-1 opacity-80 group-hover:opacity-100">
            {!branch.current && (
              <button
                type="button"
                className={buttonClass}
                disabled={busy}
                onClick={() => void action("checkout", { branch: branch.name })}
              >
                Switch
              </button>
            )}
            <button
              type="button"
              className={buttonClass}
              disabled={busy || branch.name === current}
              onClick={() => void action("merge", { ref: branch.name })}
            >
              Merge
            </button>
            <button
              type="button"
              className={buttonClass}
              disabled={busy || branch.name === current}
              onClick={() => void action("merge", { ref: branch.name, squash: true })}
            >
              Squash
            </button>
            <button
              type="button"
              className={buttonClass}
              disabled={busy || branch.name === current}
              onClick={() => void action("merge", { ref: branch.name, noFf: true })}
            >
              No-FF
            </button>
            <button
              type="button"
              className={buttonClass}
              disabled={busy || branch.name === current}
              onClick={() => void action("rebase", { ref: branch.name })}
            >
              Rebase onto
            </button>
            {!branch.remote && !branch.current && (
              <>
                <button
                  type="button"
                  className={`${buttonClass} hover:text-red-400`}
                  disabled={busy}
                  onClick={async () => {
                    if (await confirm({ title: "Delete branch?", message: `Delete branch ${branch.name}?`, confirmLabel: "Delete", danger: true })) {
                      await action("deleteBranch", { branch: branch.name });
                    }
                  }}
                >
                  Delete
                </button>
                <button
                  type="button"
                  className={`${buttonClass} hover:text-red-400`}
                  disabled={busy}
                  onClick={async () => {
                    if (await confirm({ title: "Force-delete branch?", message: `Force-delete unmerged branch ${branch.name}?`, confirmLabel: "Force delete", danger: true })) {
                      await action("deleteBranch", { branch: branch.name, force: true });
                    }
                  }}
                >
                  Force delete
                </button>
              </>
            )}
            {!branch.remote && (
              <button
                type="button"
                className={buttonClass}
                disabled={busy}
                onClick={async () => {
                  const next = await prompt({
                    title: "Rename branch",
                    defaultValue: branch.name,
                    placeholder: "New branch name",
                    confirmLabel: "Rename",
                    required: true,
                  });
                  if (next) await action("renameBranch", { branch: branch.name, newName: next.trim() });
                }}
              >
                Rename
              </button>
            )}
          </div>
        </div>
      ))}
    </Section>
  );
}

function Stashes({
  stashes,
  busy,
  action,
  confirm,
  prompt,
  onShow,
  selected,
}: {
  stashes: GitStash[];
  busy: boolean;
  action: Action;
  confirm: ConfirmFn;
  prompt: PromptFn;
  onShow: (stash: GitStash) => void;
  selected: string | null;
}) {
  const [message, setMessage] = useState("");
  const [includeUntracked, setIncludeUntracked] = useState(true);
  const [keepIndex, setKeepIndex] = useState(false);

  return (
    <>
      <Section title="Save working changes">
        <input
          className={`${inputClass} w-full`}
          value={message}
          onChange={(event) => setMessage(event.target.value)}
          placeholder="Optional stash message"
        />
        <div className="mt-2 flex flex-wrap items-center gap-3">
          <Check label="Include untracked" value={includeUntracked} set={setIncludeUntracked} />
          <Check label="Keep staged" value={keepIndex} set={setKeepIndex} />
          <button
            type="button"
            className={`${primaryClass} ml-auto`}
            disabled={busy}
            onClick={async () => {
              if (await action("stashPush", { message, includeUntracked, keepIndex })) setMessage("");
            }}
          >
            Stash
          </button>
        </div>
      </Section>
      {stashes.map((stash) => (
        <div
          key={stash.ref}
          className={`border-b border-ink-800 px-3 py-2 ${selected === stash.ref ? "bg-ink-850/80" : ""}`}
        >
          <button type="button" className="flex w-full gap-2 text-left" onClick={() => onShow(stash)}>
            <span className="font-mono text-[10px] text-violet-300">{stash.ref}</span>
            <span className="min-w-0 flex-1 truncate text-[12px] text-ink-200">{stash.subject}</span>
          </button>
          <div className="mt-0.5 text-[10px] text-ink-600">{formatDate(stash.ts)}</div>
          <div className="mt-1.5 flex flex-wrap gap-1">
            <button type="button" className={buttonClass} disabled={busy} onClick={() => onShow(stash)}>
              View
            </button>
            <button
              type="button"
              className={buttonClass}
              disabled={busy}
              onClick={() => void action("stashApply", { ref: stash.ref })}
            >
              Apply
            </button>
            <button
              type="button"
              className={buttonClass}
              disabled={busy}
              onClick={() => void action("stashApply", { ref: stash.ref, pop: true })}
            >
              Pop
            </button>
            <button
              type="button"
              className={buttonClass}
              disabled={busy}
              onClick={async () => {
                const branch = await prompt({
                  title: "Create branch from stash",
                  placeholder: "New branch name",
                  confirmLabel: "Create",
                  required: true,
                });
                if (branch) await action("stashBranch", { ref: stash.ref, branch: branch.trim() });
              }}
            >
              Create branch
            </button>
            <button
              type="button"
              className={`${buttonClass} hover:text-red-400`}
              disabled={busy}
              onClick={async () => {
                if (await confirm({ title: "Drop stash?", message: `Drop ${stash.ref}?`, confirmLabel: "Drop", danger: true })) {
                  await action("stashDrop", { ref: stash.ref });
                }
              }}
            >
              Drop
            </button>
          </div>
        </div>
      ))}
      {stashes.length === 0 && <Empty>No stashes.</Empty>}
    </>
  );
}

function Repository({
  status,
  busy,
  action,
  confirm,
  prompt,
}: {
  status: GitStatus;
  busy: boolean;
  action: Action;
  confirm: ConfirmFn;
  prompt: PromptFn;
}) {
  return (
    <>
      <Tags tags={status.tags ?? []} busy={busy} action={action} confirm={confirm} />
      <Remotes
        remotes={status.remotes ?? []}
        current={status.branch}
        busy={busy}
        action={action}
        confirm={confirm}
        prompt={prompt}
      />
      <Section title="Sync & recovery">
        <div className="flex flex-wrap gap-1">
          <button
            type="button"
            className={buttonClass}
            disabled={busy}
            onClick={() => void action("fetch", { all: true, prune: true })}
          >
            Fetch all + prune
          </button>
          <button
            type="button"
            className={buttonClass}
            disabled={busy}
            onClick={() => void action("push", { tags: true })}
          >
            Push tags
          </button>
          <button
            type="button"
            className={buttonClass}
            disabled={busy}
            onClick={async () => {
              if (await confirm({ title: "Force-push with lease?", message: "Force-push with lease? Remote history may change for anyone who pulled.", confirmLabel: "Force push", danger: true })) {
                await action("push", { forceWithLease: true });
              }
            }}
          >
            Force push with lease
          </button>
          <button
            type="button"
            className={buttonClass}
            disabled={busy}
            onClick={async () => {
              const upstream = await prompt({
                title: "Set upstream",
                message: "Example: origin/main",
                placeholder: "origin/main",
                confirmLabel: "Set",
                required: true,
              });
              if (upstream) await action("setUpstream", { branch: status.branch, upstream: upstream.trim() });
            }}
          >
            Set upstream
          </button>
          {status.upstream && (
            <button
              type="button"
              className={buttonClass}
              disabled={busy}
              onClick={() => void action("unsetUpstream", { branch: status.branch })}
            >
              Unset upstream
            </button>
          )}
        </div>
        {(status.operations?.length ?? 0) === 0 && (
          <>
            <div className="mt-3 text-[10px] font-semibold uppercase tracking-wider text-ink-500">
              Manual recovery
            </div>
            <div className="mt-1 grid grid-cols-2 gap-1">
              {(["merge", "rebase", "cherry-pick", "revert"] as const).flatMap((operation) => [
                <button
                  key={`${operation}-continue`}
                  type="button"
                  className={buttonClass}
                  disabled={busy}
                  onClick={() => void action("continue", { operation })}
                >
                  Continue {operation}
                </button>,
                <button
                  key={`${operation}-abort`}
                  type="button"
                  className={buttonClass}
                  disabled={busy}
                  onClick={() => void action("abort", { operation })}
                >
                  Abort {operation}
                </button>,
              ])}
            </div>
          </>
        )}
      </Section>
      <Section title="Danger zone">
        <button
          type="button"
          className={`${buttonClass} hover:text-red-400`}
          disabled={busy}
          onClick={async () => {
            if (await confirm({ title: "Clean ignored files?", message: "Delete untracked and ignored files? This permanently removes build output and all ignored files.", confirmLabel: "Clean all", danger: true })) {
              await action("clean", { includeIgnored: true });
            }
          }}
        >
          Clean untracked + ignored files
        </button>
      </Section>
    </>
  );
}

function Tags({
  tags,
  busy,
  action,
  confirm,
}: {
  tags: GitTag[];
  busy: boolean;
  action: Action;
  confirm: ConfirmFn;
}) {
  const [tag, setTag] = useState("");
  const [ref, setRef] = useState("");
  const [message, setMessage] = useState("");
  const annotated = Boolean(message.trim());

  return (
    <Section title={`Tags (${tags.length})`}>
      <div className="grid grid-cols-2 gap-1.5">
        <input
          className={inputClass}
          value={tag}
          onChange={(event) => setTag(event.target.value)}
          placeholder="tag name"
        />
        <input
          className={inputClass}
          value={ref}
          onChange={(event) => setRef(event.target.value)}
          placeholder="target (HEAD)"
        />
      </div>
      <input
        className={`${inputClass} mt-1.5 w-full`}
        value={message}
        onChange={(event) => setMessage(event.target.value)}
        placeholder="Annotation (blank creates lightweight tag)"
      />
      <button
        type="button"
        className={`${primaryClass} mt-2`}
        disabled={busy || !tag.trim()}
        onClick={() =>
          void action("tagCreate", { tag, ref: ref || undefined, message, annotated })
        }
      >
        Create tag
      </button>
      <div className="mt-2 max-h-48 overflow-y-auto">
        {tags.map((item) => (
          <div key={item.name} className="group flex items-center gap-2 border-t border-ink-800/70 py-1.5">
            <span className="min-w-0 flex-1 truncate text-[12px] text-ink-200" title={item.subject}>
              {item.name}
            </span>
            <span className="font-mono text-[9px] text-ink-600">{item.oid}</span>
            <button
              type="button"
              className={buttonClass}
              disabled={busy}
              onClick={() => void action("tagPush", { tag: item.name })}
            >
              Push
            </button>
            <button
              type="button"
              className={`${buttonClass} hover:text-red-400`}
              disabled={busy}
              onClick={async () => {
                if (await confirm({ title: "Delete tag?", message: `Delete local tag ${item.name}?`, confirmLabel: "Delete", danger: true })) {
                  await action("tagDelete", { tag: item.name });
                }
              }}
            >
              Delete
            </button>
          </div>
        ))}
      </div>
    </Section>
  );
}

function Remotes({
  remotes,
  current,
  busy,
  action,
  confirm,
  prompt,
}: {
  remotes: GitRemote[];
  current: string;
  busy: boolean;
  action: Action;
  confirm: ConfirmFn;
  prompt: PromptFn;
}) {
  const [name, setName] = useState("");
  const [url, setUrl] = useState("");

  return (
    <Section title={`Remotes (${remotes.length})`}>
      <div className="flex gap-1.5">
        <input
          className={`${inputClass} w-24`}
          value={name}
          onChange={(event) => setName(event.target.value)}
          placeholder="name"
        />
        <input
          className={`${inputClass} flex-1`}
          value={url}
          onChange={(event) => setUrl(event.target.value)}
          placeholder="URL"
        />
        <button
          type="button"
          className={primaryClass}
          disabled={busy || !name.trim() || !url.trim()}
          onClick={async () => {
            if (await action("remoteAdd", { remote: name, url })) {
              setName("");
              setUrl("");
            }
          }}
        >
          Add
        </button>
      </div>
      {remotes.map((remote) => (
        <div key={remote.name} className="border-t border-ink-800/70 py-2 first:mt-2">
          <div className="flex items-center gap-2">
            <span className="text-[12px] font-medium text-ink-100">{remote.name}</span>
            <span className="min-w-0 flex-1 truncate text-[10px] text-ink-500" title={remote.fetchUrl}>
              {remote.fetchUrl}
            </span>
          </div>
          <div className="mt-1.5 flex flex-wrap gap-1">
            <button
              type="button"
              className={buttonClass}
              disabled={busy}
              onClick={() =>
                void action("push", { setUpstream: true, remote: remote.name, branch: current })
              }
            >
              Publish branch
            </button>
            <button
              type="button"
              className={buttonClass}
              disabled={busy}
              onClick={async () => {
                const next = await prompt({
                  title: "Change remote URL",
                  defaultValue: remote.fetchUrl,
                  placeholder: "git@… or https://…",
                  confirmLabel: "Update",
                  required: true,
                });
                if (next) await action("remoteSetUrl", { remote: remote.name, url: next.trim() });
              }}
            >
              Change URL
            </button>
            <button
              type="button"
              className={buttonClass}
              disabled={busy}
              onClick={async () => {
                const next = await prompt({
                  title: "Rename remote",
                  defaultValue: remote.name,
                  placeholder: "New remote name",
                  confirmLabel: "Rename",
                  required: true,
                });
                if (next) await action("remoteRename", { remote: remote.name, newName: next.trim() });
              }}
            >
              Rename
            </button>
            <button
              type="button"
              className={`${buttonClass} hover:text-red-400`}
              disabled={busy}
              onClick={async () => {
                if (await confirm({ title: "Remove remote?", message: `Remove remote ${remote.name}?`, confirmLabel: "Remove", danger: true })) {
                  await action("remoteRemove", { remote: remote.name });
                }
              }}
            >
              Remove
            </button>
          </div>
        </div>
      ))}
    </Section>
  );
}

function ChangeGroup({
  title,
  entries,
  actionLabel,
  onAction,
  busy,
  children,
  accent,
}: {
  title: string;
  entries: GitStatusEntry[];
  actionLabel: string;
  onAction: () => void;
  busy: boolean;
  children: (entry: GitStatusEntry) => ReactNode;
  accent?: "danger";
}) {
  if (!entries.length) return null;
  return (
    <div className={`border-b border-ink-800 ${accent === "danger" ? "bg-red-400/5" : ""}`}>
      <div className="flex items-center justify-between px-3 py-1.5">
        <span
          className={`text-[11px] font-semibold uppercase tracking-wider ${
            accent === "danger" ? "text-red-300" : "text-ink-400"
          }`}
        >
          {title} <span className="text-ink-600">({entries.length})</span>
        </span>
        <button
          type="button"
          className="text-[11px] text-ink-500 hover:text-ink-100 disabled:opacity-40"
          onClick={onAction}
          disabled={busy}
        >
          {actionLabel}
        </button>
      </div>
      {entries.map(children)}
    </div>
  );
}

function FileRow({
  entry,
  active,
  onClick,
  children,
}: {
  entry: GitStatusEntry;
  active: boolean;
  onClick: () => void;
  children: ReactNode;
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
        type="button"
        onClick={onClick}
        title={entry.path}
        className="min-w-0 flex-1 truncate text-left text-ink-200 hover:text-ink-100"
      >
        {entry.path}
        {entry.oldPath && <span className="text-ink-600"> ← {entry.oldPath}</span>}
      </button>
      <div className="flex items-center gap-0.5 opacity-100 transition-opacity sm:opacity-0 sm:group-hover:opacity-100 sm:focus-within:opacity-100">
        {children}
      </div>
    </div>
  );
}

function IconButton({
  title,
  onClick,
  disabled,
  danger,
  children,
}: {
  title: string;
  onClick: () => void;
  disabled: boolean;
  danger?: boolean;
  children: ReactNode;
}) {
  return (
    <button
      type="button"
      title={title}
      aria-label={title}
      onClick={(event) => {
        event.stopPropagation();
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

function Check({
  label,
  value,
  set,
}: {
  label: string;
  value: boolean;
  set: (value: boolean) => void;
}) {
  return (
    <label className="flex items-center gap-1.5 text-[10px] text-ink-400">
      <input
        type="checkbox"
        checked={value}
        onChange={(event) => set(event.target.checked)}
        className="accent-accent"
      />
      {label}
    </label>
  );
}

function Section({ title, children }: { title: string; children: ReactNode }) {
  return (
    <section className="border-b border-ink-800 p-3">
      <h3 className="mb-2 text-[11px] font-semibold uppercase tracking-wider text-ink-400">{title}</h3>
      {children}
    </section>
  );
}

function Empty({ children }: { children: ReactNode }) {
  return <div className="px-3 py-6 text-center text-[12px] text-ink-500">{children}</div>;
}

function Notice({
  children,
  danger,
  onClose,
}: {
  children: ReactNode;
  danger?: boolean;
  onClose: () => void;
}) {
  return (
    <div
      className={`m-2 flex items-start gap-2 rounded-lg border px-2.5 py-1.5 text-[11px] ${
        danger
          ? "border-red-400/30 bg-red-400/10 text-red-300"
          : "border-emerald-400/20 bg-emerald-400/10 text-emerald-300"
      }`}
    >
      <span className="min-w-0 flex-1 break-words">{children}</span>
      <button type="button" onClick={onClose} aria-label="Dismiss" className="opacity-70 hover:opacity-100">
        ×
      </button>
    </div>
  );
}
