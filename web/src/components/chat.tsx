"use client";

// Chat — the application shell. Owns the useAgent hook and wires it to the
// sidebar, header, message list, approval gate, composer, and toasts. Handles
// auto-scroll, the empty-state hero, the API-key overlay, theme toggle,
// message edit/regenerate, transcript export, and the slash-command dispatch.

import { useCallback, useEffect, useRef, useState } from "react";
import { useAgent } from "@/lib/use-agent";
import { basename } from "@/lib/format";
import { Sidebar } from "./sidebar";
import { Header } from "./header";
import { Message } from "./message";
import { Composer } from "./composer";
import { Toasts } from "./toasts";
import { Approval } from "./approval";
import { IntercomPrompt, SubagentPanel } from "./intercom";
import { MemoryPanel } from "./memory";
import { PluginsPanel } from "./plugins";
import { SettingsModal } from "./settings";
import { HelpModal } from "./help-modal";
import { ErrorBoundary } from "./error-boundary";
import { SparkIcon, ShieldIcon, SendIcon } from "./icons";

const EXAMPLES = [
  "Explain the architecture of this codebase.",
  "Find and fix any obvious bugs in the core.",
  "Write a unit test for the path-confinement logic.",
  "Summarize the most recent changes.",
];

function lsGet(k: string): string | null {
  try {
    return typeof localStorage !== "undefined" ? localStorage.getItem(k) : null;
  } catch {
    return null;
  }
}
function lsSet(k: string, v: string): void {
  try {
    if (typeof localStorage !== "undefined") localStorage.setItem(k, v);
  } catch {
    /* ignore */
  }
}

export function Chat() {
  const agent = useAgent();
  const { state } = agent;
  const [sidebarOpen, setSidebarOpen] = useState(false);
  const [keyInput, setKeyInput] = useState("");
  const [keyBusy, setKeyBusy] = useState(false);
  const scrollRef = useRef<HTMLDivElement>(null);
  const [autoScroll, setAutoScroll] = useState(true);
  const [modal, setModal] = useState<null | "memory" | "plugins" | "settings" | "subagents" | "help">(null);
  const [images, setImages] = useState<string[]>([]);
  const [theme, setTheme] = useState<string>(() => lsGet("umans:theme") ?? "dark");

  // Theme: toggle a data-theme attribute + persist. CSS variables adjust.
  useEffect(() => {
    document.documentElement.setAttribute("data-theme", theme);
    lsSet("umans:theme", theme);
  }, [theme]);

  // Auto-scroll to the bottom while streaming, unless the user scrolled up.
  useEffect(() => {
    if (!autoScroll) return;
    const el = scrollRef.current;
    if (el) el.scrollTop = el.scrollHeight;
  }, [state.messages, state.pendingApproval, autoScroll]);

  const onScroll = () => {
    const el = scrollRef.current;
    if (!el) return;
    const atBottom = el.scrollHeight - el.scrollTop - el.clientHeight < 90;
    setAutoScroll(atBottom);
  };

  // ── Export transcript as a downloadable markdown file ──
  const doExport = useCallback(() => {
    const md = agent.exportTranscript();
    const blob = new Blob([md], { type: "text/markdown" });
    const url = URL.createObjectURL(blob);
    const a = document.createElement("a");
    a.href = url;
    a.download = `umans-transcript-${new Date().toISOString().slice(0, 10)}.md`;
    document.body.appendChild(a);
    a.click();
    document.body.removeChild(a);
    URL.revokeObjectURL(url);
  }, [agent]);

  // ── Slash-command dispatch (single switch; the catalog is the source of truth) ──
  const onCommand = useCallback(
    (name: string) => {
      switch (name) {
        case "reset":
          return agent.reset();
        case "compact":
          return agent.compact();
        case "new":
          return agent.newSession();
        case "abort":
          return agent.abort();
        case "stats":
          return agent.stats();
        case "sessions":
          return agent.listSessions();
        case "undo":
          return agent.undo();
        case "clear":
          return agent.clear();
        case "memory":
        case "remember":
        case "forget":
          agent.listMemory();
          return setModal("memory");
        case "plugins":
          agent.listPlugins();
          return setModal("plugins");
        case "settings":
        case "model":
        case "reasoning":
        case "approval":
        case "vision":
          return setModal("settings");
        case "subagents":
          return setModal("subagents");
        case "help":
          return setModal("help");
        case "copy":
          return agent.copyLastReply();
        case "export":
          return doExport();
        case "theme":
          return setTheme((t) => (t === "dark" ? "light" : "dark"));
        case "key": {
          const key = window.prompt("Enter API key:");
          if (key?.trim()) void agent.setKey(key.trim());
          return;
        }
        case "steer":
          // Focus the composer for a steer; the placeholder guides the user.
          return setSidebarOpen(false);
        case "run":
          return agent.prompt("Delegate to a subagent: ");
        case "parallel":
          return agent.prompt("Run subagents in parallel: ");
        case "chain":
          return agent.prompt("Run a subagent chain: ");
        default:
          return;
      }
    },
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [agent],
  );

  const onAddImage = (url: string) => setImages((prev) => [...prev, url]);
  const onRemoveImage = (i: number) => setImages((prev) => prev.filter((_, idx) => idx !== i));
  const sendPrompt = async (text: string, imgs?: string[]) => {
    setImages([]);
    await agent.prompt(text, imgs);
  };

  const submitKey = async () => {
    if (!keyInput.trim()) return;
    setKeyBusy(true);
    await agent.setKey(keyInput.trim());
    setKeyBusy(false);
    setKeyInput("");
  };

  // ── Edit a user message: undo the last turn, then re-send the edited text ──
  const onEditUser = useCallback(
    (newText: string) => {
      void agent.undo().then(() => agent.prompt(newText));
    },
    [agent],
  );

  // ── Regenerate: undo the last turn, re-send the same prompt ──
  const onRegenerate = useCallback(() => {
    const msgs = state.messages;
    let lastUserText = "";
    for (let i = msgs.length - 1; i >= 0; i--) {
      if (msgs[i].role === "user") {
        lastUserText = (msgs[i as number] as { text: string }).text;
        break;
      }
    }
    if (lastUserText) {
      void agent.undo().then(() => agent.prompt(lastUserText));
    }
  }, [agent, state.messages]);

  // Compute indices for edit/regenerate affordances (only the latest of each).
  const messages = state.messages;
  let lastUserIdx = -1;
  let lastAssistantIdx = -1;
  for (let i = messages.length - 1; i >= 0; i--) {
    if (lastUserIdx < 0 && messages[i].role === "user") lastUserIdx = i;
    if (lastAssistantIdx < 0 && messages[i].role === "assistant") lastAssistantIdx = i;
    if (lastUserIdx >= 0 && lastAssistantIdx >= 0) break;
  }

  const needKey = state.ready != null && state.authed === false;
  const currentModel = state.models.find((m) => m.id === state.selectedModel) ?? state.models[0];
  const modelLabel = currentModel?.name ?? currentModel?.id ?? "no model";
  const empty = state.messages.length === 0;
  const switching = state.switching;

  return (
    <div className="flex h-[100dvh] w-full overflow-hidden bg-ink-950 bg-grid text-ink-100">
      <Sidebar
        open={sidebarOpen}
        onClose={() => setSidebarOpen(false)}
        workspace={state.workspace}
        projects={state.projects}
        switching={switching}
        sessions={state.sessions}
        currentSessionFile={state.currentSessionFile}
        stats={state.stats}
        onNewSession={() => {
          agent.newSession();
          setSidebarOpen(false);
        }}
        onLoadSession={(p) => {
          agent.loadSession(p);
          setSidebarOpen(false);
        }}
        onReset={agent.reset}
        onCompact={agent.compact}
        onStats={agent.stats}
        onOpenPanel={(p) => {
          if (p === "memory") agent.listMemory();
          if (p === "plugins") agent.listPlugins();
          setModal(p as "memory" | "plugins" | "settings" | "subagents" | "help");
        }}
        onSwitchWorkspace={(p) => agent.switchWorkspace(p)}
        onAddProject={(p) => agent.addProject(p)}
        onRenameSession={(name, title) => agent.renameSession(name, title)}
      />

      <div className="flex min-w-0 flex-1 flex-col">
        <Header
          connected={agent.connected}
          workspace={state.workspace}
          provider={state.provider}
          models={state.models}
          selectedModel={state.selectedModel}
          thinkingLevel={state.thinkingLevel}
          approvalMode={state.approvalMode}
          metrics={state.metrics}
          streaming={state.streaming}
          retrying={state.retrying}
          sessionFile={state.currentSessionFile}
          switching={switching}
          theme={theme}
          onMenuClick={() => setSidebarOpen(true)}
          onSelectModel={agent.setModel}
          onSelectThinking={agent.setThinking}
          onSetApproval={agent.setApproval}
          onReconnect={agent.reconnect}
          onToggleTheme={() => setTheme((t) => (t === "dark" ? "light" : "dark"))}
        />

        {/* Messages */}
        <div ref={scrollRef} onScroll={onScroll} className="relative flex-1 overflow-y-auto">
          {empty || switching ? (
            <EmptyState
              workspace={state.workspace}
              connected={agent.connected}
              switching={switching}
              onPick={(t) => agent.prompt(t)}
            />
          ) : (
            <div className="mx-auto max-w-3xl py-4">
              <ErrorBoundary label="message list">
                {state.messages.map((m, i) => (
                  <Message
                    key={m.id}
                    m={m}
                    canEdit={i === lastUserIdx && !state.streaming}
                    canRegenerate={i === lastAssistantIdx && !state.streaming}
                    onEditUser={onEditUser}
                    onRegenerate={onRegenerate}
                  />
                ))}
              </ErrorBoundary>
              {state.pendingApproval && (
                <div className="mx-4 mb-2 sm:mx-6">
                  <Approval approval={state.pendingApproval} onApprove={agent.approve} />
                </div>
              )}
              {state.pendingIntercom && (
                <div className="mx-4 mb-2 sm:mx-6">
                  <IntercomPrompt
                    prompt={state.pendingIntercom}
                    onReply={agent.intercomReply}
                    onDismiss={() => agent.intercomReply("")}
                  />
                </div>
              )}
              <div className="h-4" />
            </div>
          )}

          {!autoScroll && !empty && !switching && (
            <button
              onClick={() => {
                setAutoScroll(true);
                const el = scrollRef.current;
                if (el) el.scrollTop = el.scrollHeight;
              }}
              className="sticky bottom-3 left-1/2 z-10 mx-auto flex -translate-x-1/2 items-center gap-1.5 rounded-full border border-ink-700 bg-ink-900/90 px-3 py-1.5 text-[12px] text-ink-200 shadow-lg backdrop-blur hover:bg-ink-850"
            >
              ↓ Jump to latest
            </button>
          )}
        </div>

        <Composer
          streaming={state.streaming}
          connected={agent.connected}
          canSend={!!currentModel}
          thinkingLevel={state.thinkingLevel}
          modelLabel={modelLabel}
          images={images}
          onAddImage={onAddImage}
          onRemoveImage={onRemoveImage}
          onPrompt={sendPrompt}
          onSteer={(t) => agent.steer(t)}
          onAbort={agent.abort}
          onCommand={onCommand}
        />
      </div>

      <Toasts toasts={state.toasts} onDismiss={agent.dismissToast} />

      {modal === "memory" && (
        <MemoryPanel
          memories={state.memories}
          onSave={agent.saveMemory}
          onForget={agent.forgetMemory}
          onClose={() => setModal(null)}
        />
      )}
      {modal === "plugins" && (
        <PluginsPanel
          plugins={state.plugins}
          onInstall={agent.installPlugin}
          onRemove={agent.removePlugin}
          onEnable={agent.enablePlugin}
          onDisable={agent.disablePlugin}
          onClose={() => setModal(null)}
        />
      )}
      {modal === "subagents" && (
        <SubagentPanel log={state.intercomLog} onClose={() => setModal(null)} />
      )}
      {modal === "settings" && (
        <SettingsModal
          ready={state.ready}
          models={state.models}
          selectedModel={state.selectedModel}
          thinkingLevel={state.thinkingLevel}
          approvalMode={state.approvalMode}
          onSelectModel={agent.setModel}
          onSelectThinking={agent.setThinking}
          onSetApproval={agent.setApproval}
          onSetBashTimeout={(secs) => agent.setConfig("bash_timeout_secs", secs)}
          onClose={() => setModal(null)}
        />
      )}
      {modal === "help" && <HelpModal onClose={() => setModal(null)} />}

      {needKey && (
        <KeyOverlay value={keyInput} busy={keyBusy} onChange={setKeyInput} onSubmit={submitKey} />
      )}
    </div>
  );
}

function EmptyState({
  workspace,
  connected,
  switching,
  onPick,
}: {
  workspace: string;
  connected: boolean;
  switching: boolean;
  onPick: (t: string) => void;
}) {
  return (
    <div className="flex h-full flex-col items-center justify-center px-6 py-10 text-center">
      <div className="mb-5 flex h-16 w-16 items-center justify-center rounded-2xl bg-gradient-to-br from-accent to-accent-deep text-3xl font-bold text-white shadow-glow">
        u
      </div>
      <h1 className="text-2xl font-semibold tracking-tight text-ink-100">Umans Harness</h1>
      <p className="mt-2 max-w-md text-[14px] text-ink-400">
        {switching ? (
          "Switching workspace…"
        ) : (
          <>
            An agentic coding companion running on{" "}
            <span className="font-mono text-accent-soft">
              {basename(workspace) || workspace || "this workspace"}
            </span>
            .{connected ? " Ask it to build, debug, explore, or explain." : " Connecting…"}
          </>
        )}
      </p>
      {!switching && (
        <div className="mt-7 grid w-full max-w-lg gap-2">
          {EXAMPLES.map((ex) => (
            <button
              key={ex}
              onClick={() => onPick(ex)}
              className="group flex items-center gap-3 rounded-xl border border-ink-800 bg-ink-900/40 px-4 py-3 text-left text-[13px] text-ink-300 transition-all hover:border-accent/40 hover:bg-ink-850 hover:text-ink-100"
            >
              <SparkIcon width={14} height={14} className="shrink-0 text-ink-500 group-hover:text-accent-soft" />
              <span className="flex-1">{ex}</span>
              <SendIcon width={13} height={13} className="shrink-0 text-ink-600 group-hover:text-accent-soft" />
            </button>
          ))}
        </div>
      )}
    </div>
  );
}

function KeyOverlay({
  value,
  busy,
  onChange,
  onSubmit,
}: {
  value: string;
  busy: boolean;
  onChange: (v: string) => void;
  onSubmit: () => void;
}) {
  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/60 p-4 backdrop-blur-sm">
      <div className="w-full max-w-md rounded-2xl border border-ink-700 bg-ink-900 p-6 shadow-2xl animate-fade-in">
        <div className="mb-4 flex items-center gap-3">
          <span className="flex h-10 w-10 items-center justify-center rounded-xl bg-accent/15 text-accent-soft">
            <ShieldIcon width={18} height={18} />
          </span>
          <div>
            <h2 className="text-[15px] font-semibold text-ink-100">Connect your provider</h2>
            <p className="text-[12px] text-ink-400">Enter an API key to start chatting.</p>
          </div>
        </div>
        <input
          type="password"
          autoFocus
          value={value}
          onChange={(e) => onChange(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter") onSubmit();
          }}
          placeholder="sk-..."
          className="w-full rounded-xl border border-ink-700 bg-ink-950 px-3.5 py-2.5 font-mono text-[13px] text-ink-100 placeholder:text-ink-600 focus:border-accent/50 focus:outline-none focus:shadow-glow"
        />
        <button
          onClick={onSubmit}
          disabled={busy || !value.trim()}
          className="mt-3 w-full rounded-xl bg-accent py-2.5 text-[13px] font-semibold text-white transition-colors hover:bg-accent-soft disabled:cursor-not-allowed disabled:bg-ink-800 disabled:text-ink-500"
        >
          {busy ? "Connecting…" : "Connect"}
        </button>
        <p className="mt-3 text-center text-[11px] text-ink-500">
          Or set <code className="font-mono text-ink-400">UMANS_API_KEY</code> before launching the server.
        </p>
      </div>
    </div>
  );
}
