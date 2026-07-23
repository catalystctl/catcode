"use client";

// ControlCenterPanel — autonomous CEO mission-control surface.
//
// Kick off a mission with start_goal(ceo_mode=true, auto_deploy=true).
// The parent model is the CEO; no user approval / revise prompts.
// Only user control while running: Abort (cancel_goal).

import { useMemo, useState } from "react";
import type {
  GoalIterationRecord,
  GoalModeState,
  GoalPlan,
  GoalPrompt,
  GoalVerdict,
  ModelInfo,
  ProviderPreset,
  SubagentRunView,
} from "@/lib/types";
import type { GoalStartOpts } from "./goal-modal";
import { useOutsideClose, mergeRefs } from "@/lib/use-outside-close";
import { useFocusTrap } from "@/lib/use-focus-trap";
import { useBodyScrollLock } from "@/lib/use-body-scroll-lock";
import { truncate } from "@/lib/format";
import { BoltIcon, CheckIcon, StopIcon, WarningIcon, XIcon, ChevronRight } from "./icons";
import { RunCard, RunDetail } from "./subagents";

const CEO_PHASES = [
  "planning",
  "reviewing",
  "plan_ready",
  "deploying",
  "running",
  "verifying",
  "replanning",
  "synthesizing",
  "done",
  "failed",
] as const;

function phaseLabel(phase: string, certified?: boolean): string {
  const p = (phase || "").toLowerCase();
  if (certified && (p === "done" || p === "synthesizing")) return "Certified-Done";
  const map: Record<string, string> = {
    planning: "Planning",
    reviewing: "Reviewing",
    plan_ready: "Deploying",
    deploying: "Deploying",
    running: "Deploying",
    verifying: "Verifying",
    replanning: "Replanning",
    synthesizing: "Verifying",
    done: "Certified-Done",
    failed: "Failed",
    idle: "Idle",
  };
  return map[p] || phase || "Idle";
}

function phasePillClass(phase: string, certified?: boolean): string {
  const p = (phase || "").toLowerCase();
  if (p === "failed") return "border border-danger text-danger";
  if (certified || p === "done") return "border border-success text-success";
  if (p === "verifying" || p === "reviewing") return "border border-accent text-accent-soft";
  if (p === "deploying" || p === "running" || p === "plan_ready") return "border border-warning text-warning";
  if (p === "planning" || p === "replanning" || p === "synthesizing")
    return "bg-ink-800 text-ink-200";
  return "bg-ink-850 text-ink-400";
}

function isActivePhase(phase: string | undefined): boolean {
  if (!phase) return false;
  return !["idle", "done", "failed"].includes(phase.toLowerCase());
}

function VerdictCard({
  title,
  verdict,
  gaps,
}: {
  title: string;
  verdict: GoalVerdict | null | undefined;
  gaps?: string[];
}) {
  if (!verdict) {
    return (
      <div className="rounded-sm border border-ink-800 bg-ink-900 px-3 py-2.5">
        <div className="text-[10px] font-mono uppercase tracking-wider text-ink-500">{title}</div>
        <p className="mt-1 text-[12px] text-ink-600">Waiting…</p>
      </div>
    );
  }
  return (
    <div
      className={`rounded-sm border border-ink-800 bg-ink-900 px-3 py-2.5 border-l-2 ${
        verdict.ok ? "border-l-success" : "border-l-danger"
      }`}
    >
      <div className="flex items-center gap-2">
        <span className="text-[10px] font-mono uppercase tracking-wider text-ink-500">{title}</span>
        <span
          className={`ml-auto flex items-center gap-1 text-[11px] font-semibold ${
            verdict.ok ? "text-success" : "text-danger"
          }`}
        >
          {verdict.ok ? <CheckIcon width={12} height={12} /> : <WarningIcon width={12} height={12} />}
          {verdict.ok ? "PASS" : "FAIL"}
        </span>
      </div>
      {verdict.summary && (
        <p className="mt-1.5 whitespace-pre-wrap text-[12px] leading-relaxed text-ink-200">
          {truncate(verdict.summary, 600)}
        </p>
      )}
      {gaps && gaps.length > 0 && (
        <ul className="mt-2 list-inside list-disc space-y-0.5 text-[12px] text-warning">
          {gaps.map((g, i) => (
            <li key={i}>{g}</li>
          ))}
        </ul>
      )}
      {verdict.evidence_paths && verdict.evidence_paths.length > 0 && (
        <div className="mt-2 flex flex-wrap gap-1">
          {verdict.evidence_paths.map((p) => (
            <code
              key={p}
              className="rounded-sm bg-ink-950 px-1.5 py-0.5 font-mono text-[10px] text-ink-400"
            >
              {p}
            </code>
          ))}
        </div>
      )}
    </div>
  );
}

function PromptRunRow({
  prompt,
  run,
  onOpenRun,
}: {
  prompt: GoalPrompt;
  run?: SubagentRunView;
  onOpenRun?: (id: string) => void;
}) {
  const status = String(prompt.status || "pending").toLowerCase();
  const statusCls =
    status === "failed"
      ? "text-danger"
      : status === "done"
        ? "text-success"
        : status === "skipped"
          ? "text-ink-500"
          : status === "running" || status === "in_progress"
            ? "text-warning"
            : "text-ink-400";
  return (
    <button
      type="button"
      disabled={!prompt.run_id || !onOpenRun}
      onClick={() => prompt.run_id && onOpenRun?.(prompt.run_id)}
      className={`flex w-full items-start gap-2 rounded-sm border border-ink-800 bg-ink-900 px-2.5 py-2 text-left ${
        prompt.run_id && onOpenRun ? "hover:border-accent/60 hover:bg-ink-850" : ""
      }`}
    >
      <div className="min-w-0 flex-1">
        <div className="flex items-center gap-2">
          <span className="font-mono text-[12px] font-semibold text-accent-soft">
            {prompt.agent || "agent"}
          </span>
          <span className={`text-[11px] uppercase ${statusCls}`}>{status}</span>
          {run?.state === "running" && (
            <span className="text-[10px] text-warning">live</span>
          )}
        </div>
        <div className="mt-0.5 truncate text-[12px] text-ink-300">
          {prompt.title || prompt.task || prompt.step_id}
        </div>
        {prompt.summary && (
          <div className="mt-1 line-clamp-2 text-[11px] text-ink-500">
            {prompt.summary}
          </div>
        )}
      </div>
      {prompt.run_id && onOpenRun && (
        <ChevronRight width={14} height={14} className="mt-1 shrink-0 text-ink-600" />
      )}
    </button>
  );
}

function IterationBlock({
  record,
  isCurrent,
  runs,
  onOpenRun,
}: {
  record: GoalIterationRecord;
  isCurrent: boolean;
  runs: Record<string, SubagentRunView>;
  onOpenRun: (id: string) => void;
}) {
  const prompts = record.prompts ?? [];
  return (
    <div
      className={`rounded-sm border border-ink-800 bg-ink-900 px-3 py-2.5 ${
        isCurrent ? "border-l-2 border-l-accent" : ""
      }`}
    >
      <div className="mb-2 flex items-center gap-2">
        <span className="text-[12px] font-semibold text-ink-100">
          Iteration {record.iteration}
        </span>
        {typeof record.plan_revision === "number" && (
          <span className="text-[11px] text-ink-500">
            plan rev {record.plan_revision}
          </span>
        )}
        {isCurrent && (
          <span className="rounded-sm border border-accent px-1.5 py-0.5 text-[10px] font-medium text-accent-soft">
            current
          </span>
        )}
        {record.certified && (
          <span className="ml-auto text-[11px] font-medium text-success">certified</span>
        )}
      </div>
      {record.review_verdict && (
        <div className="mb-2">
          <VerdictCard title="Self-review" verdict={record.review_verdict} />
        </div>
      )}
      {prompts.length > 0 && (
        <div className="mb-2 space-y-1.5">
          <div className="text-[10px] font-mono uppercase tracking-wider text-ink-500">
            Agent runs
          </div>
          {prompts.map((p) => (
            <PromptRunRow
              key={p.step_id}
              prompt={p}
              run={p.run_id ? runs[p.run_id] : undefined}
              onOpenRun={onOpenRun}
            />
          ))}
        </div>
      )}
      {record.verify_verdict && (
        <VerdictCard
          title="Verify"
          verdict={record.verify_verdict}
          gaps={record.remaining_gaps}
        />
      )}
    </div>
  );
}

interface Props {
  models: ModelInfo[];
  providerPresets: ProviderPreset[];
  providers: string[];
  goalMode: GoalModeState | null;
  goalPlan: GoalPlan | null;
  goalIterations: GoalIterationRecord[];
  subagentRuns: Record<string, SubagentRunView>;
  onStart: (opts: GoalStartOpts) => void;
  onAbort: () => void;
  onClose: () => void;
}

export function ControlCenterPanel({
  models,
  providerPresets,
  providers,
  goalMode,
  goalPlan,
  goalIterations,
  subagentRuns,
  onStart,
  onAbort,
  onClose,
}: Props) {
  const closeRef = useOutsideClose(onClose);
  const trapRef = useFocusTrap<HTMLDivElement>();
  useBodyScrollLock();

  const [goal, setGoal] = useState("");
  const [concurrency, setConcurrency] = useState(4);
  const [maxTasks, setMaxTasks] = useState(8);
  const [selectedProviders, setSelectedProviders] = useState<Set<string>>(new Set());
  const [selectedModels, setSelectedModels] = useState<Set<string>>(new Set());
  const [advanced, setAdvanced] = useState(false);
  const [plannerModel, setPlannerModel] = useState("");
  const [workerModel, setWorkerModel] = useState("");
  const [reviewerModel, setReviewerModel] = useState("");
  const [selectedRunId, setSelectedRunId] = useState<string | null>(null);

  const active = isActivePhase(goalMode?.phase);
  const ceoActive = !!goalMode?.ceo_mode || active;
  const phase = goalMode?.phase ?? "idle";
  const certified = !!goalMode?.certified;

  const providerOpts = useMemo(() => {
    if (providers.length > 0) return providers;
    return providerPresets
      .filter((p) => p.loggedIn || p.configured)
      .map((p) => p.id);
  }, [providers, providerPresets]);

  const modelOpts = useMemo(() => {
    if (selectedProviders.size === 0) return models;
    return models.filter(
      (m) =>
        !m.provider ||
        [...selectedProviders].some((p) => p.toLowerCase() === m.provider?.toLowerCase()),
    );
  }, [models, selectedProviders]);

  const toggle = (set: Set<string>, id: string, setter: (s: Set<string>) => void) => {
    const next = new Set(set);
    if (next.has(id)) next.delete(id);
    else next.add(id);
    setter(next);
  };

  const submit = () => {
    const g = goal.trim();
    if (!g || active) return;
    const c = Math.max(1, Math.min(32, concurrency));
    const mt = Math.max(1, Math.min(64, maxTasks));
    const opts: GoalStartOpts = {
      goal: g,
      concurrency: Math.min(c, mt),
      max_tasks: mt,
      allowed_models: selectedModels.size ? [...selectedModels] : undefined,
      allowed_providers: selectedProviders.size ? [...selectedProviders] : undefined,
      // Fully autonomous — never wait for approve_goal_plan.
      auto_deploy: true,
      ceo_mode: true,
      // Pinned Control Center defaults (core also defaults these when ceo_mode).
      max_iterations: 3,
      max_plan_revisions: 2,
    };
    if (advanced) {
      if (plannerModel) opts.planner_model = plannerModel;
      if (workerModel) opts.worker_model = workerModel;
      if (reviewerModel) opts.reviewer_model = reviewerModel;
    }
    onStart(opts);
  };

  const selectedRun = selectedRunId ? subagentRuns[selectedRunId] : null;

  const iterations = useMemo(() => {
    const list = [...(goalIterations ?? [])];
    if (goalMode?.ceo_mode && typeof goalMode.iteration === "number") {
      const has = list.some((r) => r.iteration === goalMode.iteration);
      if (!has) {
        list.push({
          iteration: goalMode.iteration,
          plan_revision: goalMode.plan_revision,
          review_verdict: goalMode.review_verdict,
          verify_verdict: goalMode.verify_verdict,
          remaining_gaps: goalMode.remaining_gaps,
          prompts: goalMode.prompts,
          certified: goalMode.certified,
        });
      }
    }
    return list.sort((a, b) => a.iteration - b.iteration);
  }, [goalIterations, goalMode]);

  const liveRuns = useMemo(() => {
    const ids = new Set(goalMode?.active_run_ids ?? []);
    for (const p of goalMode?.prompts ?? []) {
      if (p.run_id) ids.add(p.run_id);
    }
    return Object.values(subagentRuns)
      .filter((r) => ids.has(r.id) || r.state === "running")
      .sort((a, b) => (b.startedAt || 0) - (a.startedAt || 0));
  }, [goalMode, subagentRuns]);

  return (
    <div className="modal-backdrop">
      <div
        ref={mergeRefs(closeRef, trapRef)}
        className="modal-sheet flex max-h-[min(92vh,56rem)] max-w-3xl flex-col"
        role="dialog"
        aria-modal="true"
        aria-label="Control Center"
      >
        <div className="flex items-center gap-2 border-b border-ink-800 px-4 py-3">
          <BoltIcon width={16} height={16} className="text-accent-soft" />
          <span className="text-[15px] font-semibold text-ink-100">Control Center</span>
          <span
            className={`rounded-sm px-2 py-0.5 text-[11px] font-medium ${phasePillClass(phase, certified)}`}
          >
            {phaseLabel(phase, certified)}
          </span>
          {goalMode?.ceo_mode && (
            <span className="hidden text-[11px] text-ink-500 sm:inline">
              iter {(goalMode.iteration ?? 0)}/{goalMode.max_iterations ?? 3}
              {typeof goalMode.plan_revision === "number"
                ? ` · plan ${goalMode.plan_revision}/${goalMode.max_plan_revisions ?? 2}`
                : ""}
            </span>
          )}
          <div className="ml-auto flex items-center gap-1.5">
            {active && (
              <button
                type="button"
                onClick={onAbort}
                className="inline-flex items-center gap-1.5 rounded-sm bg-danger/90 px-2.5 py-1 text-[11px] font-medium text-white hover:bg-danger"
              >
                <StopIcon width={12} height={12} />
                Abort mission
              </button>
            )}
            <button
              type="button"
              onClick={onClose}
              className="flex h-6 w-6 items-center justify-center rounded-sm text-ink-400 hover:bg-ink-800 hover:text-ink-100"
              aria-label="Close"
            >
              <XIcon width={16} height={16} />
            </button>
          </div>
        </div>

        {selectedRun ? (
          <div className="min-h-0 flex-1 overflow-hidden">
            <RunDetail run={selectedRun} onBack={() => setSelectedRunId(null)} />
          </div>
        ) : (
          <div className="min-h-0 flex-1 space-y-4 overflow-y-auto p-4">
            {/* Kickoff */}
            {!active && (
              <section className="space-y-3">
                <div>
                  <label className="mb-1 block text-[10px] font-mono uppercase tracking-wider text-ink-500">
                    Mission prompt
                  </label>
                  <textarea
                    value={goal}
                    onChange={(e) => setGoal(e.target.value)}
                    rows={4}
                    placeholder="Describe the outcome. The CEO will plan, self-review, deploy employees, verify, and replan until certified — without asking you."
                    className="w-full resize-y rounded-sm border border-ink-700 bg-ink-950 px-3 py-2.5 text-[13px] leading-relaxed text-ink-100 placeholder:text-ink-600 focus:border-accent/60 focus:outline-none"
                  />
                </div>
                <div className="grid grid-cols-2 gap-3">
                  <label className="block text-[12px] text-ink-400">
                    Concurrency
                    <input
                      type="number"
                      min={1}
                      max={32}
                      value={concurrency}
                      onChange={(e) => setConcurrency(Number(e.target.value) || 1)}
                      className="mt-1 w-full rounded-sm border border-ink-700 bg-ink-950 px-2.5 py-1.5 font-mono text-[13px] text-ink-100 focus:border-accent/60 focus:outline-none"
                    />
                  </label>
                  <label className="block text-[12px] text-ink-400">
                    Max tasks
                    <input
                      type="number"
                      min={1}
                      max={64}
                      value={maxTasks}
                      onChange={(e) => setMaxTasks(Number(e.target.value) || 1)}
                      className="mt-1 w-full rounded-sm border border-ink-700 bg-ink-950 px-2.5 py-1.5 font-mono text-[13px] text-ink-100 focus:border-accent/60 focus:outline-none"
                    />
                  </label>
                </div>
                {providerOpts.length > 0 && (
                  <div>
                    <div className="mb-1 text-[10px] font-mono uppercase tracking-wider text-ink-500">
                      Providers (optional allowlist)
                    </div>
                    <div className="flex flex-wrap gap-1.5">
                      {providerOpts.map((id) => (
                        <button
                          key={id}
                          type="button"
                          onClick={() => toggle(selectedProviders, id, setSelectedProviders)}
                          className={`rounded-sm border px-2 py-1 font-mono text-[11px] ${
                            selectedProviders.has(id)
                              ? "border-accent text-accent-soft"
                              : "border-ink-700 bg-ink-850 text-ink-400 hover:text-ink-200"
                          }`}
                        >
                          {id}
                        </button>
                      ))}
                    </div>
                  </div>
                )}
                {modelOpts.length > 0 && (
                  <div>
                    <div className="mb-1 text-[10px] font-mono uppercase tracking-wider text-ink-500">
                      Models (optional allowlist)
                    </div>
                    <div className="flex max-h-28 flex-wrap gap-1.5 overflow-y-auto">
                      {modelOpts.slice(0, 40).map((m) => (
                        <button
                          key={m.id}
                          type="button"
                          onClick={() => toggle(selectedModels, m.id, setSelectedModels)}
                          className={`rounded-sm border px-2 py-1 font-mono text-[11px] ${
                            selectedModels.has(m.id)
                              ? "border-accent text-accent-soft"
                              : "border-ink-700 bg-ink-850 text-ink-400 hover:text-ink-200"
                          }`}
                        >
                          {m.name || m.id}
                        </button>
                      ))}
                    </div>
                  </div>
                )}
                <button
                  type="button"
                  onClick={() => setAdvanced((v) => !v)}
                  className="text-[12px] text-ink-500 hover:text-ink-300"
                >
                  {advanced ? "Hide" : "Show"} advanced role models
                </button>
                {advanced && (
                  <div className="grid grid-cols-1 gap-2 sm:grid-cols-3">
                    {(
                      [
                        ["Planner", plannerModel, setPlannerModel],
                        ["Worker", workerModel, setWorkerModel],
                        ["Reviewer", reviewerModel, setReviewerModel],
                      ] as const
                    ).map(([label, value, set]) => (
                      <label key={label} className="block text-[12px] text-ink-400">
                        {label}
                        <select
                          value={value}
                          onChange={(e) => set(e.target.value)}
                          className="mt-1 w-full rounded-sm border border-ink-700 bg-ink-950 px-2 py-1.5 font-mono text-[12px] text-ink-100 focus:border-accent/60 focus:outline-none"
                        >
                          <option value="">(default)</option>
                          {modelOpts.map((m) => (
                            <option key={m.id} value={m.id}>
                              {m.name || m.id}
                            </option>
                          ))}
                        </select>
                      </label>
                    ))}
                  </div>
                )}
                <p className="text-[11px] text-ink-600">
                  Autonomous mode: auto-deploy is always on. No approval prompts — Abort is the only
                  stop control. Models/providers are your selection (never hardcoded).
                </p>
                <button
                  type="button"
                  disabled={!goal.trim()}
                  onClick={submit}
                  className="inline-flex w-full items-center justify-center gap-2 rounded-sm bg-accent px-4 py-2 text-[12px] font-medium text-white hover:bg-accent-soft disabled:opacity-40"
                >
                  <BoltIcon width={14} height={14} />
                  Launch mission
                </button>
              </section>
            )}

            {/* Active / history mission dashboard */}
            {goalMode && (
              <section className="space-y-3">
                <div className="rounded-sm border border-ink-800 bg-ink-900 px-3 py-2.5">
                  <div className="text-[10px] font-mono uppercase tracking-wider text-ink-500">
                    Mission
                  </div>
                  <p className="mt-1 text-[13px] leading-relaxed text-ink-100">{goalMode.goal}</p>
                  {goalMode.error && (
                    <p className="mt-2 text-[12px] text-danger">{goalMode.error}</p>
                  )}
                </div>

                {goalPlan && (
                  <div className="rounded-sm border border-ink-800 bg-ink-900 px-3 py-2.5">
                    <div className="mb-1 text-[10px] font-mono uppercase tracking-wider text-ink-500">
                      Orchestrator plan
                    </div>
                    <p className="text-[13px] text-ink-200">{goalPlan.summary}</p>
                    <ol className="mt-2 space-y-1">
                      {goalPlan.steps.map((s, i) => (
                        <li key={s.id} className="flex gap-2 text-[12px] text-ink-300">
                          <span className="font-mono text-ink-600">{i + 1}.</span>
                          <span>
                            <span className="font-mono text-accent-soft">{s.agent}</span>
                            {" — "}
                            {s.title || s.task}
                          </span>
                        </li>
                      ))}
                    </ol>
                    {goalPlan.validation?.length > 0 && (
                      <div className="mt-2 text-[11px] text-ink-500">
                        Validation: {goalPlan.validation.join("; ")}
                      </div>
                    )}
                  </div>
                )}

                <VerdictCard title="Pre-deploy self-review" verdict={goalMode.review_verdict} />

                <div className="space-y-2">
                  <div className="text-[10px] font-mono uppercase tracking-wider text-ink-500">
                    Iterations
                  </div>
                  {iterations.length === 0 ? (
                    <p className="text-[12px] text-ink-600">No iterations yet.</p>
                  ) : (
                    iterations.map((rec) => (
                      <IterationBlock
                        key={rec.iteration}
                        record={rec}
                        isCurrent={rec.iteration === (goalMode.iteration ?? 0)}
                        runs={subagentRuns}
                        onOpenRun={setSelectedRunId}
                      />
                    ))
                  )}
                </div>

                {liveRuns.length > 0 && (
                  <div className="space-y-2">
                    <div className="text-[10px] font-mono uppercase tracking-wider text-ink-500">
                      Live agent runs
                    </div>
                    {liveRuns.map((r) => (
                      <RunCard key={r.id} run={r} onClick={() => setSelectedRunId(r.id)} />
                    ))}
                  </div>
                )}

                <VerdictCard
                  title="Verify verdict"
                  verdict={goalMode.verify_verdict}
                  gaps={goalMode.remaining_gaps}
                />

                {certified && (
                  <div className="rounded-sm border border-ink-700 border-l-2 border-l-success bg-ink-925 px-3 py-2.5 text-[13px] text-success">
                    Mission certified complete.
                  </div>
                )}

                {active && (
                  <button
                    type="button"
                    onClick={onAbort}
                    className="inline-flex w-full items-center justify-center gap-2 rounded-sm bg-danger/90 px-4 py-2 text-[12px] font-medium text-white hover:bg-danger"
                  >
                    <StopIcon width={14} height={14} />
                    Abort mission
                  </button>
                )}
              </section>
            )}

            {!goalMode && !ceoActive && (
              <p className="text-center text-[12px] text-ink-600">
                Launch a mission to watch the CEO plan → review → deploy → verify loop.
              </p>
            )}

            {/* phase legend (compact) */}
            <div className="flex flex-wrap gap-1 border-t border-ink-800 pt-3">
              {CEO_PHASES.filter((p) => !["plan_ready", "synthesizing"].includes(p)).map((p) => (
                <span
                  key={p}
                  className={`rounded-sm px-1.5 py-0.5 text-[10px] ${
                    phase === p ? phasePillClass(p, certified) : "bg-ink-900 text-ink-600"
                  }`}
                >
                  {phaseLabel(p)}
                </span>
              ))}
            </div>
          </div>
        )}
      </div>
    </div>
  );
}
