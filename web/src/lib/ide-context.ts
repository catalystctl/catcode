"use client";

// React context for hub panels. GitPanel still speaks the IdeApi shape;
// ChatInner uses openSettings / openProjects / attachToChat. The hub provides
// a minimal shim — no editor, docks, or terminal grid.

import { createContext, useContext } from "react";
import type { GitStatus } from "./types";

/** Payload for attaching Preview / IDE context into the chat composer. */
export type AttachToChatPayload = {
  /** Text appended to the composer draft. */
  text: string;
  /** Optional image data URL attached like a paste/upload. */
  image?: string;
};

export type AttachToChatFn = (payload: AttachToChatPayload) => void;

/** The subset of the old IdeApi that GitPanel (and Chat) actually call. */
export interface IdeApi {
  state: {
    gitStatus: GitStatus | null;
    /** Chat reads openTabs for "active file" chip; hub has no editor so []. */
    openTabs: Array<{ id: string; kind: string; target?: string }>;
    activeTabId: string | null;
  };
  setGitStatus: (s: GitStatus | null) => void;
  openDiff: (path: string, opts?: { staged?: boolean }) => void;
  openPatch: (source: "commit" | "stash", ref: string, label: string) => void;
  openFile: (path: string, language?: string) => void;
  selectEditor: () => void;
  /** No-op in hub (no IDE mode). Kept so Chat header can optionally call it. */
  setUiMode: (mode: "ide" | "chat") => void;
}

export interface IdeContextValue {
  /** Absolute workspace path of the active hub project tab. */
  workspace: string;
  /** Minimal IDE api shim (git open/diff/patch + status). */
  ide: IdeApi;
  openSettings: () => void;
  openProjects: () => void;
  /** Append text (and optional image) to the docked chat composer. */
  attachToChat: AttachToChatFn;
  /** ChatInner registers its composer bridge here on mount. */
  registerAttachToChat: (fn: AttachToChatFn | null) => void;
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
