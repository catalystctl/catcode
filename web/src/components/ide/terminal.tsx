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

import { useEffect, useRef, useState } from "react";
import type { Terminal as GhosttyTerminal } from "ghostty-web";
import {
  terminalOpenEnvelope,
  terminalTerminateEnvelope,
  type TerminalLaunch,
} from "@/lib/terminal-protocol";
import {
  clientToCell,
  domButtonToXterm,
  encodeSgrMouse,
  encodeSgrMouseMotion,
  encodeWheelReports,
  modsFromEvent,
} from "@/lib/terminal-mouse";

// ── WS message envelopes (mirror web/src/server/server.ts, contract §4.5) ──
type ServerMsg =
  | { type: "data"; data: string }
  | { type: "exit"; code: number }
  | { type: "ready"; sessionId: string }
  | { type: "terminated" }
  | { type: "missing"; sessionId: string }
  | { type: "pong" };

type ConnectionState = "initializing" | "connecting" | "connected" | "reconnecting" | "failed";

const PING_INTERVAL_MS = 30_000;
/** No server traffic for this long (pings go out every 30s and the server
 *  answers pong immediately) means the socket is half-open — the classic
 *  laptop-lid / phone-backgrounded / NAT-timeout case where the browser never
 *  fires onclose. The watchdog then tears the socket down and reattaches. */
const PONG_TIMEOUT_MS = 75_000;
const DEFAULT_MAX_RECONNECTS = 6;

let ghosttyReady: Promise<void> | null = null;

/**
 * Ghostty-web 0.4 `Terminal.open()` sets the container `contenteditable=true`
 * for IME. Chromium then reports `keyCode === 229` (composition) for plain
 * Latin keys, and Ghostty's InputHandler drops them (`if (keyCode===229) return`).
 * Holding Alt cancels composition so keys appear to "only work with Alt".
 *
 * Fix: drop contenteditable, keep the container as the tabbable target, and
 * park the hidden textarea at tabIndex=-1. Safe to call repeatedly (idempotent
 * attribute updates) when Ghostty re-applies contenteditable.
 */
function applyGhosttyInputHardening(
  term: GhosttyTerminal,
  container: HTMLElement,
): void {
  try {
    container.removeAttribute("contenteditable");
    // Explicit false beats some engines re-inferring editable from residual attrs.
    container.setAttribute("contenteditable", "false");
    container.removeAttribute("contenteditable");
    if (!container.hasAttribute("tabindex")) container.setAttribute("tabindex", "0");
    container.style.outline = container.style.outline || "none";
    // Stop mobile browsers from hijacking gestures (double-tap zoom / swipe nav)
    // which steals the first tap and breaks "click then type".
    container.style.touchAction = container.style.touchAction || "manipulation";
    container.style.userSelect = "none";
    (container.style as CSSStyleDeclaration & { webkitUserSelect?: string }).webkitUserSelect =
      "none";
  } catch {
    /* ignore */
  }
  const textarea = term.textarea;
  if (textarea) {
    try {
      textarea.tabIndex = -1;
      textarea.setAttribute("tabindex", "-1");
      // iOS: keep the software keyboard from auto-capitalising / correcting
      // when the textarea is briefly focused for paste.
      textarea.setAttribute("autocorrect", "off");
      textarea.setAttribute("autocapitalize", "off");
      textarea.setAttribute("spellcheck", "false");
      textarea.setAttribute("autocomplete", "off");
      textarea.style.pointerEvents = "none";
    } catch {
      /* ignore */
    }
  }
}

/**
 * One-shot listener install: canvas/textarea pointer events must focus the
 * container (Ghostty InputHandler target), not the hidden textarea. Also
 * re-assert hardening if Ghostty re-sets contenteditable, and skip right-click
 * so the context-menu paste path can raise the textarea briefly.
 */
function installGhosttyFocusRedirect(
  term: GhosttyTerminal,
  container: HTMLElement,
): () => void {
  applyGhosttyInputHardening(term, container);
  const textarea = term.textarea;

  const refocusContainer = (event: Event) => {
    const mouse = event as MouseEvent;
    // button===2: allow Ghostty's contextmenu handler to park the textarea
    // under the cursor for system paste without fighting focus.
    if (typeof mouse.button === "number" && mouse.button === 2) return;
    const target = event.target as HTMLElement | null;
    if (!target) return;
    const isCanvas = target.tagName === "CANVAS";
    const isTextarea = !!(textarea && (target === textarea || textarea.contains(target)));
    if (!isCanvas && !isTextarea) return;
    try {
      applyGhosttyInputHardening(term, container);
      // Only steal focus when this pane is allowed to — callers gate via
      // autoFocus, but a click on the surface always means the user wants it.
      container.focus({ preventScroll: true });
    } catch {
      /* ignore */
    }
  };

  // Capture on both mouse + touch: Ghostty open() binds canvas touchend →
  // textarea.focus(), which reintroduces the keyCode-229 path on mobile Chrome.
  container.addEventListener("mousedown", refocusContainer, true);
  container.addEventListener("touchstart", refocusContainer, { capture: true, passive: true });
  container.addEventListener("touchend", refocusContainer, { capture: true, passive: true });

  // Ghostty (or accessibility helpers) may flip contenteditable back on; watch
  // and strip it so Chromium does not re-enter composition mode mid-session.
  const mo = new MutationObserver(() => {
    if (container.getAttribute("contenteditable") === "true") {
      applyGhosttyInputHardening(term, container);
    }
  });
  try {
    mo.observe(container, { attributes: true, attributeFilter: ["contenteditable", "tabindex"] });
  } catch {
    /* ignore */
  }

  // Without contenteditable, many browsers never fire a paste event on the
  // focused container for Ctrl/Cmd+V. Ghostty's InputHandler also bails on
  // meta/ctrl+v ("let the browser handle it"), so bridge clipboard → onData.
  const onPasteKey = (event: KeyboardEvent) => {
    const isPaste =
      (event.ctrlKey || event.metaKey) &&
      !event.altKey &&
      !event.shiftKey &&
      (event.key === "v" || event.key === "V" || event.code === "KeyV");
    if (!isPaste) return;
    if (document.activeElement !== container && !container.contains(document.activeElement)) {
      return;
    }
    // Prefer the async Clipboard API; fall back is silent if permission denied.
    if (typeof navigator === "undefined" || !navigator.clipboard?.readText) return;
    event.preventDefault();
    event.stopPropagation();
    void navigator.clipboard
      .readText()
      .then((text) => {
        if (!text) return;
        try {
          term.paste(text);
        } catch {
          try {
            term.input(text, true);
          } catch {
            /* disposed */
          }
        }
      })
      .catch(() => {
        /* permission denied / insecure context — user can still right-click paste */
      });
  };
  container.addEventListener("keydown", onPasteKey, true);

  return () => {
    container.removeEventListener("mousedown", refocusContainer, true);
    container.removeEventListener("touchstart", refocusContainer, true);
    container.removeEventListener("touchend", refocusContainer, true);
    container.removeEventListener("keydown", onPasteKey, true);
    mo.disconnect();
  };
}

/**
 * Ghostty-web 0.4 never emits application mouse reports (DECSET 1000/1002/1003
 * + SGR 1006) on onData — hasMouseTracking() becomes true after the TUI enables
 * it, but clicks only drive local selection / scrollbar / alt-screen cursor-key
 * wheel. Bridge pointer events → SGR CSI sequences while tracking is on so the
 * catcode clickable chrome works in the hub.
 */
function installAppMouseBridge(
  term: GhosttyTerminal,
  container: HTMLElement,
): () => void {
  let pressedBtn: number | null = null;
  let lastCell = { col: 1, row: 1 };

  const tracking = (): boolean => {
    try {
      return term.hasMouseTracking();
    } catch {
      return false;
    }
  };

  const metrics = () => {
    const canvas = container.querySelector("canvas");
    const el = canvas ?? container;
    const rect = el.getBoundingClientRect();
    return {
      cols: term.cols,
      rows: term.rows,
      rect: { left: rect.left, top: rect.top, width: rect.width, height: rect.height },
    };
  };

  /** Push bytes through Ghostty's onData path (same pipe as keystrokes → WS). */
  const emit = (seq: string) => {
    if (!seq) return;
    try {
      term.input(seq, true);
    } catch {
      /* disposed */
    }
  };

  const isTerminalSurface = (target: EventTarget | null): boolean => {
    if (!(target instanceof HTMLElement)) return false;
    if (target.tagName === "CANVAS") return true;
    if (target === container) return true;
    // Textarea is 1×1 / pointer-events none after hardening; still allow.
    if (term.textarea && (target === term.textarea || term.textarea.contains(target))) return true;
    return false;
  };

  const onPointerDown = (event: PointerEvent) => {
    if (!tracking()) return;
    if (!isTerminalSurface(event.target)) return;
    // Let the pane toolbar / chrome keep their own clicks.
    if ((event.target as HTMLElement).closest?.("[data-pane-toolbar]")) return;

    event.preventDefault();
    event.stopPropagation();

    const cell = clientToCell(event.clientX, event.clientY, metrics());
    lastCell = cell;
    const btn = domButtonToXterm(event.button);
    pressedBtn = btn;
    try {
      container.setPointerCapture(event.pointerId);
    } catch {
      /* older Safari */
    }
    try {
      term.clearSelection();
    } catch {
      /* ignore */
    }
    emit(encodeSgrMouse(btn, cell.col, cell.row, false, modsFromEvent(event)));
  };

  const onPointerMove = (event: PointerEvent) => {
    if (!tracking() || pressedBtn === null) return;
    const cell = clientToCell(event.clientX, event.clientY, metrics());
    if (cell.col === lastCell.col && cell.row === lastCell.row) return;
    lastCell = cell;
    // Cell-motion (1002): report motion only while a button is held.
    emit(encodeSgrMouseMotion(pressedBtn, cell.col, cell.row, modsFromEvent(event)));
  };

  const onPointerUp = (event: PointerEvent) => {
    if (!tracking()) {
      pressedBtn = null;
      return;
    }
    if (pressedBtn === null) return;
    const cell = clientToCell(event.clientX, event.clientY, metrics());
    lastCell = cell;
    const btn = pressedBtn;
    pressedBtn = null;
    try {
      container.releasePointerCapture(event.pointerId);
    } catch {
      /* ignore */
    }
    emit(encodeSgrMouse(btn, cell.col, cell.row, true, modsFromEvent(event)));
  };

  const onPointerCancel = (event: PointerEvent) => {
    if (pressedBtn === null) return;
    const btn = pressedBtn;
    pressedBtn = null;
    emit(encodeSgrMouse(btn, lastCell.col, lastCell.row, true, modsFromEvent(event)));
  };

  // Capture so we beat Ghostty's SelectionManager (canvas bubble) and the
  // canvas mousedown → textarea.focus() handler while app tracking is live.
  container.addEventListener("pointerdown", onPointerDown, true);
  container.addEventListener("pointermove", onPointerMove, true);
  container.addEventListener("pointerup", onPointerUp, true);
  container.addEventListener("pointercancel", onPointerCancel, true);

  // Wheel: when tracking, send SGR wheel buttons instead of Ghostty's
  // alt-screen cursor-key emulation (\x1b[A/B) which the TUI does not want.
  const prevWheel = undefined;
  term.attachCustomWheelEventHandler((event) => {
    if (!tracking()) return false;
    event.preventDefault();
    event.stopPropagation();
    const cell = clientToCell(event.clientX, event.clientY, metrics());
    for (const seq of encodeWheelReports(
      event.deltaY,
      event.deltaX,
      cell.col,
      cell.row,
      modsFromEvent(event),
    )) {
      emit(seq);
    }
    return true;
  });

  // Focus in/out reports (DECSET 1004) — TUI sets ReportFocus; Ghostty never
  // forwards \x1b[I / \x1b[O on its own.
  const onFocus = () => {
    try {
      if (term.hasFocusEvents()) emit("\x1b[I");
    } catch {
      /* ignore */
    }
  };
  const onBlur = () => {
    try {
      if (term.hasFocusEvents()) emit("\x1b[O");
    } catch {
      /* ignore */
    }
  };
  container.addEventListener("focus", onFocus);
  container.addEventListener("blur", onBlur);

  return () => {
    container.removeEventListener("pointerdown", onPointerDown, true);
    container.removeEventListener("pointermove", onPointerMove, true);
    container.removeEventListener("pointerup", onPointerUp, true);
    container.removeEventListener("pointercancel", onPointerCancel, true);
    container.removeEventListener("focus", onFocus);
    container.removeEventListener("blur", onBlur);
    try {
      term.attachCustomWheelEventHandler(prevWheel);
    } catch {
      /* ignore */
    }
  };
}

function terminalSocketUrl(): string {
  const scheme = location.protocol === "https:" ? "wss" : "ws";
  return `${scheme}://${location.host}/api/terminal`;
}

/** Explicitly terminate a persistent server-side PTY, including when detached. */
export function terminateTerminalSession(sessionId: string, workspace: string): void {
  const socket = new WebSocket(terminalSocketUrl());
  const timeout = window.setTimeout(() => socket.close(), 3000);
  const finish = () => {
    window.clearTimeout(timeout);
    if (socket.readyState === WebSocket.OPEN || socket.readyState === WebSocket.CONNECTING) {
      socket.close();
    }
  };
  socket.onopen = () => socket.send(JSON.stringify(terminalTerminateEnvelope(sessionId, workspace)));
  socket.onmessage = finish;
  socket.onclose = () => window.clearTimeout(timeout);
  socket.onerror = finish;
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
  /** Called when the server confirms that the persistent PTY no longer exists. */
  onUnavailable?: () => void;
  /** Incremented to request a clear of the terminal scrollback+screen. */
  clearSeq?: number;
  /** What the server spawns in the PTY: the login shell (default) or the
   *  catcode TUI (used by the /hub terminal workspace). */
  launch?: TerminalLaunch;
  /** Reconnect attempts before declaring the terminal unavailable (default 6).
   *  The hub passes Infinity: server-side PTYs persist across tab closes,
   *  sign-outs and long network gaps, so a later reattach can still succeed —
   *  and a genuinely gone PTY answers "missing" to the attach-only open and
   *  surfaces via onUnavailable regardless of the budget. */
  maxReconnects?: number;
  /** When true (default), grab keyboard focus after open. The hub keeps many
   *  panes mounted (including under `display:none`) and passes `focused` so
   *  only the active pane steals focus — otherwise a hidden terminal can own
   *  document.activeElement and keystrokes never reach the visible catcode. */
  autoFocus?: boolean;
  /** When false the pane is under a hidden project tab (display:none). We skip
   *  auto-focus work and only re-fit once the tab becomes visible again —
   *  FitAddon measures 0×0 under `hidden`, which desyncs mouse cell math. */
  visible?: boolean;
}

/**
 * One Ghostty terminal session bound to a single WebSocket. Opens the WS on
 * mount, sends {type:"open",sessionId,cwd,cols,rows}, pipes data ↔ Ghostty, and
 * reports the shell exit via onExit. Disposes cleanly on unmount.
 */
export function Terminal({
  sessionId,
  workspace,
  cwd,
  onExit,
  onUnavailable,
  clearSeq,
  launch,
  maxReconnects,
  autoFocus = true,
  visible = true,
}: TerminalProps) {
  const containerRef = useRef<HTMLDivElement>(null);
  const termRef = useRef<GhosttyTerminal | null>(null);
  const fitRef = useRef<{ fit: () => void } | null>(null);
  const [connectionState, setConnectionState] = useState<ConnectionState>("initializing");
  const [termReady, setTermReady] = useState(false);
  const lastClearSeq = useRef(clearSeq ?? 0);
  const autoFocusRef = useRef(autoFocus);
  autoFocusRef.current = autoFocus;
  const visibleRef = useRef(visible);
  visibleRef.current = visible;

  // Keep latest values in refs so the effect only re-runs when the session id
  // changes (one renderer + WS attachment per session), not on parent renders.
  const workspaceRef = useRef(workspace);
  workspaceRef.current = workspace;
  const cwdRef = useRef(cwd);
  cwdRef.current = cwd;
  const onExitRef = useRef(onExit);
  onExitRef.current = onExit;
  const onUnavailableRef = useRef(onUnavailable);
  onUnavailableRef.current = onUnavailable;
  const launchRef = useRef(launch);
  launchRef.current = launch;
  const maxReconnectsRef = useRef(maxReconnects ?? DEFAULT_MAX_RECONNECTS);
  maxReconnectsRef.current = maxReconnects ?? DEFAULT_MAX_RECONNECTS;

  useEffect(() => {
    let disposed = false;
    let term: GhosttyTerminal | null = null;
    let ws: WebSocket | null = null;
    let pingTimer: ReturnType<typeof setInterval> | null = null;
    let reconnectTimer: ReturnType<typeof setTimeout> | null = null;
    let attachedOnce = false;
    let ended = false;
    let reconnectAttempts = 0;
    let lastActivityAt = Date.now();

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
          // Ink tokens (dark execution-register): ground / body / ember / raised selection
          background: "#0e0f12",
          foreground: "#d8dbe2",
          cursor: "#d68e58",
          selectionBackground: "#22252c",
        },
      });
      const fit = new ghostty.FitAddon();
      term.loadAddon(fit);
      term.open(containerRef.current);
      // Must run after open(): open() re-applies contenteditable + textarea tabindex.
      const unharden = installGhosttyFocusRedirect(term, containerRef.current);
      // Application mouse tracking bridge (Ghostty never emits SGR mouse itself).
      const unmouse = installAppMouseBridge(term, containerRef.current);
      fitRef.current = fit;
      try {
        fit.fit();
      } catch {
        /* container not sized yet */
      }
      termRef.current = term;
      setTermReady(true);
      // Only auto-focus when this pane is supposed to own the keyboard. Hub
      // mounts every project tab at once; focusing a hidden pane makes the
      // visible one appear dead to typing. Focus the container (InputHandler
      // target) — never the hidden textarea (see applyGhosttyInputHardening).
      if (autoFocusRef.current) {
        try {
          term.focus();
        } catch {
          /* ignore */
        }
      }

      const connect = (attachOnly: boolean) => {
        if (disposed || ended || !term) return;
        setConnectionState(attachOnly ? "reconnecting" : "connecting");
        const socket = new WebSocket(terminalSocketUrl());
        ws = socket;
        socket.onopen = () => {
          lastActivityAt = Date.now();
          socket.send(
            JSON.stringify(
              terminalOpenEnvelope(
                sessionId,
                workspaceRef.current,
                cwdRef.current ?? "",
                term!.cols,
                term!.rows,
                attachOnly,
                launchRef.current,
              ),
            ),
          );
        };
        socket.onmessage = (ev) => {
          const str =
            typeof ev.data === "string"
              ? ev.data
              : new TextDecoder().decode(ev.data as ArrayBuffer);
          let m: ServerMsg;
          try {
            m = JSON.parse(str) as ServerMsg;
          } catch {
            return;
          }
          lastActivityAt = Date.now();
          if (m.type === "ready") {
            attachedOnce = true;
            reconnectAttempts = 0;
            setConnectionState("connected");
          } else if (m.type === "data") {
            term!.write(m.data);
          } else if (m.type === "exit") {
            ended = true;
            onExitRef.current?.(m.code);
          } else if (m.type === "terminated" || m.type === "missing") {
            ended = true;
            setConnectionState("failed");
            onUnavailableRef.current?.();
          }
        };
        socket.onerror = () => {
          /* onclose owns retry and status transitions */
        };
        socket.onclose = () => {
          if (disposed || ended || socket !== ws) return;
          reconnectAttempts += 1;
          if (reconnectAttempts > maxReconnectsRef.current) {
            ended = true;
            setConnectionState("failed");
            onUnavailableRef.current?.();
            return;
          }
          setConnectionState("reconnecting");
          const delay = Math.min(10_000, 500 * 2 ** (reconnectAttempts - 1));
          reconnectTimer = setTimeout(() => connect(attachedOnce), delay);
        };
      };
      connect(false);

      // stdin: every Ghostty-encoded keystroke → the real PTY.
      const onDataDisp = term.onData((data) => {
        if (ws && ws.readyState === WebSocket.OPEN) {
          ws.send(JSON.stringify({ type: "data", data }));
        }
      });

      // Keepalive (some proxies close idle WSes) + half-open socket watchdog.
      // When a machine sleeps or the phone OS suspends the tab, the socket
      // dies silently (no onclose). If no server traffic arrived since the
      // last ping cycle, detach the stale socket ourselves and reattach —
      // the server-side PTY kept running the whole time.
      pingTimer = setInterval(() => {
        if (disposed || ended || !ws || ws.readyState !== WebSocket.OPEN) return;
        if (Date.now() - lastActivityAt > PONG_TIMEOUT_MS) {
          const stale = ws;
          stale.onopen = null;
          stale.onmessage = null;
          stale.onerror = null;
          stale.onclose = null;
          try { stale.close(); } catch { /* already closed */ }
          ws = null;
          reconnectAttempts += 1;
          if (reconnectAttempts > maxReconnectsRef.current) {
            ended = true;
            setConnectionState("failed");
            onUnavailableRef.current?.();
            return;
          }
          setConnectionState("reconnecting");
          const delay = Math.min(10_000, 500 * 2 ** (reconnectAttempts - 1));
          reconnectTimer = setTimeout(() => connect(attachedOnce), delay);
          return;
        }
        ws.send(JSON.stringify({ type: "ping" }));
      }, PING_INTERVAL_MS);

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
        unharden();
        unmouse();
        if (pingTimer) clearInterval(pingTimer);
        if (reconnectTimer) clearTimeout(reconnectTimer);
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
        ws.onclose = null;
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
      fitRef.current = null;
      setTermReady(false);
    };
  }, [sessionId]);

  // Clear the terminal screen + scrollback when clearSeq bumps.
  useEffect(() => {
    if (clearSeq == null || clearSeq <= (lastClearSeq.current ?? 0)) return;
    lastClearSeq.current = clearSeq;
    try {
      // Clear screen, move cursor home, and clear scrollback (3J).
      termRef.current?.write("\x1b[3J\x1b[H\x1b[2J");
    } catch {
      /* ignore */
    }
  }, [clearSeq]);

  // Take / release keyboard focus when the hub marks this pane active, and
  // re-fit after becoming visible (FitAddon often measures 0×0 under `hidden`).
  // `visible` covers project-tab switches where sibling leaves in the newly
  // shown tab are not focused but still need correct cols/rows for mouse math.
  useEffect(() => {
    const term = termRef.current;
    if (!term || !termReady) return;

    const refit = () => {
      try {
        fitRef.current?.fit();
      } catch {
        /* container may still be settling */
      }
      if (containerRef.current) {
        try {
          applyGhosttyInputHardening(term, containerRef.current);
        } catch {
          /* ignore */
        }
      }
    };

    if (!visible) {
      // Hidden tab: release focus if we somehow still own it, skip fit (0×0).
      try {
        term.blur?.();
      } catch {
        /* ignore */
      }
      return;
    }

    refit();
    if (autoFocus) {
      try {
        term.focus();
      } catch {
        /* ignore */
      }
    } else {
      try {
        term.blur?.();
      } catch {
        /* ignore */
      }
    }
    // Second pass after layout (tab un-hide / split resize) so cols/rows match.
    const t = window.setTimeout(() => {
      refit();
      if (autoFocus) {
        try {
          term.focus();
        } catch {
          /* ignore */
        }
      }
    }, 50);
    // visualViewport resize (mobile keyboard) also desyncs cell metrics.
    const onVv = () => refit();
    try {
      window.visualViewport?.addEventListener("resize", onVv);
    } catch {
      /* ignore */
    }
    return () => {
      window.clearTimeout(t);
      try {
        window.visualViewport?.removeEventListener("resize", onVv);
      } catch {
        /* ignore */
      }
    };
  }, [autoFocus, visible, termReady]);

  return (
    <div
      className="relative h-full w-full overflow-hidden bg-ink-950"
      onPointerDownCapture={() => {
        // Clicking the terminal surface must own the keyboard even if a sibling
        // pane or a previously focused hidden tab still held document focus.
        // Focus the container (Ghostty InputHandler), not the hidden textarea.
        try {
          const term = termRef.current;
          if (term && containerRef.current) applyGhosttyInputHardening(term, containerRef.current);
          term?.focus();
        } catch {
          /* ignore */
        }
      }}
    >
      <div ref={containerRef} className="h-full w-full" />
      {connectionState !== "connected" ? (
        <div
          className="pointer-events-none absolute right-2 top-2 rounded-sm border border-ink-700 bg-ink-900 px-2 py-1 font-mono text-[11px] text-ink-300"
          role="status"
        >
          {connectionState === "initializing"
            ? "Initializing terminal…"
            : connectionState === "connecting"
              ? "Connecting…"
              : connectionState === "reconnecting"
                ? "Reconnecting…"
                : "Terminal unavailable"}
        </div>
      ) : null}
    </div>
  );
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
  onUnavailable: (id: string) => void;
  onRename: (id: string, title: string) => void;
  onRestart: (id: string) => void;
}

export function TerminalPanel({
  workspace,
  sessions,
  activeId,
  onNew,
  onClose,
  onSelect,
  onExit,
  onUnavailable,
  onRename,
  onRestart,
}: TerminalPanelProps) {
  const active = sessions.find((s) => s.id === activeId) ?? null;
  const [renamingId, setRenamingId] = useState<string | null>(null);
  const [renameValue, setRenameValue] = useState("");
  const [clearSeqs, setClearSeqs] = useState<Record<string, number>>({});

  const closeWithConfirm = (id: string, title: string, alive: boolean) => {
    if (alive && !window.confirm(`Close terminal “${title}”? Its running process will be terminated.`)) return;
    terminateTerminalSession(id, workspace);
    onClose(id);
  };

  const startRename = (s: TerminalSession) => {
    setRenamingId(s.id);
    setRenameValue(s.title);
  };
  const commitRename = () => {
    if (renamingId && renameValue.trim()) onRename(renamingId, renameValue.trim());
    setRenamingId(null);
  };
  const requestClear = (id: string) => {
    setClearSeqs((prev) => ({ ...prev, [id]: (prev[id] ?? 0) + 1 }));
  };

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
            className={`group flex min-h-9 cursor-pointer items-center gap-1 rounded-t-sm px-2 py-1 font-mono text-[11px] ${
              s.id === activeId
                ? "bg-ink-950 text-ink-100"
                : "text-ink-400 hover:bg-ink-800 hover:text-ink-200"
            }`}
            title={s.cwd}
          >
            {renamingId === s.id ? (
              <input
                autoFocus
                value={renameValue}
                onChange={(e) => setRenameValue(e.target.value)}
                onClick={(e) => e.stopPropagation()}
                onBlur={commitRename}
                onKeyDown={(e) => {
                  if (e.key === "Enter") { e.preventDefault(); commitRename(); }
                  if (e.key === "Escape") { e.preventDefault(); setRenamingId(null); }
                }}
                className="w-24 rounded-sm border border-accent bg-ink-950 px-1 py-0.5 font-mono text-[11px] text-ink-100 outline-none focus-visible:ring-2 focus-visible:ring-accent"
                aria-label={`Rename ${s.title}`}
              />
            ) : (
              <span
                className="max-w-[12rem] truncate"
                onDoubleClick={(e) => { e.stopPropagation(); startRename(s); }}
              >{s.title}</span>
            )}
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
                startRename(s);
              }}
              title="Rename"
              aria-label={`Rename ${s.title}`}
              className="ml-1 hidden rounded-sm px-0.5 text-ink-500 hover:bg-ink-800 hover:text-ink-100 sm:group-hover:inline-block"
            >
              ✎
            </button>
            <button
              type="button"
              onClick={(e) => {
                e.preventDefault();
                e.stopPropagation();
                closeWithConfirm(s.id, s.title, s.alive);
              }}
              onKeyDown={(e) => {
                if (e.key === "Enter" || e.key === " ") {
                  e.preventDefault();
                  e.stopPropagation();
                  closeWithConfirm(s.id, s.title, s.alive);
                }
              }}
              className="ml-0.5 rounded-sm px-0.5 text-ink-500 hover:bg-ink-800 hover:text-ink-100 sm:opacity-0 sm:group-hover:opacity-100 sm:focus-within:opacity-100"
              aria-label={`close ${s.title}`}
            >
              ×
            </button>
          </div>
        ))}
        <button
          type="button"
          onClick={onNew}
          className="ml-auto min-h-9 rounded-sm px-3 py-1 font-mono text-[12px] text-ink-400 hover:bg-ink-800 hover:text-ink-100 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-accent"
          title="New terminal"
          aria-label="new terminal"
        >
          +
        </button>
      </div>
      {active ? (
        <div className="flex items-center gap-1 border-b border-ink-800 bg-ink-925 px-2 py-1">
          <button type="button" onClick={() => onRestart(active.id)} title="Restart terminal" aria-label="Restart terminal" className="rounded-sm px-2 py-0.5 font-mono text-[11px] text-ink-400 hover:bg-ink-800 hover:text-ink-100 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-accent">Restart</button>
          <button type="button" onClick={() => requestClear(active.id)} title="Clear terminal" aria-label="Clear terminal" className="rounded-sm px-2 py-0.5 font-mono text-[11px] text-ink-400 hover:bg-ink-800 hover:text-ink-100 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-accent">Clear</button>
          <span className="ml-auto truncate font-mono text-[10px] text-ink-600" title={active.cwd}>{active.cwd || "workspace root"}</span>
        </div>
      ) : null}
      <div className="relative min-h-0 flex-1 p-1">
        {active?.alive ? (
          <Terminal
            key={active.id}
            sessionId={active.id}
            workspace={workspace}
            cwd={active.cwd}
            onExit={(code) => onExit(active.id, code)}
            onUnavailable={() => onUnavailable(active.id)}
            clearSeq={clearSeqs[active.id]}
          />
        ) : active ? (
          <div className="flex h-full flex-col items-center justify-center gap-3 text-sm text-ink-400">
            <span>
              {active.exitCode === null
                ? "This terminal session is no longer available."
                : `Terminal exited with code ${active.exitCode}.`}
            </span>
            <button
              type="button"
              onClick={() => onRestart(active.id)}
              className="rounded-sm border border-ink-700 bg-ink-900 px-3 py-1.5 font-mono text-[12px] text-ink-300 hover:bg-ink-800 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-accent"
            >
              Restart terminal
            </button>
          </div>
        ) : (
          <div className="flex h-full items-center justify-center">
            <button
              type="button"
              onClick={onNew}
              className="rounded-sm border border-ink-700 bg-ink-900 px-3 py-1.5 font-mono text-[12px] text-ink-300 hover:bg-ink-800 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-accent"
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
