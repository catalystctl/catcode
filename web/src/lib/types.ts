// Shared types for the hub frontend (project tabs + git + terminals).
// Kept deliberately small — the old agent/chat/IDE type surface lived here
// and was removed when the web UI became hub-only.

export interface ProjectEntry {
  /** Absolute workspace path. */
  path: string;
  /** Display name (basename). */
  name: string;
  /** Last-accessed timestamp (ms). */
  lastUsed: number;
}

/** One row of `git status --porcelain=v2`. */
export interface GitStatusEntry {
  /** Workspace-relative path. For renames: "old -> new". */
  path: string;
  /** Original path for renames, else null. */
  oldPath?: string | null;
  /** XY status codes from porcelain v2 (e.g. "M ", " M", "A ", "??", "R "). */
  xy: string;
  /** Human label. */
  status: "modified" | "added" | "deleted" | "renamed" | "untracked" | "conflicted";
  /** Staged (index) vs unstaged (worktree). */
  staged: boolean;
}

/** In-progress Git operations the panel can continue or abort. */
export type GitOperation = "merge" | "rebase" | "cherry-pick" | "revert";

/** Aggregate git state for the git panel. */
export interface GitStatus {
  /** Current branch name, or "HEAD (detached)". */
  branch: string;
  /** Commits ahead of upstream (0 if no upstream). */
  ahead: number;
  /** Commits behind upstream. */
  behind: number;
  /** All changed entries (staged + unstaged + untracked). */
  entries: GitStatusEntry[];
  /** HEAD commit short oid, or null if no commits. */
  head: { oid: string; message: string; author: string; ts: number } | null;
  /** True if the workspace is not a git repo (panel shows "initialize" CTA). */
  bare: boolean;
  /** Configured upstream for the current branch, if any. */
  upstream?: string | null;
  /** Local and remote branches. */
  branches?: GitBranch[];
  /** Recent commits across all refs. */
  commits?: GitCommit[];
  /** Saved worktree snapshots. */
  stashes?: GitStash[];
  /** Repository tags. */
  tags?: GitTag[];
  /** Configured remotes and their fetch/push URLs. */
  remotes?: GitRemote[];
  /** Active merge/rebase/cherry-pick/revert operations (if any). */
  operations?: GitOperation[];
}

export interface GitBranch {
  name: string;
  oid: string;
  current: boolean;
  remote: boolean;
  upstream: string | null;
  ahead: number;
  behind: number;
}

export interface GitCommit {
  oid: string;
  shortOid: string;
  parents: string[];
  subject: string;
  author: string;
  email: string;
  ts: number;
  refs: string[];
}

export interface GitStash {
  ref: string;
  oid: string;
  subject: string;
  ts: number;
}

export interface GitTag {
  name: string;
  oid: string;
  subject: string;
}

export interface GitRemote {
  name: string;
  fetchUrl: string;
  pushUrl: string;
}
