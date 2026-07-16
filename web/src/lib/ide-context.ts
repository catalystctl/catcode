"use client";

// React context exposing { workspace, ide } to all IDE panels so props stay
// minimal. IdeShell provides it; panels consume via useIdeContext().
// ChatInner registers attachToChat so Preview (and others) can push context
// into the composer without importing Chat.

import { createContext, useContext } from "react";
import type { IdeApi } from "./use-ide";

/** Payload for attaching Preview / IDE context into the chat composer. */
export type AttachToChatPayload = {
  /** Text appended to the composer draft. */
  text: string;
  /** Optional image data URL attached like a paste/upload. */
  image?: string;
};

export type AttachToChatFn = (payload: AttachToChatPayload) => void;

export interface IdeContextValue {
  /** Absolute workspace path (from the core `ready` event; updates on switch). */
  workspace: string;
  /** The IDE layout/panel state api. */
  ide: IdeApi;
  /** Opens application settings from any IDE panel or command surface. */
  openSettings: () => void;
  /** Append text (and optional image) to the docked chat composer. */
  attachToChat: AttachToChatFn;
  /** ChatInner registers its composer bridge here on mount. */
  registerAttachToChat: (fn: AttachToChatFn | null) => void;
}

export const IdeContext = createContext<IdeContextValue | null>(null);

/** Consume the IDE context. Throws if used outside an IdeShell provider. */
export function useIdeContext(): IdeContextValue {
  const ctx = useContext(IdeContext);
  if (!ctx) {
    throw new Error("useIdeContext must be used within an IdeContext provider");
  }
  return ctx;
}
