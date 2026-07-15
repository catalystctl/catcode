"use client";

// GoalModal — multi-field form for /goal: plan then deploy subagents under
// concurrency + model/provider allowlists. Advanced: role models + per-model caps.

import { useMemo, useState } from "react";
import type { ModelInfo, ProviderPreset } from "@/lib/types";
import { useOutsideClose, mergeRefs } from "@/lib/use-outside-close";
import { useFocusTrap } from "@/lib/use-focus-trap";
import { XIcon } from "./icons";

export interface GoalStartOpts {
  goal: string;
  concurrency?: number;
  max_tasks?: number;
  allowed_models?: string[];
  allowed_providers?: string[];
  auto_deploy?: boolean;
  planner_model?: string;
  worker_model?: string;
  reviewer_model?: string;
  model_concurrency?: Record<string, number>;
}

interface Props {
  models: ModelInfo[];
  providerPresets: ProviderPreset[];
  providers: string[];
  onStart: (opts: GoalStartOpts) => void;
  onClose: () => void;
}

export function GoalModal({ models, providerPresets, providers, onStart, onClose }: Props) {
  const [goal, setGoal] = useState("");
  const [concurrency, setConcurrency] = useState(4);
  const [maxTasks, setMaxTasks] = useState(8);
  const [selectedProviders, setSelectedProviders] = useState<Set<string>>(new Set());
  const [selectedModels, setSelectedModels] = useState<Set<string>>(new Set());
  const [reviewBeforeDeploy, setReviewBeforeDeploy] = useState(false);
  const [advanced, setAdvanced] = useState(false);
  const [plannerModel, setPlannerModel] = useState("");
  const [workerModel, setWorkerModel] = useState("");
  const [reviewerModel, setReviewerModel] = useState("");
  const [modelConcurrency, setModelConcurrency] = useState<Record<string, number>>({});
  const closeRef = useOutsideClose(onClose);
  const trapRef = useFocusTrap<HTMLDivElement>();
  const executionProfile = concurrency >= 8 ? "Ultra parallel" : concurrency > 1 ? "Parallel" : "Serial";

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

  const concModelOpts = useMemo(() => {
    let base = modelOpts.map((m) => m.id);
    if (selectedModels.size > 0) {
      base = base.filter((id) => selectedModels.has(id));
    }
    for (const role of [plannerModel, workerModel, reviewerModel]) {
      if (role && !base.includes(role)) base = [...base, role];
    }
    return base;
  }, [modelOpts, selectedModels, plannerModel, workerModel, reviewerModel]);

  const toggle = (set: Set<string>, id: string, setter: (s: Set<string>) => void) => {
    const next = new Set(set);
    if (next.has(id)) next.delete(id);
    else next.add(id);
    setter(next);
  };

  const submit = () => {
    const g = goal.trim();
    if (!g) return;
    const c = Math.max(1, Math.min(32, concurrency));
    const mt = Math.max(1, Math.min(64, maxTasks));
    const opts: GoalStartOpts = {
      goal: g,
      concurrency: Math.min(c, mt),
      max_tasks: mt,
      allowed_models: selectedModels.size ? [...selectedModels] : undefined,
      allowed_providers: selectedProviders.size ? [...selectedProviders] : undefined,
      auto_deploy: !reviewBeforeDeploy,
    };
    if (advanced) {
      if (plannerModel) opts.planner_model = plannerModel;
      if (workerModel) opts.worker_model = workerModel;
      if (reviewerModel) opts.reviewer_model = reviewerModel;
      const mc: Record<string, number> = {};
      for (const [k, v] of Object.entries(modelConcurrency)) {
        if (v >= 1 && v <= Math.min(c, mt)) mc[k] = v;
      }
      if (Object.keys(mc).length) opts.model_concurrency = mc;
    }
    onStart(opts);
    onClose();
  };

  const roleSelect = (
    label: string,
    value: string,
    onChange: (v: string) => void,
  ) => (
    <label className="block">
      <span className="mb-1 block text-[11px] font-medium uppercase tracking-wide text-ink-500">
        {label}
      </span>
      <select
        value={value}
        onChange={(e) => onChange(e.target.value)}
        className="w-full rounded-lg border border-ink-700 bg-ink-950 px-2 py-1.5 text-[12px] text-ink-200 focus:border-accent/50 focus:outline-none"
      >
        <option value="">(default)</option>
        {modelOpts.map((m) => (
          <option key={m.id} value={m.id}>
            {m.id}
            {m.provider ? ` [${m.provider}]` : ""}
          </option>
        ))}
      </select>
    </label>
  );

  return (
    <div className="modal-backdrop">
      <div
        ref={mergeRefs(closeRef, trapRef)}
        className="modal-sheet max-w-lg"
        role="dialog"
        aria-modal="true"
        aria-label="Goal mode"
      >
        <div className="flex items-center justify-between border-b border-ink-800/80 px-4 py-3">
          <div>
            <span className="text-[13px] font-semibold text-ink-100">Goal mode</span>
            <p className="text-[11px] text-ink-500">Plan, then deploy subagents</p>
          </div>
          <button
            onClick={onClose}
            className="rounded-md p-1 text-ink-500 hover:bg-ink-800 hover:text-ink-100"
            aria-label="Close"
          >
            <XIcon width={16} height={16} />
          </button>
        </div>

        <div className="flex-1 space-y-3 overflow-y-auto p-4">
          <label className="block">
            <span className="mb-1 block text-[11px] font-medium uppercase tracking-wide text-ink-500">
              Goal
            </span>
            <textarea
              rows={3}
              value={goal}
              onChange={(e) => setGoal(e.target.value)}
              placeholder="Describe what you want the harness to plan and deploy…"
              className="w-full resize-none rounded-lg border border-ink-700 bg-ink-950 px-3 py-2 text-[13px] text-ink-100 placeholder:text-ink-600 focus:border-accent/50 focus:outline-none"
              autoFocus
            />
          </label>

          <div className="grid grid-cols-1 gap-3 sm:grid-cols-2">
            <label className="block">
              <span className="mb-1 block text-[11px] font-medium uppercase tracking-wide text-ink-500">
                Concurrency
              </span>
              <input
                type="number"
                min={1}
                max={32}
                value={concurrency}
                onChange={(e) => {
                  const next = Math.max(1, Math.min(32, Number(e.target.value) || 1));
                  setConcurrency(next);
                  setMaxTasks((current) => Math.max(current, next));
                }}
                className="w-full rounded-lg border border-ink-700 bg-ink-950 px-3 py-1.5 font-mono text-[12px] text-ink-200 focus:border-accent/50 focus:outline-none"
              />
            </label>
            <label className="block">
              <span className="mb-1 block text-[11px] font-medium uppercase tracking-wide text-ink-500">
                Max tasks
              </span>
              <input
                type="number"
                min={1}
                max={64}
                value={maxTasks}
                onChange={(e) => setMaxTasks(Number(e.target.value) || 1)}
                className="w-full rounded-lg border border-ink-700 bg-ink-950 px-3 py-1.5 font-mono text-[12px] text-ink-200 focus:border-accent/50 focus:outline-none"
              />
            </label>
          </div>
          <p className="-mt-1 text-[11px] text-ink-500">
            Execution profile: <span className="font-medium text-ink-300">{executionProfile}</span>
            {concurrency >= 8
              ? " — plans are shaped to fill a wide concurrency window."
              : " — independent plan steps run concurrently."}
          </p>

          <div>
            <span className="mb-1.5 block text-[11px] font-medium uppercase tracking-wide text-ink-500">
              Providers <span className="normal-case text-ink-600">(empty = all)</span>
            </span>
            <div className="flex flex-wrap gap-1.5">
              {providerOpts.length === 0 && (
                <span className="text-[12px] text-ink-600">No providers logged in</span>
              )}
              {providerOpts.map((p) => {
                const on = selectedProviders.has(p);
                return (
                  <button
                    key={p}
                    type="button"
                    onClick={() => toggle(selectedProviders, p, setSelectedProviders)}
                    className={`rounded-full border px-2.5 py-0.5 text-[11px] font-medium transition-colors ${
                      on
                        ? "border-accent/50 bg-accent/15 text-accent-soft"
                        : "border-ink-700 bg-ink-950 text-ink-400 hover:border-ink-600"
                    }`}
                  >
                    {p}
                  </button>
                );
              })}
            </div>
          </div>

          <div>
            <span className="mb-1.5 block text-[11px] font-medium uppercase tracking-wide text-ink-500">
              Models <span className="normal-case text-ink-600">(empty = all)</span>
            </span>
            <div className="flex max-h-28 flex-wrap gap-1.5 overflow-y-auto">
              {modelOpts.map((m) => {
                const on = selectedModels.has(m.id);
                return (
                  <button
                    key={m.id}
                    type="button"
                    onClick={() => toggle(selectedModels, m.id, setSelectedModels)}
                    className={`rounded-full border px-2.5 py-0.5 text-[11px] font-medium transition-colors ${
                      on
                        ? "border-accent/50 bg-accent/15 text-accent-soft"
                        : "border-ink-700 bg-ink-950 text-ink-400 hover:border-ink-600"
                    }`}
                    title={m.provider ? `${m.id} [${m.provider}]` : m.id}
                  >
                    {m.id}
                  </button>
                );
              })}
            </div>
          </div>

          <label className="flex cursor-pointer items-center gap-2 rounded-lg border border-ink-800 bg-ink-925/40 px-3 py-2">
            <input
              type="checkbox"
              checked={reviewBeforeDeploy}
              onChange={(e) => setReviewBeforeDeploy(e.target.checked)}
              className="rounded border-ink-600"
            />
            <span className="text-[12px] text-ink-200">
              Review plan before deploy
              <span className="block text-[11px] text-ink-500">
                Off = plan then auto-deploy
              </span>
            </span>
          </label>

          <label className="flex cursor-pointer items-center gap-2 rounded-lg border border-ink-800 bg-ink-925/40 px-3 py-2">
            <input
              type="checkbox"
              checked={advanced}
              onChange={(e) => setAdvanced(e.target.checked)}
              className="rounded border-ink-600"
            />
            <span className="text-[12px] text-ink-200">
              Advanced
              <span className="block text-[11px] text-ink-500">
                Pin planner / worker / reviewer models and per-model concurrency
              </span>
            </span>
          </label>

          {advanced && (
            <div className="space-y-3 rounded-xl border border-ink-800 bg-ink-950/50 p-3">
              <div className="grid grid-cols-1 gap-2 sm:grid-cols-3">
                {roleSelect("Planner", plannerModel, setPlannerModel)}
                {roleSelect("Worker", workerModel, setWorkerModel)}
                {roleSelect("Reviewer", reviewerModel, setReviewerModel)}
              </div>
              <div>
                <span className="mb-1.5 block text-[11px] font-medium uppercase tracking-wide text-ink-500">
                  Per-model concurrency{" "}
                  <span className="normal-case text-ink-600">
                    (max {Math.min(concurrency, maxTasks)}; empty = global)
                  </span>
                </span>
                <div className="max-h-36 space-y-1.5 overflow-y-auto">
                  {concModelOpts.length === 0 && (
                    <span className="text-[12px] text-ink-600">No models available</span>
                  )}
                  {concModelOpts.map((id) => (
                    <div key={id} className="flex items-center gap-2">
                      <span className="min-w-0 flex-1 truncate font-mono text-[11px] text-ink-300">
                        {id}
                      </span>
                      <input
                        type="number"
                        min={1}
                        max={Math.min(concurrency, maxTasks)}
                        placeholder={String(Math.min(concurrency, maxTasks))}
                        value={modelConcurrency[id] ?? ""}
                        onChange={(e) => {
                          const n = Number(e.target.value);
                          setModelConcurrency((prev) => {
                            const next = { ...prev };
                            if (!e.target.value || Number.isNaN(n) || n <= 0) {
                              delete next[id];
                            } else {
                              next[id] = Math.min(n, Math.min(concurrency, maxTasks));
                            }
                            return next;
                          });
                        }}
                        className="w-16 rounded-lg border border-ink-700 bg-ink-950 px-2 py-1 font-mono text-[12px] text-ink-200 focus:border-accent/50 focus:outline-none"
                      />
                    </div>
                  ))}
                </div>
              </div>
            </div>
          )}
        </div>

        <div className="flex items-center justify-end gap-2 border-t border-ink-800/80 px-4 py-3">
          <button
            type="button"
            onClick={onClose}
            className="rounded-lg px-3 py-1.5 text-[12px] text-ink-400 hover:bg-ink-800 hover:text-ink-100"
          >
            Cancel
          </button>
          <button
            type="button"
            onClick={submit}
            disabled={!goal.trim()}
            className="rounded-lg bg-accent px-4 py-1.5 text-[12px] font-semibold text-white transition-colors hover:bg-accent-soft disabled:cursor-not-allowed disabled:bg-ink-800 disabled:text-ink-500"
          >
            Start goal
          </button>
        </div>
      </div>
    </div>
  );
}

/** Banner shown when phase is plan_ready and auto_deploy is false. */
export function GoalPlanBanner({
  goal,
  summary,
  steps,
  onApprove,
  onRevise,
  onCancel,
}: {
  goal: string;
  summary?: string;
  steps: Array<{ agent: string; title: string }>;
  onApprove: () => void;
  onRevise: () => void;
  onCancel: () => void;
}) {
  return (
    <div className="mx-auto mb-3 w-full max-w-3xl rounded-xl border border-accent/30 bg-accent/10 px-4 py-3">
      <div className="mb-1 text-[12px] font-semibold text-accent-soft">Goal plan ready</div>
      <div className="mb-1 text-[13px] text-ink-100">{goal}</div>
      {summary && <p className="mb-2 text-[12px] text-ink-400">{summary}</p>}
      {steps.length > 0 && (
        <ul className="mb-3 space-y-0.5 text-[12px] text-ink-300">
          {steps.map((s, i) => (
            <li key={i}>
              {i + 1}. <span className="text-ink-500">[{s.agent}]</span> {s.title}
            </li>
          ))}
        </ul>
      )}
      <div className="flex flex-wrap gap-2">
        <button
          type="button"
          onClick={onApprove}
          className="rounded-lg bg-accent px-3 py-1 text-[12px] font-semibold text-white hover:bg-accent-soft"
        >
          Approve & deploy
        </button>
        <button
          type="button"
          onClick={onRevise}
          className="rounded-lg border border-ink-700 bg-ink-950 px-3 py-1 text-[12px] text-ink-200 hover:border-ink-600"
        >
          Revise
        </button>
        <button
          type="button"
          onClick={onCancel}
          className="rounded-lg px-3 py-1 text-[12px] text-ink-500 hover:text-ink-200"
        >
          Cancel goal
        </button>
      </div>
    </div>
  );
}

/** Compact status chip for an active goal. */
export function GoalStatusChip({
  phase,
  goal,
  onCancel,
}: {
  phase: string;
  goal: string;
  onCancel?: () => void;
}) {
  if (!phase || phase === "idle" || phase === "done") return null;
  const short = goal.length > 48 ? goal.slice(0, 47) + "…" : goal;
  return (
    <div className="flex items-center gap-2 rounded-lg border border-ink-800 bg-ink-925/60 px-2.5 py-1 text-[11px]">
      <span className="font-mono uppercase tracking-wide text-accent-soft">{phase}</span>
      <span className="truncate text-ink-300">{short}</span>
      {onCancel && phase !== "failed" && (
        <button
          type="button"
          onClick={onCancel}
          className="ml-auto text-ink-500 hover:text-rose-400"
        >
          cancel
        </button>
      )}
    </div>
  );
}
