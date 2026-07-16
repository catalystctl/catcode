"use client";

// Screen panel — live VNC view of a test_env (Linux container or Windows VM).
//
// The agent obtains a `vnc_url` (ws://host:port/websockify) from the `test_env`
// tool's create/vnc_url actions. Paste it here to watch and drive the env's
// screen in real time — useful for webui/GUI tests where you want to see what
// the agent is doing, or take over interactively.
//
// noVNC's web client (vnc.html) is served by the env ITSELF over HTTP at
// http://<host>:<port>/vnc.html — NOT loaded from a CDN. This matters: a page
// served over HTTPS that opens an insecure ws:// connection is blocked as
// mixed content. Loading noVNC from the env's own HTTP server keeps the page
// and the websocket on matching schemes (http + ws), so the connection is
// allowed. (Linux container serves this via websockify --web; the Windows
// host-side websockify serves it when CATALYST_TESTENV_NOVNC_WEB is set.) For a
// native viewer, use the raw VNC port shown below.
//
// This is a USER-driven panel: it never touches the core agent loop. It accepts
// an optional `target` prop (a ws:// URL) so the shell can drive it, and falls
// back to a local address-bar input so it works standalone.

import { useCallback, useEffect, useMemo, useState } from "react";
import { MonitorIcon } from "@/components/icons";

export interface ScreenProps {
  /** Optional override: connect to this ws:// URL directly. */
  target?: string;
}

/** Parse a ws://host:port/path URL into noVNC vnc.html query params. */
function parseVncUrl(url: string): { host: string; port: string; path: string } | null {
  try {
    const u = new URL(url);
    if (u.protocol !== "ws:" && u.protocol !== "wss:") return null;
    const host = u.hostname;
    const port = u.port || (u.protocol === "wss:" ? "443" : "80");
    const path = u.pathname.replace(/^\//, "") || "websockify";
    return { host, port, path };
  } catch {
    return null;
  }
}

export function Screen({ target }: ScreenProps) {
  const external = target ?? "";
  const [addr, setAddr] = useState(external);
  const [connected, setConnected] = useState(external);

  // Keep local address/connection in sync when the shell drives `target`.
  useEffect(() => {
    const next = target ?? "";
    setAddr(next);
    setConnected(next);
  }, [target]);

  const commit = useCallback((v: string) => setConnected(v), []);

  const onConnect = useCallback(() => {
    commit(addr.trim());
  }, [addr, commit]);

  const iframeSrc = useMemo(() => {
    if (!connected) return null;
    const parsed = parseVncUrl(connected);
    if (!parsed) return null;
    const params = new URLSearchParams({
      host: parsed.host,
      port: parsed.port,
      path: parsed.path,
      autoconnect: "true",
      resize: "scale",
      reconnect: "false",
      show_dot: "true",
    });
    return `http://${parsed.host}:${parsed.port}/vnc.html?${params.toString()}`;
  }, [connected]);

  const parsed = connected ? parseVncUrl(connected) : null;
  const rawVncPort = parsed?.port; // native VNC viewers connect here

  return (
    <div className="flex h-full flex-col">
      {/* Address bar */}
      <div className="flex items-center gap-2 border-b border-ink-700 px-3 py-2">
        <MonitorIcon width={14} height={14} />
        <input
          className="flex-1 rounded border border-ink-700 bg-ink-950 px-2 py-1 text-xs focus:border-accent/50 focus:outline-none"
          placeholder="ws://localhost:6080/websockify"
          value={addr}
          onChange={(e) => setAddr(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter") onConnect();
          }}
        />
        <button
          type="button"
          className="rounded border border-ink-700 px-2 py-1 text-xs hover:bg-ink-850"
          onClick={onConnect}
        >
          Connect
        </button>
        {rawVncPort && (
          <span className="text-[10px] text-ink-500" title="port for a native VNC viewer">
            vnc :{rawVncPort}
          </span>
        )}
      </div>

      {/* noVNC viewer */}
      <div className="relative flex-1 bg-black">
        {iframeSrc ? (
          <iframe
            key={iframeSrc}
            src={iframeSrc}
            title="noVNC screen"
            className="h-full w-full border-0"
            sandbox="allow-scripts allow-same-origin allow-popups"
          />
        ) : (
          <div className="flex h-full flex-col items-center justify-center gap-2 text-center text-xs text-ink-500">
            <MonitorIcon width={32} height={32} />
            <p>No screen connected.</p>
            <p>
              Spin up a test env with the <code className="rounded bg-ink-850 px-1">test_env</code>{" "}
              tool, then paste its <code className="rounded bg-ink-850 px-1">vnc_url</code> above.
            </p>
          </div>
        )}
      </div>
    </div>
  );
}
