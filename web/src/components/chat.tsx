"use client";

// Chat — the application shell. Owns the useAgent hook and wires it to the
// sidebar, header, message list, approval gate, composer, and toasts. Handles
// auto-scroll, the empty-state hero, the API-key overlay, theme toggle,
// message edit/regenerate, transcript export, and the slash-command dispatch.

import { useCallback, useEffect, useRef, useState } from "react";
import { useAgent, type AgentApi } from "@/lib/use-agent";
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
import { ProviderLoginModal } from "./provider-login-modal";
import { DiagnosticsModal } from "./diagnostics-modal";
import { ErrorBoundary } from "./error-boundary";
import { AppDialogHost, useAppDialog } from "./app-dialog";
import { useOutsideClose, mergeRefs } from "@/lib/use-outside-close";
import { useFocusTrap } from "@/lib/use-focus-trap";
import { SparkIcon, ShieldIcon, SendIcon } from "./icons";

const EXAMPLES = [
  "Explain the architecture of this codebase.",
  "Find and fix any obvious bugs in the core.",
  "Write a unit test for the path-confinement logic.",
  "Summarize the most recent changes.",
];

/** Parse `agent "task"` / `agent 'task'` / `agent bare-task` pairs from slash args. */
function parseAgentTasks(args: string): Array<{ agent: string; task: string }> {
  const out: Array<{ agent: string; task: string }> = [];
  const re = /(\S+)\s+(?:"([^"]*)"|'([^']*)'|(\S+))/g;
  let m: RegExpExecArray | null;
  while ((m = re.exec(args))) {
    out.push({ agent: m[1], task: m[2] ?? m[3] ?? m[4] ?? "" });
  }
  return out;
}

function buildRunPrompt(agent: string, task: string): string {
  return `Run the subagent tool: agent="${agent}", task="${task}". Return its result.`;
}

function buildParallelPrompt(tasks: Array<{ agent: string; task: string }>): string {
  const lines = tasks.map((t) => `- agent="${t.agent}" task="${t.task}"`).join("\n");
  return `Run the subagent tool in parallel mode with these tasks:\n${lines}`;
}

function buildChainPrompt(tasks: Array<{ agent: string; task: string }>): string {
  const lines = tasks.map((t) => `- agent="${t.agent}" task="${t.task}"`).join("\n");
  return `Run the subagent tool as a chain with these steps (use {previous} to pass the prior step's output):\n${lines}`;
}

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

export function Chat({ agent: injected, docked }: { agent?: AgentApi; docked?: boolean } = {}) {
  const ownAgent = useAgent();
  const agent = injected ?? ownAgent;
  const { state } = agent;
  const { confirm, prompt, dialog } = useAppDialog();
  const dialogApi = useRef({ confirm, prompt });
  useEffect(() => {
    dialogApi.current = { confirm, prompt };
  }, [confirm, prompt]);
  const [sidebarOpen, setSidebarOpen] = useState(false);
  const [keyInput, setKeyInput] = useState("");
  const [keyBusy, setKeyBusy] = useState(false);
  const [keyDismissed, setKeyDismissed] = useState(false);
  const scrollRef = useRef<HTMLDivElement>(null);
  const composerRef = useRef<ComposerHandle>(null);
  const [autoScroll, setAutoScroll] = useState(true);
  const [modal, setModal] = useState<
    null | "memory" | "plugins" | "settings" | "subagents" | "help" | "goal" | "login" | "logout" | "diagnostics"
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

  // Re-fetch goal status once the core is ready (covers reconnect / mid-goal resume).
  useEffect(() => {
    if (state.ready) void agent.goalStatus();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [state.ready != null]);

  const openDiagnostics = useCallback(() => {
    const a = agentRef.current;
    void a.stats();
    void a.context();
    void a.usage(a.state.selectedModel ?? undefined);
    setModal("diagnostics");
  }, []);

  // Auto-scroll to the bottom while streaming / when HITL gates appear,
  // unless the user scrolled up.
  useEffect(() => {
    if (!autoScroll) return;
    const el = scrollRef.current;
    if (el) el.scrollTop = el.scrollHeight;
  }, [
    state.messages,
    state.pendingApproval,
    state.pendingAsk,
    state.pendingSudo,
    state.pendingIntercom,
    state.pendingOauth,
    state.goalMode,
    state.goalPlan,
    autoScroll,
  ]);

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
  // `args` is the remainder after the slash token (e.g. `/run scout "bugs"` →
  // action "run", args `scout "bugs"`).
  const onCommand = useCallback((name: string, args?: string) => {
    void (async () => {
      const a = agentRef.current;
      const d = dialogApi.current;
      switch (name) {
        case "reset": {
          const ok = await d.confirm({
            title: "Reset session",
            message: "Reset the conversation and session file? This cannot be undone.",
            confirmLabel: "Reset",
            danger: true,
          });
          if (ok) await a.reset();
          return;
        }
        case "compact": {
          let instr = args?.trim();
          if (!instr) {
            const typed = await d.prompt({
              title: "Compact context",
              message:
                "Optional: what should compaction preserve? Leave blank for the default summary.",
              placeholder: "e.g. Focus on code samples and API usage",
              multiline: true,
              confirmLabel: "Compact",
            });
            if (typed === null) return;
            instr = typed.trim() || undefined;
          }
          await a.compact(instr || undefined);
          return;
        }
        case "context":
        case "usage":
        case "stats":
          openDiagnostics();
          return;
        case "new":
          return void a.newSession();
        case "abort":
          return void a.abort();
        case "sessions":
          return void a.listSessions();
        case "undo":
          return void a.undo();
        case "clear": {
          const ok = await d.confirm({
            title: "Clear view",
            message: "Clear the conversation view? The session file is kept.",
            confirmLabel: "Clear",
          });
          if (ok) await a.clear();
          return;
        }
        case "memory":
          a.listMemory();
          return setModal("memory");
        case "remember": {
          let note = args?.trim();
          if (!note) {
            const typed = await d.prompt({
              title: "Remember",
              message: "A durable note for future sessions.",
              placeholder: "Remember what?",
              multiline: true,
              required: true,
              confirmLabel: "Save",
            });
            if (!typed?.trim()) return;
            note = typed.trim();
          }
          void a.saveMemory(note);
          a.listMemory();
          return setModal("memory");
        }
        case "forget":
          a.listMemory();
          return setModal("memory");
        case "plugins":
          a.listPlugins();
          return setModal("plugins");
        case "auto-compact": {
          const on = !(a.state.ready?.auto_compact ?? true);
          return void a.setConfig("auto_compact", on);
        }
        case "sandbox": {
          const mode = (args ?? "").trim().toLowerCase();
          if (mode === "none" || mode === "firejail" || mode === "seatbelt") {
            return void a.setConfig("sandbox", mode);
          }
          void a.getVisionConfig();
          return setModal("settings");
        }
        case "settings":
        case "vision":
          void a.getVisionConfig();
          return setModal("settings");
        case "model": {
          const id = args?.trim();
          if (id) {
            const match =
              a.state.models.find((m) => m.id === id) ||
              a.state.models.find((m) => m.id.endsWith(`/${id}`) || m.name === id);
            if (match) {
              a.setModel(match.id);
              return;
            }
          }
          void a.getVisionConfig();
          return setModal("settings");
        }
        case "reasoning": {
          const level = args?.trim().toLowerCase();
          if (level && ["off", "low", "medium", "high", "xhigh", "max"].includes(level)) {
            a.setThinking(level);
            return;
          }
          void a.getVisionConfig();
          return setModal("settings");
        }
        case "approval": {
          const mode = args?.trim().toLowerCase();
          if (mode === "never" || mode === "destructive" || mode === "always") {
            return void a.setApproval(mode);
          }
          void a.getVisionConfig();
          return setModal("settings");
        }
        case "subagents":
          void a.listAgents();
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
          const key = await d.prompt({
            title: "API key",
            message: "Enter an API key for the current provider.",
            placeholder: "sk-…",
            password: true,
            required: true,
            confirmLabel: "Save key",
          });
          if (key?.trim()) void a.setKey(key.trim());
          return;
        }
                case "oauth-code": {
          const code = await d.prompt({
            title: "OAuth code",
            message: "Paste the OAuth code or final callback URL.",
            placeholder: "code or https://…",
            required: true,
            confirmLabel: "Submit",
          });
          if (code?.trim()) void a.submitOauthCode(code.trim());
          return;
        }
        case "search-key": {
          let provider = (args ?? "").trim().toLowerCase();
          if (provider !== "exa" && provider !== "tavily") {
            const picked = await d.prompt({
              title: "Search API key provider",
              message: "Which search provider? Type exa or tavily.",
              placeholder: "exa",
              confirmLabel: "Next",
            });
            provider = (picked ?? "").trim().toLowerCase();
            if (provider !== "exa" && provider !== "tavily") return;
          }
          const key = await d.prompt({
            title: `${provider === "exa" ? "Exa" : "Tavily"} API key`,
            message: `Paste your ${provider} search API key (leave blank to clear).`,
            placeholder: provider === "exa" ? "exa-key…" : "tvly-key…",
            password: true,
            confirmLabel: "Save",
          });
          if (key !== null) void a.setSearchKey(provider, key.trim());
          return;
        }
        case "login":
          void a.listProviderPresets();
          return setModal("login");
        case "logout":
          void a.listProviderPresets();
          return setModal("logout");
        case "steer": {
          const msg = args?.trim();
          if (msg) {
            void a.steer(msg);
            return;
          }
          setSidebarOpen(false);
          return composerRef.current?.focus();
        }
        case "attach":
          return composerRef.current?.openAttach();
        case "goal":
          void a.goalStatus();
          return setModal("goal");
        case "cancel-goal":
          return void a.cancelGoal();
        case "index": {
          const incremental =
            /\b(--incremental|-i)\b/.test(args ?? "") ||
            (args ?? "").trim() === "incremental";
          const task = incremental
            ? "Run an incremental knowledge index of this repository. Use `git status` + `git diff --name-only` to find files changed since the last index; for each changed area, read it and use the `memory` tool (action: append) to UPDATE the relevant existing memories — architecture, conventions, APIs, gotchas — rather than creating duplicates. If a changed file reveals a new subsystem with no memory yet, save a new one. Then list the memories you touched. Be concise: only persist what genuinely changed."
            : "Run a full knowledge index of this repository to bootstrap learning. Walk the top-level layout, read README/package-manifest/entry points/config/tests, and identify the architecture, major subsystems, conventions, reusable patterns, build/test/deploy steps, and gotchas. Use the `memory` tool (action: save) to persist each as a durable, named memory (types: architecture/convention/api/gotcha/build). Then use `list_dir .catalyst-code/skills/` and, for any reusable workflow you solved 2+ times that has no skill yet, write a candidate SKILL.md under `.catalyst-code/skills/<name>/` with write_file (frontmatter: name/description; body: when-to-use + steps + example). End by listing the memories and any candidate skills you created, and name one area you are least confident about.";
          return void a.prompt(task);
        }
        case "reflect":
          return void a.prompt(
            "Reflect on the work done in this session so far. Identify: (1) any convention, architecture fact, decision, or gotcha worth persisting so future sessions don't rediscover it, and (2) any repetitive pattern you performed more than once that should become a reusable skill under `.catalyst-code/skills/`. Use the `memory` tool (action: append if a topic memory exists, else save) to persist durable facts only — skip transient task state. If you wrote a skill, name it. Finish with a two-line summary: what you learned and what you persisted.",
          );
        case "bash-timeout": {
          const n = Number((args ?? "").trim());
          if (Number.isFinite(n) && n > 0) return void a.setConfig("bash_timeout_secs", n);
          void a.getVisionConfig();
          return setModal("settings");
        }
        case "run": {
          let raw = args?.trim();
          if (!raw) {
            raw =
              (await d.prompt({
                title: "Delegate to a subagent",
                message: 'Format: agent "task"',
                placeholder: 'scout "find bugs"',
                required: true,
                confirmLabel: "Run",
              })) ?? "";
          }
          const parsed = parseAgentTasks(raw.trim());
          if (parsed.length === 0) {
            if (raw.trim()) {
              const sp = raw.trim().indexOf(" ");
              if (sp > 0) {
                void a.prompt(
                  buildRunPrompt(
                    raw.trim().slice(0, sp),
                    raw.trim().slice(sp + 1).replace(/^["']|["']$/g, ""),
                  ),
                );
              }
            }
            return;
          }
          void a.prompt(buildRunPrompt(parsed[0].agent, parsed[0].task));
          return;
        }
        case "parallel": {
          let raw = args?.trim();
          if (!raw) {
            raw =
              (await d.prompt({
                title: "Run subagents in parallel",
                message: 'Format: a1 "t1" a2 "t2"',
                placeholder: 'scout "scan" worker "fix"',
                required: true,
                confirmLabel: "Run",
              })) ?? "";
          }
          const parsed = parseAgentTasks(raw.trim());
          if (parsed.length === 0) return;
          void a.prompt(buildParallelPrompt(parsed));
          return;
        }
        case "chain": {
          let raw = args?.trim();
          if (!raw) {
            raw =
              (await d.prompt({
                title: "Run a subagent chain",
                message: 'Format: a1 "t1" a2 "t2" — use {previous} in later tasks',
                placeholder: 'scout "map auth" worker "fix using {previous}"',
                required: true,
                confirmLabel: "Run",
              })) ?? "";
          }
          const parsed = parseAgentTasks(raw.trim());
          if (parsed.length === 0) return;
          void a.prompt(buildChainPrompt(parsed));
          return;
        }
        default:
          return;
      }
    })();
  }, [doExport, openDiagnostics]);

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
    void a.undo().then((ok) => {
      if (ok) void a.prompt(newText);
    });
  }, []);

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
      void a.undo().then((ok) => {
        if (ok) void a.prompt(lastUserText);
      });
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
    <div className={`chat-panel relative flex min-h-0 min-w-0 ${docked ? "h-full" : "h-[100dvh]"} w-full overflow-hidden bg-ink-950 bg-grid text-ink-100`}>
      <Sidebar
        embedded={docked}
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
        onReset={() => void onCommand("reset")}
        onCompact={() => void onCommand("compact")}
        onStats={openDiagnostics}
        onOpenPanel={(p) => {
          if (p === "memory") agent.listMemory();
          if (p === "plugins") agent.listPlugins();
          if (p === "subagents") void agent.listAgents();
          if (p === "settings") void agent.getVisionConfig();
          setModal(p as "memory" | "plugins" | "settings" | "subagents" | "help");
        }}
        onSwitchWorkspace={(p) => agent.switchWorkspace(p)}
        onRemoveProject={(p) => agent.removeProject(p)}
        onDeleteSession={(p) => agent.deleteSession(p)}
        onRenameSession={(name, title) => agent.renameSession(name, title)}
        onConfirmDelete={async (title) =>
          dialogApi.current.confirm({
            title: "Delete session",
            message: `Delete session "${title}"? The .jsonl file will be permanently removed.`,
            confirmLabel: "Delete",
            danger: true,
          })
        }
      />

      <div className="flex min-w-0 flex-1 flex-col">
        <Header
          compact={docked}
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
          {/* HITL first so empty-session OAuth/sudo/ask aren't below a full-height hero. */}
          {!switching && (
            <div className="mx-auto max-w-3xl">
              {state.pendingApproval && (
                <div className="mx-4 mb-2 mt-3 sm:mx-6">
                  <Approval approval={state.pendingApproval} onApprove={agent.approve} />
                </div>
              )}
              {state.pendingIntercom && (
                <div className="mx-4 mb-2 mt-3 sm:mx-6">
                  <IntercomPrompt
                    prompt={state.pendingIntercom}
                    onReply={agent.intercomReply}
                    onDismiss={() => agent.intercomReply("(skipped — no decision provided)")}
                  />
                </div>
              )}
              {state.pendingAsk && (
                <div className="mx-4 mb-2 mt-3 sm:mx-6">
                  <AskFlyout
                    prompt={state.pendingAsk}
                    onSubmit={(answers) => agent.askReply(answers)}
                    onSkip={() => agent.askReply(null)}
                  />
                </div>
              )}
              {state.pendingSudo && (
                <div className="mx-4 mb-2 mt-3 sm:mx-6">
                  <SudoPrompt
                    prompt={state.pendingSudo}
                    onApprove={(password) => agent.sudoReply(true, password)}
                    onDecline={() => agent.sudoReply(false)}
                  />
                </div>
              )}
              {state.pendingOauth && (
                <div className="mx-4 mb-2 mt-3 sm:mx-6">
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
                  <div className="mx-4 mb-2 mt-3 sm:mx-6">
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
                        void dialogApi.current
                          .prompt({
                            title: "Revise plan",
                            message: "What should change in the plan?",
                            multiline: true,
                            required: true,
                            confirmLabel: "Revise",
                          })
                          .then((fb) => {
                            if (fb?.trim()) void agent.reviseGoal(fb.trim());
                          });
                      }}
                      onCancel={() => void agent.cancelGoal()}
                    />
                  </div>
                )}
              {state.goalMode &&
                state.goalMode.phase !== "idle" &&
                state.goalMode.phase !== "plan_ready" && (
                  <div className="mx-4 mb-2 mt-3 sm:mx-6">
                    <GoalStatusChip
                      phase={state.goalMode.phase}
                      goal={state.goalMode.goal}
                      onCancel={() => void agent.cancelGoal()}
                    />
                  </div>
                )}
            </div>
          )}

          {empty || switching ? (
            <EmptyState
              workspace={state.workspace}
              connected={agent.connected}
              switching={switching}
              canSend={!!currentModel && agent.connected}
              compact={
                !!(
                  state.pendingApproval ||
                  state.pendingAsk ||
                  state.pendingSudo ||
                  state.pendingIntercom ||
                  state.pendingOauth ||
                  state.goalMode
                )
              }
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
          compact={docked}
          streaming={state.streaming}
          followUpQueued={state.followUpQueued}
          hitlOpen={
            !!(
              state.pendingApproval ||
              state.pendingAsk ||
              state.pendingSudo ||
              state.pendingIntercom ||
              state.pendingOauth
            )
          }
          connected={agent.connected && !switching}
          canSend={!!currentModel && !switching}
          thinkingLevel={state.thinkingLevel}
          modelLabel={modelLabel}
          images={images}
          workspace={state.workspace}
          onAddImage={onAddImage}
          onRemoveImage={onRemoveImage}
          onPrompt={sendPrompt}
          onSteer={(t) => agent.steer(t)}
          onAbort={agent.abort}
          onClearQueue={agent.clearQueue}
          onCommand={onCommand}
          skills={state.skills}
          onSkill={(name, task) => agent.applySkill(name, task)}
          onBash={(command, exclude) => void agent.userBash(command, exclude)}
        />
      </div>

      <Toasts toasts={state.toasts} onDismiss={agent.dismissToast} />
      <AppDialogHost dialog={dialog} />

      {modal === "memory" && (
        <MemoryPanel
          memories={state.memories}
          onSave={agent.saveMemory}
          onForget={agent.forgetMemory}
          onRefresh={() => void agent.refreshMemory()}
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
        <SubagentsPanel
          runs={state.subagentRuns}
          agents={state.availableAgents}
          onRefreshAgents={() => void agent.listAgents()}
          onClose={() => setModal(null)}
        />
      )}
      {modal === "settings" && (
        <SettingsModal
          ready={state.ready}
          models={state.models}
          selectedModel={state.selectedModel}
          thinkingLevel={state.thinkingLevel}
          approvalMode={state.approvalMode}
          autoCompact={state.ready?.auto_compact ?? true}
          sandbox={state.ready?.sandbox ?? "none"}
          onSelectModel={agent.setModel}
          onSelectThinking={agent.setThinking}
          onSetApproval={agent.setApproval}
          onSetBashTimeout={(secs) => agent.setConfig("bash_timeout_secs", secs)}
          onSetAutoCompact={(on) => void agent.setConfig("auto_compact", on)}
          onSetSandbox={(mode) => void agent.setConfig("sandbox", mode)}
          visionConfig={state.visionConfig}
          onSetVisionConfig={(vision_model, vision_models) =>
            void agent.setVisionConfig(vision_model, vision_models)
          }
          onRefreshVision={() => void agent.getVisionConfig()}
          onClose={() => setModal(null)}
        />
      )}
      {modal === "diagnostics" && (
        <DiagnosticsModal
          stats={state.stats}
          context={state.contextBreakdown}
          usage={state.usageSnapshot}
          onRefresh={() => {
            void agent.stats();
            void agent.context();
            void agent.usage(agent.state.selectedModel ?? undefined);
          }}
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
      {(modal === "login" || modal === "logout") && (
        <ProviderLoginModal
          presets={state.providerPresets}
          mode={modal}
          onLoginKey={(id, key) => void agent.login(id, key)}
          onLoginOauth={(id) => void agent.loginOauth(id)}
          onLoginSaved={(id) => void agent.login(id)}
          onSwitchProvider={(id) => void agent.setProvider(id)}
          onLogout={(id) => void agent.logout(id)}
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
  canSend,
  compact,
  onPick,
}: {
  workspace: string;
  connected: boolean;
  switching: boolean;
  canSend: boolean;
  compact?: boolean;
  onPick: (t: string) => void;
}) {
  return (
    <div
      className={`flex flex-col items-center justify-center px-6 text-center ${
        compact ? "min-h-0 py-6" : "h-full py-10"
      }`}
    >
      <div className="mb-5 flex h-16 w-16 items-center justify-center rounded-2xl bg-gradient-to-br from-accent to-accent-deep text-3xl font-bold text-white shadow-glow">
        c
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
              disabled={!canSend}
              onClick={() => {
                if (!canSend) return;
                onPick(ex);
              }}
              className="group flex items-center gap-3 rounded-xl border border-ink-800 bg-ink-900/40 px-4 py-3 text-left text-[13px] text-ink-300 transition-all hover:border-accent/40 hover:bg-ink-850 hover:text-ink-100 disabled:cursor-not-allowed disabled:opacity-40 disabled:hover:border-ink-800 disabled:hover:bg-ink-900/40 disabled:hover:text-ink-300"
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
  const closeRef = useOutsideClose(onDismiss);
  const trapRef = useFocusTrap<HTMLDivElement>();
  return (
    <div className="modal-backdrop">
      <div
        ref={mergeRefs(closeRef, trapRef)}
        className="modal-sheet modal-sheet-auto relative max-w-md p-6"
        role="dialog"
        aria-modal="true"
        aria-label="Connect your provider"
      >
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
          Paste a key here, or use <code className="font-mono text-ink-400">/login</code> for OAuth.
        </p>
      </div>
    </div>
  );
}
