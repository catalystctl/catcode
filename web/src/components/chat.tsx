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
import { Composer, type ComposerHandle } from "./composer";
import { Toasts } from "./toasts";
import { Approval } from "./approval";
import { IntercomPrompt } from "./intercom";
import { AskFlyout } from "./ask";
import { SudoPrompt } from "./sudo-prompt";
import { OauthPromptBanner } from "./oauth-prompt";
import { WorkStatePanel } from "./work-state";
import { SubagentsPanel } from "./subagents";
import { MemoryPanel } from "./memory";
import { PluginsPanel } from "./plugins";
import { SettingsModal } from "./settings";
import { HelpModal } from "./help-modal";
import { GoalModal, GoalPlanBanner, GoalStatusChip } from "./goal-modal";
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
  const [keyDismissed, setKeyDismissed] = useState(false);
  const scrollRef = useRef<HTMLDivElement>(null);
  const composerRef = useRef<ComposerHandle>(null);
  const [autoScroll, setAutoScroll] = useState(true);
  const [modal, setModal] = useState<
    null | "memory" | "plugins" | "settings" | "subagents" | "help" | "goal"
  >(null);
  const [images, setImages] = useState<string[]>([]);
  const [theme, setTheme] = useState<string>(() => lsGet("umans:theme") ?? "dark");

  // Refs so the edit/regenerate/command callbacks can stay stable (empty deps)
  // — this keeps <Message> memoized: only the streaming message re-renders on
  // each token, not the whole conversation.
  const agentRef = useRef(agent);
  useEffect(() => {
    agentRef.current = agent;
  }, [agent]);
  const msgsRef = useRef(state.messages);
  useEffect(() => {
    msgsRef.current = state.messages;
  }, [state.messages]);
  const streamingRef = useRef(state.streaming);
  useEffect(() => {
    streamingRef.current = state.streaming;
  }, [state.streaming]);

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
    const md = agentRef.current.exportTranscript();
    const blob = new Blob([md], { type: "text/markdown" });
    const url = URL.createObjectURL(blob);
    const a = document.createElement("a");
    a.href = url;
    a.download = `catalyst-code-transcript-${new Date().toISOString().slice(0, 10)}.md`;
    document.body.appendChild(a);
    a.click();
    document.body.removeChild(a);
    URL.revokeObjectURL(url);
  }, []);

  // ── Slash-command dispatch (single switch; the catalog is the source of truth) ──
  // Uses refs so this callback is stable — the Composer never re-renders from it.
  const onCommand = useCallback((name: string) => {
    const a = agentRef.current;
    switch (name) {
      case "reset":
        if (window.confirm("Reset the conversation and session file? This cannot be undone."))
          return a.reset();
        return;
      case "compact": {
        const instr = window.prompt(
          "Optional: what should compaction preserve?\n(e.g. “Focus on code samples and API usage”)\nLeave blank for the default summary.",
        );
        return a.compact(instr?.trim() || undefined);
      }
      case "context":
        return a.context();
      case "new":
        return a.newSession();
      case "abort":
        return a.abort();
      case "stats":
        return a.stats();
      case "sessions":
        return a.listSessions();
      case "undo":
        return a.undo();
      case "clear":
        if (window.confirm("Clear the conversation view? The session file is kept."))
          return a.clear();
        return;
      case "memory":
        a.listMemory();
        return setModal("memory");
      case "remember": {
        // Quick inline save, then open the panel so the user can tag/forget.
        const note = window.prompt("Remember what? A durable note for future sessions.");
        if (note?.trim()) void a.saveMemory(note.trim());
        a.listMemory();
        return setModal("memory");
      }
      case "forget":
        a.listMemory();
        return setModal("memory");
      case "plugins":
        a.listPlugins();
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
        return a.copyLastReply();
      case "export":
        return doExport();
      case "theme":
        return setTheme((t) => (t === "dark" ? "light" : "dark"));
      case "key": {
        const key = window.prompt("Enter API key:");
        if (key?.trim()) void a.setKey(key.trim());
        return;
      }
      case "oauth-code": {
        // Complete a pending no-browser OAuth login (the /login flow printed a
        // URL; paste its code or final localhost callback URL here).
        const code = window.prompt("Paste the OAuth code or final callback URL:");
        if (code?.trim()) void a.submitOauthCode(code.trim());
        return;
      }
      case "login": {
        // Pick a preset to log in to. After choosing, prompt for the key only
        // when none is available from the environment (the core resolves env
        // keys automatically when no key is passed). A logged-in provider can
        // be re-keyed here to override a bad env var (e.g. fix a 401).
        a.listProviderPresets();
        const presets = a.state?.providerPresets ?? [];
        const opts = presets
          .map((p) => `${p.loggedIn ? "✓ " : "  "}${p.label} — ${p.description}`)
          .join("\n");
        const idx = window.prompt(
          `Log in / switch provider. Pick by number:\n\n${opts}`,
        );
        const n = Number(idx);
        if (Number.isNaN(n) || n < 0 || n >= presets.length) return;
        const p = presets[n];
        if (p.loggedIn) {
          // Already logged in — offer to override the key (empty = just switch).
          const key = window.prompt(
            `${p.label} is logged in. Paste a new key to OVERRIDE it\n` +
              `(e.g. to fix a bad ${p.envVar} that caused a 401).\n` +
              `Leave blank to just switch to it.`,
          );
          if (key === null) return; // cancelled
          if (key.trim()) {
            void a.login(p.id, key.trim());
          } else {
            void a.setProvider?.(p.id);
          }
          return;
        }
        const key = p.hasKey ? undefined : window.prompt(`Paste ${p.envVar}:`);
        if (!p.hasKey && !key?.trim()) return;
        void a.login(p.id, key?.trim() || undefined);
        return;
      }
      case "logout": {
        const presets = a.state?.providerPresets ?? [];
        const loggedIn = presets.filter((p) => p.loggedIn);
        if (loggedIn.length === 0) {
          window.alert("Not logged into any provider.");
          return;
        }
        const opts = loggedIn.map((p) => `  ${p.label}`).join("\n");
        const idx = window.prompt(`Log out of which provider? Pick by number:\n\n${opts}`);
        const n = Number(idx);
        if (Number.isNaN(n) || n < 0 || n >= loggedIn.length) return;
        void a.logout(loggedIn[n].id);
        return;
      }
      case "steer":
        // Focus the composer so the user can type a steer (Enter steers while streaming).
        setSidebarOpen(false);
        return composerRef.current?.focus();
      case "attach":
        return composerRef.current?.openAttach();
      case "goal":
        return setModal("goal");
      case "cancel-goal":
        return void a.cancelGoal();
      case "run":
        composerRef.current?.insert("Delegate to a subagent: ");
        return;
      case "parallel":
        composerRef.current?.insert("Run these subagents in parallel: ");
        return;
      case "chain":
        composerRef.current?.insert("Run a subagent chain: ");
        return;
      default:
        return;
    }
  }, [doExport]);

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
  // Stable (empty deps) via refs so <Message> memo isn't defeated.
  const onEditUser = useCallback((newText: string) => {
    const a = agentRef.current;
    void a.undo().then(() => a.prompt(newText));
  }, []);

  // ── Regenerate: undo the last turn, re-send the same prompt ──
  const onRegenerate = useCallback(() => {
    const a = agentRef.current;
    const msgs = msgsRef.current;
    let lastUserText = "";
    for (let i = msgs.length - 1; i >= 0; i--) {
      if (msgs[i].role === "user") {
        lastUserText = (msgs[i] as { text: string }).text;
        break;
      }
    }
    if (lastUserText) {
      void a.undo().then(() => a.prompt(lastUserText));
    }
  }, []);

  // Compute indices for edit/regenerate affordances (only the latest of each).
  const messages = state.messages;
  let lastUserIdx = -1;
  let lastAssistantIdx = -1;
  for (let i = messages.length - 1; i >= 0; i--) {
    if (lastUserIdx < 0 && messages[i].role === "user") lastUserIdx = i;
    if (lastAssistantIdx < 0 && messages[i].role === "assistant") lastAssistantIdx = i;
    if (lastUserIdx >= 0 && lastAssistantIdx >= 0) break;
  }

  const needKey = state.ready != null && state.authed === false && !keyDismissed;
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
        onRemoveProject={(p) => agent.removeProject(p)}
        onDeleteSession={(p) => agent.deleteSession(p)}
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
          umansConc={state.umansConc}
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

        {state.workState && <WorkStatePanel ws={state.workState} />}

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
                    onDismiss={() => agent.intercomReply("(skipped — no decision provided)")}
                  />
                </div>
              )}
              {state.pendingAsk && (
                <div className="mx-4 mb-2 sm:mx-6">
                  <AskFlyout
                    prompt={state.pendingAsk}
                    onSubmit={(answers) => agent.askReply(answers)}
                    onSkip={() => agent.askReply(null)}
                  />
                </div>
              )}
              {state.pendingSudo && (
                <div className="mx-4 mb-2 sm:mx-6">
                  <SudoPrompt
                    prompt={state.pendingSudo}
                    onApprove={(password) => agent.sudoReply(true, password)}
                    onDecline={() => agent.sudoReply(false)}
                  />
                </div>
              )}
              {state.pendingOauth && (
                <div className="mx-4 mb-2 sm:mx-6">
                  <OauthPromptBanner
                    prompt={state.pendingOauth}
                    onSubmit={agent.submitOauthCode}
                    onDismiss={agent.dismissOauth}
                  />
                </div>
              )}
              {state.goalMode &&
                state.goalMode.phase === "plan_ready" &&
                !state.goalMode.auto_deploy && (
                  <div className="mx-4 mb-2 sm:mx-6">
                    <GoalPlanBanner
                      goal={state.goalMode.goal}
                      summary={state.goalPlan?.summary}
                      steps={
                        state.goalMode.prompts.map((p) => ({
                          agent: p.agent,
                          title: p.title || p.step_id,
                        })) || []
                      }
                      onApprove={() => void agent.approveGoalPlan()}
                      onRevise={() => {
                        const fb = window.prompt("What should change in the plan?");
                        if (fb?.trim()) void agent.reviseGoal(fb.trim());
                      }}
                      onCancel={() => void agent.cancelGoal()}
                    />
                  </div>
                )}
              {state.goalMode &&
                state.goalMode.phase !== "idle" &&
                state.goalMode.phase !== "plan_ready" && (
                  <div className="mx-4 mb-2 sm:mx-6">
                    <GoalStatusChip
                      phase={state.goalMode.phase}
                      goal={state.goalMode.goal}
                      onCancel={() => void agent.cancelGoal()}
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
          ref={composerRef}
          streaming={state.streaming}
          connected={agent.connected}
          canSend={!!currentModel}
          thinkingLevel={state.thinkingLevel}
          modelLabel={modelLabel}
          images={images}
          workspace={state.workspace}
          onAddImage={onAddImage}
          onRemoveImage={onRemoveImage}
          onPrompt={sendPrompt}
          onSteer={(t) => agent.steer(t)}
          onAbort={agent.abort}
          onCommand={onCommand}
          skills={state.skills}
          onSkill={(name, task) => agent.applySkill(name, task)}
          onBash={(command, exclude) => void agent.userBash(command, exclude)}
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
        <SubagentsPanel runs={state.subagentRuns} onClose={() => setModal(null)} />
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
      {modal === "goal" && (
        <GoalModal
          models={state.models}
          providerPresets={state.providerPresets}
          providers={state.ready?.providers ?? []}
          onStart={(opts) => void agent.startGoal(opts)}
          onClose={() => setModal(null)}
        />
      )}

      {needKey && (
        <KeyOverlay
          value={keyInput}
          busy={keyBusy}
          onChange={setKeyInput}
          onSubmit={submitKey}
          onDismiss={() => setKeyDismissed(true)}
        />
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
      <h1 className="text-2xl font-semibold tracking-tight text-ink-100">Catalyst Code</h1>
      <p className="mt-2 max-w-md text-[14px] text-ink-400">
        {switching ? (
          "Loading session…"
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
  onDismiss,
}: {
  value: string;
  busy: boolean;
  onChange: (v: string) => void;
  onSubmit: () => void;
  onDismiss: () => void;
}) {
  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/60 p-4 backdrop-blur-sm">
      <div className="relative w-full max-w-md rounded-2xl border border-ink-700 bg-ink-900 p-6 shadow-2xl animate-fade-in">
        <button
          onClick={onDismiss}
          className="absolute right-3 top-3 rounded-md p-1 text-ink-500 transition-colors hover:bg-ink-800 hover:text-ink-100"
          aria-label="Dismiss"
          title="Dismiss (use /login to enter a key later)"
        >
          <svg width={16} height={16} viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth={2} strokeLinecap="round" strokeLinejoin="round"><path d="M18 6L6 18M6 6l12 12" /></svg>
        </button>
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
