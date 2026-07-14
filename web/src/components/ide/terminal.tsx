"use client";
// web/src/components/ide/terminal.tsx
//
// xterm.js terminal over a WebSocket at /api/terminal (same origin). The shell
// itself is spawned server-side by the custom Next server
// (web/src/server/server.ts) — it is a line-mode child_process shell (no PTY,
// to keep the release cross-platform pure-JS; see contract §0.5/§9.3).
//
// This component is loaded via next/dynamic({ ssr:false }) by the panel
// registry so xterm.js never runs on the server and never enters the main
// bundle chunk. It is intentionally self-contained (props, not a shared
// IdeContext) so it can be wired up before/without the rest of the IDE shell.

import "@xterm/xterm/css/xterm.css";
import { useEffect, useRef } from "react";
import type { Terminal as XTerm } from "@xterm/xterm";
import type { FitAddon as FitAddonT } from "@xterm/addon-fit";

// ── WS message envelopes (mirror web/src/server/server.ts, contract §4.5) ──
type ServerMsg =
  | { type: "data"; data: string }
  | { type: "exit"; code: number }
  | { type: "pong" };

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
 * One xterm.js terminal session bound to a single WebSocket. Opens the WS on
 * mount, sends {type:"open",workspace,cwd,cols,rows}, pipes data ↔ xterm, and
 * reports the shell exit via onExit. Disposes cleanly on unmount.
 */
export function Terminal({ sessionId, workspace, cwd, onExit }: TerminalProps) {
  const containerRef = useRef<HTMLDivElement>(null);
  const termRef = useRef<XTerm | null>(null);

  // Keep latest values in refs so the effect only re-runs when the session id
  // changes (one xterm + WS per session), not on every parent render.
  const workspaceRef = useRef(workspace);
  workspaceRef.current = workspace;
  const cwdRef = useRef(cwd);
  cwdRef.current = cwd;
  const onExitRef = useRef(onExit);
  onExitRef.current = onExit;

  useEffect(() => {
    let disposed = false;
    let fit: FitAddonT | null = null;
    let term: XTerm | null = null;
    let ws: WebSocket | null = null;
    let pingTimer: ReturnType<typeof setInterval> | null = null;
    let ro: ResizeObserver | null = null;

    (async () => {
      // Dynamic imports keep xterm.js out of the server bundle + main chunk.
      const [{ Terminal: XTerm }, { FitAddon: Fit }] = await Promise.all([
        import("@xterm/xterm"),
        import("@xterm/addon-fit"),
      ]);
      // web-links addon is optional; load best-effort.
      let WebLinksAddon: typeof import("@xterm/addon-web-links").WebLinksAddon | null = null;
      try {
        ({ WebLinksAddon } = await import("@xterm/addon-web-links"));
      } catch {
        WebLinksAddon = null;
      }
      if (disposed || !containerRef.current) return;

      term = new XTerm({
        fontFamily:
          "'JetBrains Mono Variable', 'JetBrains Mono', ui-monospace, monospace",
        fontSize: 13,
        lineHeight: 1.2,
        cursorBlink: true,
        scrollback: 5000,
        allowProposedApi: true,
      });
      fit = new Fit();
      term.loadAddon(fit);
      if (WebLinksAddon) term.loadAddon(new WebLinksAddon());
      term.open(containerRef.current);
      try {
        fit.fit();
      } catch {
        /* container not sized yet */
      }
      termRef.current = term;
      term.focus();

      const wsScheme = location.protocol === "https:" ? "wss" : "ws";
      ws = new WebSocket(`${wsScheme}://${location.host}/api/terminal`);
      ws.onopen = () => {
        ws!.send(
          JSON.stringify({
            type: "open",
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

      // stdin: every xterm keystroke → WS data envelope.
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

      // Resize: refit xterm to its container and notify the server.
      const doResize = () => {
        if (!fit || !term || !ws) return;
        try {
          fit.fit();
          if (ws.readyState === WebSocket.OPEN) {
            ws.send(JSON.stringify({ type: "resize", cols: term.cols, rows: term.rows }));
          }
        } catch {
          /* ignore */
        }
      };
      ro = new ResizeObserver(doResize);
      ro.observe(containerRef.current);
      window.addEventListener("resize", doResize);

      // stash disposers for the cleanup fn
      (term as XTerm & { __dispose?: () => void }).__dispose = () => {
        onDataDisp.dispose();
        if (pingTimer) clearInterval(pingTimer);
        if (ro) ro.disconnect();
        window.removeEventListener("resize", doResize);
      };
    })();

    return () => {
      disposed = true;
      const t = termRef.current as (XTerm & { __dispose?: () => void }) | null;
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

  return <div ref={containerRef} className="h-full w-full overflow-hidden bg-black/40" />;
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
          <button
            key={s.id}
            type="button"
            onClick={() => onSelect(s.id)}
            className={`group flex items-center gap-1 rounded-t px-2 py-1 text-xs ${
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
            <span
              role="button"
              tabIndex={0}
              onClick={(e) => {
                e.stopPropagation();
                onClose(s.id);
              }}
              className="ml-1 text-ink-500 opacity-0 hover:text-ink-100 group-hover:opacity-100"
              aria-label={`close ${s.title}`}
            >
              ×
            </span>
          </button>
        ))}
        <button
          type="button"
          onClick={onNew}
          className="ml-auto px-2 py-1 text-ink-400 hover:text-ink-100"
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
