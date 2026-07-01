"use client";

// Chat — the application shell. Owns the useAgent hook and wires it to the
// sidebar, header, message list, approval gate, composer, and toasts. Handles
// auto-scroll, the empty-state hero, and the API-key overlay shown when the
// core reports no key configured.

import { useEffect, useRef, useState } from "react";
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
import { SparkIcon, ShieldIcon, SendIcon } from "./icons";

const EXAMPLES = [
  "Explain the architecture of this codebase.",
  "Find and fix any obvious bugs in the core.",
  "Write a unit test for the path-confinement logic.",
  "Summarize the most recent changes.",
];

export function Chat() {
  const agent = useAgent();
  const { state } = agent;
  const [sidebarOpen, setSidebarOpen] = useState(false);
  const [keyInput, setKeyInput] = useState("");
  const [keyBusy, setKeyBusy] = useState(false);
  const scrollRef = useRef<HTMLDivElement>(null);
  const [autoScroll, setAutoScroll] = useState(true);
  const [modal, setModal] = useState<null | "memory" | "plugins" | "settings" | "subagents">(null);
  const [images, setImages] = useState<string[]>([]);

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

  const onCommand = (name: string) => {
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
        agent.listMemory();
        return setModal("memory");
      case "plugins":
        agent.listPlugins();
        return setModal("plugins");
      case "settings":
        return setModal("settings");
      case "subagents":
        return setModal("subagents");
    }
  };

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

  const needKey = state.ready != null && state.authed === false;
  const currentModel = state.models.find((m) => m.id === state.selectedModel) ?? state.models[0];
  const modelLabel = currentModel?.name ?? currentModel?.id ?? "no model";
  const empty = state.messages.length === 0;

  return (
    <div className="flex h-[100dvh] w-full overflow-hidden bg-ink-950 bg-grid text-ink-100">
      <Sidebar
        open={sidebarOpen}
        onClose={() => setSidebarOpen(false)}
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
          setModal(p as "memory" | "plugins" | "settings" | "subagents");
        }}
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
          onMenuClick={() => setSidebarOpen(true)}
          onSelectModel={agent.setModel}
          onSelectThinking={agent.setThinking}
          onSetApproval={agent.setApproval}
        />

        {/* Messages */}
        <div
          ref={scrollRef}
          onScroll={onScroll}
          className="relative flex-1 overflow-y-auto"
        >
          {empty ? (
            <EmptyState
              workspace={state.workspace}
              connected={agent.connected}
              onPick={(t) => agent.prompt(t)}
            />
          ) : (
            <div className="mx-auto max-w-3xl py-4">
              {state.messages.map((m) => (
                <Message key={m.id} m={m} />
              ))}
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

          {!autoScroll && !empty && (
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

      {needKey && (
        <KeyOverlay
          value={keyInput}
          busy={keyBusy}
          onChange={setKeyInput}
          onSubmit={submitKey}
        />
      )}
    </div>
  );
}

function EmptyState({
  workspace,
  connected,
  onPick,
}: {
  workspace: string;
  connected: boolean;
  onPick: (t: string) => void;
}) {
  return (
    <div className="flex h-full flex-col items-center justify-center px-6 py-10 text-center">
      <div className="mb-5 flex h-16 w-16 items-center justify-center rounded-2xl bg-gradient-to-br from-accent to-accent-deep text-3xl font-bold text-white shadow-glow">
        u
      </div>
      <h1 className="text-2xl font-semibold tracking-tight text-ink-100">Umans Harness</h1>
      <p className="mt-2 max-w-md text-[14px] text-ink-400">
        An agentic coding companion running on{" "}
        <span className="font-mono text-accent-soft">{basename(workspace) || workspace || "this workspace"}</span>.
        {connected ? " Ask it to build, debug, explore, or explain." : " Connecting…"}
      </p>
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
