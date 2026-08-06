"use client";

// React context for hub panels that still speak the old IdeContext shape
// (GitPanel). The hub provides a minimal shim — no editor, docks, or chat.

import { createContext, useContext } from "react";
import type { GitStatus } from "./types";

/** The subset of the old IdeApi that GitPanel actually calls. */
export interface IdeApi {
  state: { gitStatus: GitStatus | null };
  setGitStatus: (s: GitStatus | null) => void;
  openDiff: (path: string, opts?: { staged?: boolean }) => void;
  openPatch: (source: "commit" | "stash", ref: string, label: string) => void;
  openFile: (path: string, language?: string) => void;
  selectEditor: () => void;
}

export interface IdeContextValue {
  /** Absolute workspace path of the active hub project tab. */
  workspace: string;
  /** Minimal IDE api shim (git open/diff/patch + status). */
  ide: IdeApi;
  openSettings: () => void;
  openProjects: () => void;
}

export const IdeContext = createContext<IdeContextValue | null>(null);

/** Consume the IDE context. Throws if used outside a provider. */
export function useIdeContext(): IdeContextValue {
  const ctx = useContext(IdeContext);
  if (!ctx) {
    throw new Error("useIdeContext must be used within an IdeContext provider");
  }
  return ctx;
}
