"use client";

import { useCallback, useEffect, useMemo, useState, type ReactNode } from "react";
import { Diff } from "@/components/diff";
import { CheckIcon, GitBranchIcon, MinusIcon, PlusIcon, RefreshIcon, TrashIcon } from "@/components/icons";
import { useIdeContext } from "@/lib/ide-context";
import type { GitBranch, GitCommit, GitRemote, GitStash, GitStatus, GitStatusEntry, GitTag } from "@/lib/types";

type Tab = "changes" | "history" | "branches" | "stashes" | "repository";
type Action = (action: string, extra?: Record<string, unknown>) => Promise<boolean>;

const inputClass = "min-w-0 rounded border border-ink-700 bg-ink-950 px-2 py-1 text-[12px] text-ink-100 outline-none placeholder:text-ink-600 focus:border-accent";
const buttonClass = "rounded border border-ink-700 px-2 py-1 text-[11px] text-ink-300 hover:border-ink-600 hover:bg-ink-800 hover:text-ink-100 disabled:opacity-40";
const primaryClass = "rounded bg-accent px-2 py-1 text-[11px] font-medium text-ink-950 hover:bg-accent-soft disabled:opacity-40";

function statusLetter(status: GitStatusEntry["status"]) {
  return ({ modified: "M", added: "A", deleted: "D", renamed: "R", conflicted: "!", untracked: "U" } as const)[status];
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
    staged: entries.filter((entry) => entry.staged && entry.status !== "untracked"),
    unstaged: entries.filter((entry) => !entry.staged && entry.status !== "untracked"),
    untracked: entries.filter((entry) => entry.status === "untracked"),
  };
}

export function GitPanel({ compact }: { compact?: boolean }) {
  const { workspace, ide } = useIdeContext();
  const status = ide.state.gitStatus;
  const [tab, setTab] = useState<Tab>("changes");
  const [loading, setLoading] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [result, setResult] = useState<string | null>(null);
  const [selected, setSelected] = useState<string | null>(null);
  const [diff, setDiff] = useState("");
  const [diffLoading, setDiffLoading] = useState(false);

  const refresh = useCallback(async (silent = false) => {
    if (!silent) setLoading(true);
    try {
      const response = await fetch(`/api/git?workspace=${encodeURIComponent(workspace)}`);
      const data = await response.json().catch(() => ({}));
      if (!response.ok) throw new Error(typeof data.error === "string" ? data.error : "Failed to load Git repository");
      ide.setGitStatus(data as GitStatus);
      setError(null);
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : "Network error");
    } finally {
      if (!silent) setLoading(false);
    }
  }, [ide, workspace]);

  useEffect(() => {
    refresh();
    const interval = window.setInterval(() => document.visibilityState === "visible" && refresh(true), 10000);
    const onFocus = () => refresh(true);
    window.addEventListener("focus", onFocus);
    return () => { window.clearInterval(interval); window.removeEventListener("focus", onFocus); };
  }, [refresh]);

  const postAction = useCallback<Action>(async (action, extra = {}) => {
    setBusy(true);
    setResult(null);
    try {
      const response = await fetch("/api/git", {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ action, workspace, ...extra }),
      });
      const data = await response.json().catch(() => ({}));
      if (!response.ok) throw new Error(typeof data.error === "string" ? data.error : `${action} failed`);
      if (data.status) ide.setGitStatus(data.status as GitStatus);
      setError(null);
      setResult(`${action.replace(/([A-Z])/g, " $1").toLowerCase()} completed`);
      setSelected(null);
      setDiff("");
      return true;
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : "Network error");
      return false;
    } finally {
      setBusy(false);
    }
  }, [ide, workspace]);

  const fetchDiff = useCallback(async (entry: GitStatusEntry) => {
    if (entry.status === "untracked") { ide.openFile(entry.path); return; }
    setSelected(entry.path);
    setDiff("");
    setDiffLoading(true);
    try {
      const response = await fetch(`/api/git?workspace=${encodeURIComponent(workspace)}&diff=${encodeURIComponent(entry.path)}&staged=${entry.staged ? 1 : 0}`);
      const data = await response.json().catch(() => ({}));
      if (!response.ok) throw new Error("Diff unavailable");
      setDiff(typeof data.diff === "string" ? data.diff : "");
    } catch { setDiff(""); }
    finally { setDiffLoading(false); }
  }, [ide, workspace]);

  const totalChanges = status?.entries.length ?? 0;
  if (compact) {
    if (!status || status.bare) return <span className="flex items-center gap-1 text-[12px] text-ink-500"><GitBranchIcon width={12} height={12} />no repo</span>;
    return <span className="flex items-center gap-1.5 text-[12px] text-ink-400"><GitBranchIcon width={12} height={12} /><span className="text-ink-200">{status.branch}</span>{status.ahead > 0 && <span className="text-emerald-400">↑{status.ahead}</span>}{status.behind > 0 && <span className="text-amber-300">↓{status.behind}</span>}{totalChanges > 0 && <span className="text-ink-500">• {totalChanges}</span>}</span>;
  }

  return (
    <div className="flex h-full flex-col text-ink-200">
      <div className="flex items-center justify-between border-b border-ink-800 px-3 py-2">
        <span className="text-[11px] font-semibold uppercase tracking-wider text-ink-400">Source Control</span>
        <button onClick={() => refresh()} disabled={loading} title="Refresh all Git data" className="rounded p-1 text-ink-400 hover:bg-ink-800 hover:text-ink-100 disabled:opacity-40"><RefreshIcon width={14} height={14} /></button>
      </div>
      {error && <Notice danger onClose={() => setError(null)}>{error}</Notice>}
      {result && <Notice onClose={() => setResult(null)}>{result}</Notice>}
      {!status && loading && <div className="px-3 py-4 text-[12px] text-ink-500">Loading repository…</div>}
      {status?.bare && <div className="p-4 text-center"><p className="mb-3 text-[12px] text-ink-400">This workspace is not a Git repository.</p><button className={primaryClass} disabled={busy} onClick={() => postAction("init")}>Initialize Repository</button></div>}
      {status && !status.bare && <>
        <RepositoryHeader status={status} busy={busy} action={postAction} />
        <div className="flex overflow-x-auto border-b border-ink-800 px-1">
          {(["changes", "history", "branches", "stashes", "repository"] as Tab[]).map((value) => <button key={value} onClick={() => setTab(value)} className={`shrink-0 border-b-2 px-2 py-1.5 text-[10px] font-medium capitalize ${tab === value ? "border-accent text-ink-100" : "border-transparent text-ink-500 hover:text-ink-200"}`}>{value}{value === "changes" && totalChanges ? ` ${totalChanges}` : ""}</button>)}
        </div>
        <div className="flex-1 overflow-y-auto">
          {tab === "changes" && <Changes status={status} busy={busy} action={postAction} selected={selected} fetchDiff={fetchDiff} />}
          {tab === "history" && <History commits={status.commits ?? []} busy={busy} action={postAction} />}
          {tab === "branches" && <Branches current={status.branch} branches={status.branches ?? []} busy={busy} action={postAction} />}
          {tab === "stashes" && <Stashes stashes={status.stashes ?? []} busy={busy} action={postAction} />}
          {tab === "repository" && <Repository status={status} busy={busy} action={postAction} />}
          {selected && tab === "changes" && <div className="border-t border-ink-800"><div className="flex items-center justify-between px-3 py-1.5"><span className="truncate text-[11px] text-ink-400">{selected}</span><button className={buttonClass} onClick={() => { setSelected(null); setDiff(""); }}>Close</button></div><div className="max-h-80 overflow-auto px-2 pb-2">{diffLoading ? <Empty>Loading diff…</Empty> : diff ? <Diff diff={diff} /> : <Empty>No textual diff available.</Empty>}</div></div>}
        </div>
      </>}
    </div>
  );
}

function RepositoryHeader({ status, busy, action }: { status: GitStatus; busy: boolean; action: Action }) {
  const [pullMode, setPullMode] = useState("--no-rebase");
  return <div className="border-b border-ink-800 px-3 py-2 text-[12px]">
    <div className="flex flex-wrap items-center gap-2"><GitBranchIcon width={13} height={13} className="shrink-0 text-ink-400" /><span className="min-w-0 truncate font-medium text-ink-100">{status.branch}</span>{status.ahead > 0 && <span className="text-emerald-400">↑{status.ahead}</span>}{status.behind > 0 && <span className="text-amber-300">↓{status.behind}</span>}<span className="ml-auto truncate text-[10px] text-ink-600" title={status.upstream ?? "No upstream"}>{status.upstream ?? "no upstream"}</span></div>
    <div className="mt-2 flex flex-wrap gap-1"><button className={buttonClass} disabled={busy} onClick={() => action("fetch", { prune: true })}>Fetch + prune</button><button className={buttonClass} disabled={busy} onClick={() => action("pull", { mode: pullMode })}>Pull</button><select value={pullMode} onChange={(event) => setPullMode(event.target.value)} className={inputClass} aria-label="Pull strategy"><option value="--no-rebase">merge</option><option value="--rebase">rebase</option><option value="--ff-only">fast-forward only</option></select><button className={buttonClass} disabled={busy} onClick={() => action("push")}>Push</button></div>
  </div>;
}

function Changes({ status, busy, action, selected, fetchDiff }: { status: GitStatus; busy: boolean; action: Action; selected: string | null; fetchDiff: (entry: GitStatusEntry) => void }) {
  const groups = useMemo(() => groupEntries(status), [status]);
  const [message, setMessage] = useState("");
  const [amend, setAmend] = useState(false);
  const [signoff, setSignoff] = useState(false);
  const [noVerify, setNoVerify] = useState(false);
  const commit = async () => { if (await action("commit", { message, amend, signoff, noVerify })) setMessage(""); };
  const canCommit = Boolean(message.trim()) && (groups.staged.length > 0 || amend) && !busy;
  return <>
    <div className="border-b border-ink-800 p-2"><textarea className={`${inputClass} w-full resize-y`} rows={3} value={message} onChange={(event) => setMessage(event.target.value)} onKeyDown={(event) => { if ((event.ctrlKey || event.metaKey) && event.key === "Enter" && canCommit) { event.preventDefault(); commit(); } }} placeholder={amend ? "New message for amended commit" : "Commit message (Ctrl+Enter)"} /><div className="my-1.5 flex flex-wrap gap-3"><Check label="Amend" value={amend} set={setAmend} /><Check label="Sign-off" value={signoff} set={setSignoff} /><Check label="Skip hooks" value={noVerify} set={setNoVerify} /></div><button className={`${primaryClass} flex w-full items-center justify-center gap-1`} disabled={!canCommit} onClick={commit}><CheckIcon width={13} height={13} />{amend ? "Amend commit" : `Commit (${groups.staged.length})`}</button></div>
    <div className="flex flex-wrap gap-1 border-b border-ink-800 p-2"><button className={buttonClass} disabled={busy || status.entries.length === 0} onClick={() => action("stashPush", { includeUntracked: true })}>Stash all</button><button className={buttonClass} disabled={busy || groups.unstaged.length === 0} onClick={() => confirmAction("Discard every tracked worktree change?", () => action("discardAll"))}>Discard tracked</button><button className={`${buttonClass} hover:text-red-400`} disabled={busy || groups.untracked.length === 0} onClick={() => confirmAction("Permanently delete all untracked files and folders?", () => action("clean"))}>Clean untracked</button></div>
    <ChangeGroup title="Staged Changes" entries={groups.staged} actionLabel="Unstage all" onAction={() => action("unstageAll")} busy={busy}>{(entry) => <FileRow key={`s-${entry.path}`} entry={entry} active={selected === entry.path} onClick={() => fetchDiff(entry)}><IconButton title="Unstage" disabled={busy} onClick={() => action("unstage", { path: entry.path })}><MinusIcon width={13} height={13} /></IconButton></FileRow>}</ChangeGroup>
    <ChangeGroup title="Changes" entries={groups.unstaged} actionLabel="Stage all" onAction={() => action("stageAll")} busy={busy}>{(entry) => <FileRow key={`c-${entry.path}`} entry={entry} active={selected === entry.path} onClick={() => fetchDiff(entry)}><IconButton title="Stage" disabled={busy} onClick={() => action("stage", { path: entry.path })}><PlusIcon width={13} height={13} /></IconButton><IconButton title="Discard" danger disabled={busy} onClick={() => confirmAction(`Discard changes to ${entry.path}?`, () => action("discard", { path: entry.path }))}><TrashIcon width={13} height={13} /></IconButton></FileRow>}</ChangeGroup>
    <ChangeGroup title="Untracked" entries={groups.untracked} actionLabel="Stage all" onAction={() => action("stageAll")} busy={busy}>{(entry) => <FileRow key={`u-${entry.path}`} entry={entry} active={selected === entry.path} onClick={() => fetchDiff(entry)}><IconButton title="Stage" disabled={busy} onClick={() => action("stage", { path: entry.path })}><PlusIcon width={13} height={13} /></IconButton></FileRow>}</ChangeGroup>
    {status.entries.length === 0 && <Empty>No changes — working tree clean.</Empty>}
  </>;
}

function History({ commits, busy, action }: { commits: GitCommit[]; busy: boolean; action: Action }) {
  const [query, setQuery] = useState("");
  const visible = commits.filter((commit) => `${commit.subject} ${commit.author} ${commit.oid}`.toLowerCase().includes(query.toLowerCase()));
  const reset = (commit: GitCommit) => { const mode = window.prompt("Reset mode: soft, mixed, or hard", "mixed"); if (mode && ["soft", "mixed", "hard"].includes(mode) && (mode !== "hard" || window.confirm("Hard reset discards local changes. Continue?"))) action("reset", { ref: commit.oid, mode }); };
  return <><div className="border-b border-ink-800 p-2"><input className={`${inputClass} w-full`} value={query} onChange={(event) => setQuery(event.target.value)} placeholder="Filter commits by message, author, or SHA" /></div>{visible.map((commit) => <div key={commit.oid} className="group border-b border-ink-800/70 px-3 py-2"><div className="flex items-start gap-2"><span className="font-mono text-[10px] text-accent-soft">{commit.shortOid}</span><div className="min-w-0 flex-1"><div className="truncate text-[12px] text-ink-100" title={commit.subject}>{commit.subject}</div><div className="mt-0.5 truncate text-[10px] text-ink-500">{commit.author} · {formatDate(commit.ts)}</div>{commit.refs.length > 0 && <div className="mt-1 flex flex-wrap gap-1">{commit.refs.map((ref) => <span key={ref} className="rounded bg-ink-800 px-1 text-[9px] text-sky-300">{ref}</span>)}</div>}</div></div><div className="mt-1.5 flex flex-wrap gap-1 opacity-70 group-hover:opacity-100"><button className={buttonClass} disabled={busy} onClick={() => action("checkout", { branch: commit.oid })}>Checkout</button><button className={buttonClass} disabled={busy} onClick={() => action("cherryPick", { ref: commit.oid })}>Cherry-pick</button><button className={buttonClass} disabled={busy} onClick={() => confirmAction(`Create a revert commit for ${commit.shortOid}?`, () => action("revert", { ref: commit.oid }))}>Revert</button><button className={buttonClass} disabled={busy} onClick={() => reset(commit)}>Reset…</button></div></div>)}{visible.length === 0 && <Empty>No commits match.</Empty>}</>;
}

function Branches({ current, branches, busy, action }: { current: string; branches: GitBranch[]; busy: boolean; action: Action }) {
  const [name, setName] = useState(""); const [startPoint, setStartPoint] = useState(""); const [checkout, setCheckout] = useState(true);
  const local = branches.filter((branch) => !branch.remote); const remote = branches.filter((branch) => branch.remote);
  const create = async () => { if (await action("createBranch", { branch: name, startPoint: startPoint || undefined, checkout })) { setName(""); setStartPoint(""); } };
  return <><Section title="Create branch"><div className="grid grid-cols-2 gap-1"><input className={inputClass} value={name} onChange={(event) => setName(event.target.value)} placeholder="branch name" /><input className={inputClass} value={startPoint} onChange={(event) => setStartPoint(event.target.value)} placeholder="start point (HEAD)" /></div><div className="mt-2 flex items-center justify-between"><Check label="Switch after creation" value={checkout} set={setCheckout} /><button className={primaryClass} disabled={busy || !name.trim()} onClick={create}>Create</button></div></Section><BranchList title="Local branches" branches={local} current={current} busy={busy} action={action} /><BranchList title="Remote branches" branches={remote} current={current} busy={busy} action={action} /></>;
}

function BranchList({ title, branches, current, busy, action }: { title: string; branches: GitBranch[]; current: string; busy: boolean; action: Action }) {
  return <Section title={`${title} (${branches.length})`}>{branches.map((branch) => <div key={`${branch.remote}-${branch.name}`} className="group border-t border-ink-800/70 py-2 first:border-0"><div className="flex items-center gap-2"><GitBranchIcon width={12} height={12} className={branch.current ? "text-accent" : "text-ink-500"} /><span className={`min-w-0 flex-1 truncate text-[12px] ${branch.current ? "font-medium text-ink-100" : "text-ink-300"}`}>{branch.name}</span><span className="font-mono text-[9px] text-ink-600">{branch.oid}</span></div>{branch.upstream && <div className="ml-5 truncate text-[10px] text-ink-600">↳ {branch.upstream} {branch.ahead ? `↑${branch.ahead}` : ""} {branch.behind ? `↓${branch.behind}` : ""}</div>}<div className="ml-5 mt-1 flex flex-wrap gap-1 opacity-70 group-hover:opacity-100">{!branch.current && <button className={buttonClass} disabled={busy} onClick={() => action("checkout", { branch: branch.name })}>Switch</button>}<button className={buttonClass} disabled={busy || branch.name === current} onClick={() => action("merge", { ref: branch.name })}>Merge</button><button className={buttonClass} disabled={busy || branch.name === current} onClick={() => action("merge", { ref: branch.name, squash: true })}>Squash</button><button className={buttonClass} disabled={busy || branch.name === current} onClick={() => action("merge", { ref: branch.name, noFf: true })}>No-FF</button><button className={buttonClass} disabled={busy || branch.name === current} onClick={() => action("rebase", { ref: branch.name })}>Rebase onto</button>{!branch.remote && !branch.current && <><button className={`${buttonClass} hover:text-red-400`} disabled={busy} onClick={() => confirmAction(`Delete branch ${branch.name}?`, () => action("deleteBranch", { branch: branch.name }))}>Delete</button><button className={`${buttonClass} hover:text-red-400`} disabled={busy} onClick={() => confirmAction(`Force-delete unmerged branch ${branch.name}?`, () => action("deleteBranch", { branch: branch.name, force: true }))}>Force delete</button></>}{!branch.remote && <button className={buttonClass} disabled={busy} onClick={() => { const next = window.prompt("New branch name", branch.name); if (next) action("renameBranch", { branch: branch.name, newName: next }); }}>Rename</button>}</div></div>)}</Section>;
}

function Stashes({ stashes, busy, action }: { stashes: GitStash[]; busy: boolean; action: Action }) {
  const [message, setMessage] = useState(""); const [includeUntracked, setIncludeUntracked] = useState(true); const [keepIndex, setKeepIndex] = useState(false);
  return <><Section title="Save working changes"><input className={`${inputClass} w-full`} value={message} onChange={(event) => setMessage(event.target.value)} placeholder="Optional stash message" /><div className="mt-2 flex flex-wrap items-center gap-3"><Check label="Include untracked" value={includeUntracked} set={setIncludeUntracked} /><Check label="Keep staged" value={keepIndex} set={setKeepIndex} /><button className={`${primaryClass} ml-auto`} disabled={busy} onClick={async () => { if (await action("stashPush", { message, includeUntracked, keepIndex })) setMessage(""); }}>Stash</button></div></Section>{stashes.map((stash) => <div key={stash.ref} className="border-b border-ink-800 px-3 py-2"><div className="flex gap-2"><span className="font-mono text-[10px] text-violet-300">{stash.ref}</span><span className="min-w-0 flex-1 truncate text-[12px] text-ink-200">{stash.subject}</span></div><div className="mt-0.5 text-[10px] text-ink-600">{formatDate(stash.ts)}</div><div className="mt-1.5 flex flex-wrap gap-1"><button className={buttonClass} disabled={busy} onClick={() => action("stashApply", { ref: stash.ref })}>Apply</button><button className={buttonClass} disabled={busy} onClick={() => action("stashApply", { ref: stash.ref, pop: true })}>Pop</button><button className={buttonClass} disabled={busy} onClick={() => { const branch = window.prompt("New branch name"); if (branch) action("stashBranch", { ref: stash.ref, branch }); }}>Create branch</button><button className={`${buttonClass} hover:text-red-400`} disabled={busy} onClick={() => confirmAction(`Drop ${stash.ref}?`, () => action("stashDrop", { ref: stash.ref }))}>Drop</button></div></div>)}{stashes.length === 0 && <Empty>No stashes.</Empty>}</>;
}

function Repository({ status, busy, action }: { status: GitStatus; busy: boolean; action: Action }) {
  return <><Tags tags={status.tags ?? []} busy={busy} action={action} /><Remotes remotes={status.remotes ?? []} current={status.branch} busy={busy} action={action} /><Section title="Sync & recovery"><div className="flex flex-wrap gap-1"><button className={buttonClass} disabled={busy} onClick={() => action("fetch", { all: true, prune: true })}>Fetch all + prune</button><button className={buttonClass} disabled={busy} onClick={() => confirmAction("Force-push with lease? Remote history may change.", () => action("push", { forceWithLease: true }))}>Force push with lease</button><button className={buttonClass} disabled={busy} onClick={() => { const upstream = window.prompt("Upstream (for example origin/main)"); if (upstream) action("setUpstream", { branch: status.branch, upstream }); }}>Set upstream</button>{status.upstream && <button className={buttonClass} disabled={busy} onClick={() => action("unsetUpstream", { branch: status.branch })}>Unset upstream</button>}</div><div className="mt-3 text-[10px] font-semibold uppercase tracking-wider text-ink-500">In-progress operation</div><div className="mt-1 grid grid-cols-2 gap-1">{["merge", "rebase", "cherry-pick", "revert"].flatMap((operation) => [<button key={`${operation}-continue`} className={buttonClass} disabled={busy} onClick={() => action("continue", { operation })}>Continue {operation}</button>, <button key={`${operation}-abort`} className={buttonClass} disabled={busy} onClick={() => action("abort", { operation })}>Abort {operation}</button>])}</div></Section><Section title="Danger zone"><button className={`${buttonClass} hover:text-red-400`} disabled={busy} onClick={() => confirmAction("Delete ignored files too? This permanently removes build output and all ignored files.", () => action("clean", { includeIgnored: true }))}>Clean untracked + ignored files</button></Section></>;
}

function Tags({ tags, busy, action }: { tags: GitTag[]; busy: boolean; action: Action }) {
  const [tag, setTag] = useState(""); const [ref, setRef] = useState(""); const [message, setMessage] = useState(""); const annotated = Boolean(message.trim());
  return <Section title={`Tags (${tags.length})`}><div className="grid grid-cols-2 gap-1"><input className={inputClass} value={tag} onChange={(event) => setTag(event.target.value)} placeholder="tag name" /><input className={inputClass} value={ref} onChange={(event) => setRef(event.target.value)} placeholder="target (HEAD)" /></div><input className={`${inputClass} mt-1 w-full`} value={message} onChange={(event) => setMessage(event.target.value)} placeholder="Annotation (blank creates lightweight tag)" /><button className={`${primaryClass} mt-1`} disabled={busy || !tag.trim()} onClick={() => action("tagCreate", { tag, ref: ref || undefined, message, annotated })}>Create tag</button><div className="mt-2 max-h-48 overflow-y-auto">{tags.map((item) => <div key={item.name} className="group flex items-center gap-2 border-t border-ink-800/70 py-1.5"><span className="min-w-0 flex-1 truncate text-[12px] text-ink-200" title={item.subject}>{item.name}</span><span className="font-mono text-[9px] text-ink-600">{item.oid}</span><button className={buttonClass} disabled={busy} onClick={() => action("tagPush", { tag: item.name })}>Push</button><button className={`${buttonClass} hover:text-red-400`} disabled={busy} onClick={() => confirmAction(`Delete local tag ${item.name}?`, () => action("tagDelete", { tag: item.name }))}>Delete</button></div>)}</div></Section>;
}

function Remotes({ remotes, current, busy, action }: { remotes: GitRemote[]; current: string; busy: boolean; action: Action }) {
  const [name, setName] = useState(""); const [url, setUrl] = useState("");
  return <Section title={`Remotes (${remotes.length})`}><div className="flex gap-1"><input className={`${inputClass} w-24`} value={name} onChange={(event) => setName(event.target.value)} placeholder="name" /><input className={`${inputClass} flex-1`} value={url} onChange={(event) => setUrl(event.target.value)} placeholder="URL" /><button className={primaryClass} disabled={busy || !name.trim() || !url.trim()} onClick={async () => { if (await action("remoteAdd", { remote: name, url })) { setName(""); setUrl(""); } }}>Add</button></div>{remotes.map((remote) => <div key={remote.name} className="border-t border-ink-800/70 py-2 first:mt-2"><div className="flex items-center gap-2"><span className="font-medium text-[12px] text-ink-100">{remote.name}</span><span className="min-w-0 flex-1 truncate text-[10px] text-ink-500" title={remote.fetchUrl}>{remote.fetchUrl}</span></div><div className="mt-1 flex flex-wrap gap-1"><button className={buttonClass} disabled={busy} onClick={() => action("push", { setUpstream: true, remote: remote.name, branch: current })}>Publish branch</button><button className={buttonClass} disabled={busy} onClick={() => { const next = window.prompt("New remote URL", remote.fetchUrl); if (next) action("remoteSetUrl", { remote: remote.name, url: next }); }}>Change URL</button><button className={buttonClass} disabled={busy} onClick={() => { const next = window.prompt("New remote name", remote.name); if (next) action("remoteRename", { remote: remote.name, newName: next }); }}>Rename</button><button className={`${buttonClass} hover:text-red-400`} disabled={busy} onClick={() => confirmAction(`Remove remote ${remote.name}?`, () => action("remoteRemove", { remote: remote.name }))}>Remove</button></div></div>)}</Section>;
}

function ChangeGroup({ title, entries, actionLabel, onAction, busy, children }: { title: string; entries: GitStatusEntry[]; actionLabel: string; onAction: () => void; busy: boolean; children: (entry: GitStatusEntry) => ReactNode }) {
  if (!entries.length) return null;
  return <div className="border-b border-ink-800"><div className="flex items-center justify-between px-3 py-1.5"><span className="text-[11px] font-semibold uppercase tracking-wider text-ink-400">{title} <span className="text-ink-600">({entries.length})</span></span><button className="text-[11px] text-ink-500 hover:text-ink-100 disabled:opacity-40" onClick={onAction} disabled={busy}>{actionLabel}</button></div>{entries.map(children)}</div>;
}

function FileRow({ entry, active, onClick, children }: { entry: GitStatusEntry; active: boolean; onClick: () => void; children: ReactNode }) {
  return <div className={`group flex items-center gap-1.5 px-3 py-1 text-[12px] hover:bg-ink-850 ${active ? "bg-ink-850" : ""}`}><span className={`w-3 shrink-0 text-center font-mono ${statusColor(entry.status)}`}>{statusLetter(entry.status)}</span><button onClick={onClick} title={entry.path} className="min-w-0 flex-1 truncate text-left text-ink-200 hover:text-ink-100">{entry.path}{entry.oldPath && <span className="text-ink-600"> ← {entry.oldPath}</span>}</button><div className="flex items-center gap-0.5 opacity-0 transition-opacity group-hover:opacity-100 focus-within:opacity-100">{children}</div></div>;
}

function IconButton({ title, onClick, disabled, danger, children }: { title: string; onClick: () => void; disabled: boolean; danger?: boolean; children: ReactNode }) { return <button title={title} onClick={(event) => { event.stopPropagation(); onClick(); }} disabled={disabled} className={`rounded p-0.5 text-ink-400 hover:bg-ink-700 disabled:opacity-40 ${danger ? "hover:text-red-400" : "hover:text-ink-100"}`}>{children}</button>; }
function Check({ label, value, set }: { label: string; value: boolean; set: (value: boolean) => void }) { return <label className="flex items-center gap-1 text-[10px] text-ink-400"><input type="checkbox" checked={value} onChange={(event) => set(event.target.checked)} className="accent-[var(--accent)]" />{label}</label>; }
function Section({ title, children }: { title: string; children: ReactNode }) { return <section className="border-b border-ink-800 p-3"><h3 className="mb-2 text-[11px] font-semibold uppercase tracking-wider text-ink-400">{title}</h3>{children}</section>; }
function Empty({ children }: { children: ReactNode }) { return <div className="px-3 py-6 text-center text-[12px] text-ink-500">{children}</div>; }
function Notice({ children, danger, onClose }: { children: ReactNode; danger?: boolean; onClose: () => void }) { return <div className={`m-2 flex items-start gap-2 rounded border px-2 py-1.5 text-[11px] ${danger ? "border-red-400/30 bg-red-400/10 text-red-300" : "border-emerald-400/20 bg-emerald-400/10 text-emerald-300"}`}><span className="min-w-0 flex-1 break-words">{children}</span><button onClick={onClose} aria-label="Dismiss">×</button></div>; }
function confirmAction(message: string, action: () => unknown) { if (window.confirm(message)) action(); }
function formatDate(timestamp: number) { return timestamp ? new Intl.DateTimeFormat(undefined, { dateStyle: "medium", timeStyle: "short" }).format(timestamp) : "unknown date"; }
