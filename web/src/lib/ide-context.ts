"use client";

// React context exposing { workspace, ide } to all IDE panels so props stay
// minimal. IdeShell provides it; panels consume via useIdeContext().

import { createContext, useContext } from "react";
import type { IdeApi } from "./use-ide";

export interface IdeContextValue {
  /** Absolute workspace path (from the core `ready` event; updates on switch). */
  workspace: string;
  /** The IDE layout/panel state api. */
  ide: IdeApi;
  /** Opens application settings from any IDE panel or command surface. */
  openSettings: () => void;
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
