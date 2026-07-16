"use client";

import type { ReactNode } from "react";

/** Shared, compact chrome for IDE panels and tab groups. */
export function PanelHeader({
  children,
  trailing,
  className = "",
}: {
  children: ReactNode;
  trailing?: ReactNode;
  className?: string;
}) {
  return (
    <div className={`flex h-9 shrink-0 items-stretch overflow-x-auto border-b border-ink-800/90 bg-ink-925 ${className}`}>
      <div className="flex min-w-0 flex-1 items-stretch">{children}</div>
      {trailing ? <div className="ml-auto flex shrink-0 items-center gap-0.5 pr-1">{trailing}</div> : null}
    </div>
  );
}

export const panelTabClass = (active: boolean) =>
  `group relative flex min-w-0 items-center gap-1.5 border-r border-ink-800/80 px-3 text-xs transition-colors ${
    active
      ? "bg-ink-950 text-ink-100 after:absolute after:inset-x-0 after:bottom-0 after:h-px after:bg-accent/70"
      : "text-ink-400 hover:bg-ink-900 hover:text-ink-200"
  }`;
