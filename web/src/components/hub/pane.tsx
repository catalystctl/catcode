"use client";

// One hub terminal pane = one persistent server PTY running catcode.
// The leaf id in the split tree IS the terminal session id, so restored
// layouts reattach their PTYs after a refresh (server keeps them alive).

import dynamic from "next/dynamic";
import { useEffect, useState } from "react";
import { RefreshIcon, XIcon } from "@/components/icons";
import { terminateTerminalSession } from "@/components/ide/terminal";

// Ghostty's WASM renderer must never run on the server.
const Terminal = dynamic(
  () => import("@/components/ide/terminal").then((m) => m.Terminal),
  {
    ssr: false,
    loading: () => (
      <div className="flex h-full items-center justify-center text-xs text-ink-500">
        Starting catcode…
      </div>
    ),
  },
);

type IconProps = { width?: number; height?: number; className?: string };

/** Split-right glyph (two vertical bars). */
export function ColsIcon({ width = 13, height = 13, className }: IconProps) {
  return (
    <svg
      width={width}
      height={height}
      viewBox="0 0 16 16"
      fill="none"
      stroke="currentColor"
      strokeWidth="1.4"
      className={className}
      aria-hidden
    >
      <rect x="1.7" y="2.2" width="5.2" height="11.6" rx="1" />
      <rect x="9.1" y="2.2" width="5.2" height="11.6" rx="1" />
    </svg>
  );
}

/** Split-down glyph (two horizontal bars). */
export function RowsIcon({ width = 13, height = 13, className }: IconProps) {
  return (
    <svg
      width={width}
      height={height}
      viewBox="0 0 16 16"
      fill="none"
      stroke="currentColor"
      strokeWidth="1.4"
      className={className}
      aria-hidden
    >
      <rect x="2.2" y="1.7" width="11.6" height="5.2" rx="1" />
      <rect x="2.2" y="9.1" width="11.6" height="5.2" rx="1" />
    </svg>
  );
}

export interface HubPaneProps {
  /** Leaf id — doubles as the persistent terminal session id. */
  paneId: string;
  /** Absolute project path the PTY runs in (allowlisted server-side). */
  workspace: string;
  /** True when this pane is the tab's focused pane. */
  focused: boolean;
  onFocus: () => void;
  onSplitRight: () => void;
  onSplitDown: () => void;
  onRestart: () => void;
  onClose: () => void;
}

type Phase = "running" | "exited" | "unavailable";

export function HubPane({
  paneId,
  workspace,
  focused,
  onFocus,
  onSplitRight,
  onSplitDown,
  onRestart,
  onClose,
}: HubPaneProps) {
  const [phase, setPhase] = useState<Phase>("running");
  const [exitCode, setExitCode] = useState<number | null>(null);

  // A restart swaps the pane id — reset to a fresh "running" lifecycle.
  useEffect(() => {
    setPhase("running");
    setExitCode(null);
  }, [paneId]);

  return (
    <div
      className={`group/pane relative h-full w-full min-h-0 min-w-0 overflow-hidden bg-ink-950 ${
        focused ? "ring-1 ring-inset ring-accent/50" : ""
      }`}
      onPointerDownCapture={onFocus}
    >
      {phase === "running" ? (
        <Terminal
          key={paneId}
          sessionId={paneId}
          workspace={workspace}
          launch="catcode"
          // Server-side PTYs persist across tab closes, sign-outs, device
          // sleep and long network gaps — never give up reattaching. A truly
          // gone PTY answers "missing" to the attach-only open and surfaces
          // via onUnavailable regardless of the budget.
          maxReconnects={Infinity}
          // Only the hub's focused pane may own the keyboard. Every project
          // tab stays mounted under `hidden`; unrestricted auto-focus lets a
          // background pane capture document.activeElement and blocks typing.
          autoFocus={focused}
          onExit={(code) => {
            setExitCode(code);
            setPhase("exited");
          }}
          onUnavailable={() => setPhase("unavailable")}
        />
      ) : (
        <div className="flex h-full flex-col items-center justify-center gap-3 px-4 text-center text-sm text-ink-400">
          <span>
            {phase === "exited"
              ? `catcode exited with code ${exitCode ?? 0}.`
              : "This terminal session is no longer available."}
          </span>
          <div className="flex items-center gap-2">
            <button
              type="button"
              onClick={onRestart}
              className="rounded-lg border border-ink-700 px-3 py-1.5 text-[12px] text-ink-200 transition-colors hover:bg-ink-850"
            >
              Restart catcode
            </button>
            <button
              type="button"
              onClick={onClose}
              className="rounded-lg border border-ink-700 px-3 py-1.5 text-[12px] text-ink-400 transition-colors hover:bg-ink-850 hover:text-ink-200"
            >
              Close pane
            </button>
          </div>
        </div>
      )}

      {/* Pane toolbar — visible on hover for desktop pointers, ALWAYS visible
          on touch (no hover). Pointer events stop at the bar so terminal
          clicks underneath are unaffected. */}
      <div
        className="absolute right-1.5 top-1.5 z-10 flex items-center gap-0.5 rounded-md border border-ink-800 bg-ink-900/95 p-0.5 opacity-100 shadow-elev-1 transition-opacity focus-within:opacity-100 sm:opacity-0 sm:group-hover/pane:opacity-100"
        onPointerDownCapture={(e) => e.stopPropagation()}
      >
        <ToolbarButton title="Split right" onClick={onSplitRight}>
          <ColsIcon />
        </ToolbarButton>
        <ToolbarButton title="Split down" onClick={onSplitDown}>
          <RowsIcon />
        </ToolbarButton>
        <ToolbarButton title="Restart catcode" onClick={onRestart}>
          <RefreshIcon width={12} height={12} />
        </ToolbarButton>
        <ToolbarButton title="Close pane" onClick={onClose} danger>
          <XIcon width={12} height={12} />
        </ToolbarButton>
      </div>
    </div>
  );
}

function ToolbarButton({
  title,
  onClick,
  danger,
  children,
}: {
  title: string;
  onClick: () => void;
  danger?: boolean;
  children: React.ReactNode;
}) {
  return (
    <button
      type="button"
      title={title}
      aria-label={title}
      onClick={onClick}
      className={`rounded p-1.5 transition-colors sm:p-1 ${
        danger
          ? "text-ink-500 hover:bg-danger/15 hover:text-danger"
          : "text-ink-400 hover:bg-ink-800 hover:text-ink-100"
      }`}
    >
      {children}
    </button>
  );
}

export { terminateTerminalSession };
