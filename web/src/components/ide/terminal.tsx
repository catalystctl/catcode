"use client";
// web/src/components/ide/terminal.tsx
//
// Ghostty's VT engine (WASM) over a WebSocket at /api/terminal. The custom
// Next server owns a real pseudoterminal and keeps it alive across panel
// switches/reloads; this component is only the renderer + input transport.
//
// This component is loaded via next/dynamic({ ssr:false }) by the panel
// registry so Ghostty never runs on the server and never enters the main
// bundle chunk. It is intentionally self-contained (props, not a shared
// IdeContext) so it can be wired up before/without the rest of the IDE shell.

import { useEffect, useRef } from "react";
import type { Terminal as GhosttyTerminal } from "ghostty-web";

// ── WS message envelopes (mirror web/src/server/server.ts, contract §4.5) ──
type ServerMsg =
  | { type: "data"; data: string }
  | { type: "exit"; code: number }
  | { type: "ready"; sessionId: string }
  | { type: "terminated" }
  | { type: "pong" };

let ghosttyReady: Promise<void> | null = null;

function terminalSocketUrl(): string {
  const scheme = location.protocol === "https:" ? "wss" : "ws";
  return `${scheme}://${location.host}/api/terminal`;
}

/** Explicitly terminate a persistent server-side PTY, including when detached. */
function terminateTerminalSession(sessionId: string): void {
  const socket = new WebSocket(terminalSocketUrl());
  socket.onopen = () => socket.send(JSON.stringify({ type: "terminate", sessionId }));
  socket.onmessage = () => socket.close();
  socket.onerror = () => socket.close();
}

// ── Local TerminalSession type ─────────────────────────────────────────────
// Matches contract types.ts §2 verbatim. Kept local here (rather than imported
// from @/lib/types) so this panel compiles standalone before the shared IDE
// types land; it is a drop-in once types.ts exports the same interface.
export interface TerminalSession {
  /** Client-generated id (e.g. "term_<ts>_<n>"). */
  id: string;
  /** Display title (defaults to shell name; user-renamable). */
  title: string;
  /** Workspace-relative or absolute cwd the shell started in. */
  cwd: string;
  /** True while the shell process is alive. */
  alive: boolean;
  /** Last exit code (null while alive / not yet exited). */
  exitCode: number | null;
}

export interface TerminalProps {
  /** Client-generated session id (e.g. "term_<ts>_<n>"). */
  sessionId: string;
  /** Absolute workspace root the shell should run in. */
  workspace: string;
  /** Workspace-relative cwd for the shell (defaults to workspace root). */
  cwd?: string;
  /** Called when the shell process exits with a code. */
  onExit?: (code: number) => void;
}

/**
 * One Ghostty terminal session bound to a single WebSocket. Opens the WS on
 * mount, sends {type:"open",sessionId,cwd,cols,rows}, pipes data ↔ Ghostty, and
 * reports the shell exit via onExit. Disposes cleanly on unmount.
 */
export function Terminal({ sessionId, workspace, cwd, onExit }: TerminalProps) {
  const containerRef = useRef<HTMLDivElement>(null);
  const termRef = useRef<GhosttyTerminal | null>(null);

  // Keep latest values in refs so the effect only re-runs when the session id
  // changes (one renderer + WS attachment per session), not on parent renders.
  const workspaceRef = useRef(workspace);
  workspaceRef.current = workspace;
  const cwdRef = useRef(cwd);
  cwdRef.current = cwd;
  const onExitRef = useRef(onExit);
  onExitRef.current = onExit;

  useEffect(() => {
    let disposed = false;
    let term: GhosttyTerminal | null = null;
    let ws: WebSocket | null = null;
    let pingTimer: ReturnType<typeof setInterval> | null = null;

    (async () => {
      // Dynamic import keeps Ghostty's renderer + WASM out of the server and
      // initial application chunks.
      const ghostty = await import("ghostty-web");
      ghosttyReady ??= ghostty.init();
      await ghosttyReady;
      if (disposed || !containerRef.current) return;

      term = new ghostty.Terminal({
        fontFamily:
          "'JetBrains Mono Variable', 'JetBrains Mono', ui-monospace, monospace",
        fontSize: 13,
        cursorBlink: true,
        scrollback: 10000,
        theme: {
          background: "#080a0f",
          foreground: "#d6d9e0",
          cursor: "#a78bfa",
          selectionBackground: "#3b4261",
        },
      });
      const fit = new ghostty.FitAddon();
      term.loadAddon(fit);
      term.open(containerRef.current);
      try {
        fit.fit();
      } catch {
        /* container not sized yet */
      }
      termRef.current = term;
      term.focus();

      ws = new WebSocket(terminalSocketUrl());
      ws.onopen = () => {
        ws!.send(
          JSON.stringify({
            type: "open",
            sessionId,
            workspace: workspaceRef.current,
            cwd: cwdRef.current ?? "",
            cols: term!.cols,
            rows: term!.rows,
          }),
        );
      };
      ws.onmessage = (ev) => {
        const str =
          typeof ev.data === "string" ? ev.data : new TextDecoder().decode(ev.data as ArrayBuffer);
        let m: ServerMsg;
        try {
          m = JSON.parse(str) as ServerMsg;
        } catch {
          return;
        }
        if (m.type === "data") term!.write(m.data);
        else if (m.type === "exit") onExitRef.current?.(m.code);
      };
      ws.onerror = () => {
        /* surfaced via onclose; nothing else to do */
      };

      // stdin: every Ghostty-encoded keystroke → the real PTY.
      const onDataDisp = term.onData((data) => {
        if (ws && ws.readyState === WebSocket.OPEN) {
          ws.send(JSON.stringify({ type: "data", data }));
        }
      });

      // Keepalive (some proxies close idle WSes).
      pingTimer = setInterval(() => {
        if (ws && ws.readyState === WebSocket.OPEN) {
          ws.send(JSON.stringify({ type: "ping" }));
        }
      }, 30000);

      const onResizeDisp = term.onResize(({ cols, rows }) => {
        if (ws?.readyState === WebSocket.OPEN) {
          ws.send(JSON.stringify({ type: "resize", cols, rows }));
        }
      });
      fit.observeResize();

      // stash disposers for the cleanup fn
      (term as GhosttyTerminal & { __dispose?: () => void }).__dispose = () => {
        onDataDisp.dispose();
        onResizeDisp.dispose();
        if (pingTimer) clearInterval(pingTimer);
      };
    })();

    return () => {
      disposed = true;
      const t = termRef.current as (GhosttyTerminal & { __dispose?: () => void }) | null;
      t?.__dispose?.();
      if (ws) {
        ws.onopen = null;
        ws.onmessage = null;
        ws.onerror = null;
        try {
          ws.close();
        } catch {
          /* ignore */
        }
      }
      try {
        termRef.current?.dispose();
      } catch {
        /* ignore */
      }
      termRef.current = null;
    };
  }, [sessionId]);

  return <div ref={containerRef} className="h-full w-full overflow-hidden bg-[#080a0f]" />;
}

// ── Presentational panel (tab strip + active terminal) ─────────────────────
// Stateless: the owning shell wires this to the IdeContext (newTerminal /
// closeTerminal / setActiveTerminal / setTerminalExit). Kept here so the
// terminal panel is usable before/without the full IDE context plumbing.

export interface TerminalPanelProps {
  workspace: string;
  sessions: TerminalSession[];
  activeId: string | null;
  onNew: () => void;
  onClose: (id: string) => void;
  onSelect: (id: string) => void;
  onExit: (id: string, code: number) => void;
}

export function TerminalPanel({
  workspace,
  sessions,
  activeId,
  onNew,
  onClose,
  onSelect,
  onExit,
}: TerminalPanelProps) {
  const active = sessions.find((s) => s.id === activeId) ?? null;

  return (
    <div className="flex h-full w-full flex-col bg-ink-950 text-ink-100">
      <div className="flex items-center gap-1 border-b border-ink-800 bg-ink-900 px-1">
        {sessions.length === 0 && (
          <span className="px-2 py-1 text-xs text-ink-400">No terminals</span>
        )}
        {sessions.map((s) => (
          <div
            key={s.id}
            role="tab"
            tabIndex={0}
            aria-selected={s.id === activeId}
            onClick={() => onSelect(s.id)}
            onKeyDown={(e) => {
              if (e.key === "Enter" || e.key === " ") {
                e.preventDefault();
                onSelect(s.id);
              }
            }}
            className={`group flex cursor-pointer items-center gap-1 rounded-t px-2 py-1 text-xs ${
              s.id === activeId
                ? "bg-ink-950 text-ink-100"
                : "text-ink-400 hover:bg-ink-800/50 hover:text-ink-200"
            }`}
            title={s.cwd}
          >
            <span className="max-w-[12rem] truncate">{s.title}</span>
            {!s.alive && s.exitCode !== null && (
              <span className="text-ink-500" title={`exited ${s.exitCode}`}>
                [{s.exitCode}]
              </span>
            )}
            <button
              type="button"
              onClick={(e) => {
                e.preventDefault();
                e.stopPropagation();
                terminateTerminalSession(s.id);
                onClose(s.id);
              }}
              onKeyDown={(e) => {
                if (e.key === "Enter" || e.key === " ") {
                  e.preventDefault();
                  e.stopPropagation();
                  terminateTerminalSession(s.id);
                  onClose(s.id);
                }
              }}
              className="ml-1 rounded text-ink-500 opacity-100 hover:text-ink-100 sm:opacity-0 sm:group-hover:opacity-100 sm:focus-within:opacity-100"
              aria-label={`close ${s.title}`}
            >
              ×
            </button>
          </div>
        ))}
        <button
          type="button"
          onClick={onNew}
          className="ml-auto min-h-9 px-3 py-1 text-ink-400 hover:text-ink-100"
          title="New terminal"
          aria-label="new terminal"
        >
          +
        </button>
      </div>
      <div className="relative min-h-0 flex-1 p-1">
        {active ? (
          <Terminal
            key={active.id}
            sessionId={active.id}
            workspace={workspace}
            cwd={active.cwd}
            onExit={(code) => onExit(active.id, code)}
          />
        ) : (
          <div className="flex h-full items-center justify-center">
            <button
              type="button"
              onClick={onNew}
              className="rounded border border-ink-700 px-3 py-1.5 text-sm text-ink-300 hover:bg-ink-800"
            >
              Open a terminal
            </button>
          </div>
        )}
      </div>
    </div>
  );
}

export default Terminal;
