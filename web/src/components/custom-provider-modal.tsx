"use client";

// CustomProviderModal — add (or update) a custom provider with full
// config.json parity: name, wire kind, base URL, API key or env var, extra
// headers, and a context-window override. PLUS a discover step: once the
// endpoint is entered, the harness fetches the models it exposes and the user
// can refine per-model caps (reasoning levels, context length, output tokens)
// — anything left at the discovered/default value (200k/8k flat default for
// unknown ids) is not written, so the config stays clean.

import { useEffect, useMemo, useState } from "react";
import type { CustomProviderDraft, ModelInfo, ModelOverride } from "@/lib/types";
import { useOutsideClose, mergeRefs } from "@/lib/use-outside-close";
import { useFocusTrap } from "@/lib/use-focus-trap";
import { useBodyScrollLock } from "@/lib/use-body-scroll-lock";
import { XIcon, ShieldIcon, ArrowLeftIcon } from "./icons";

interface Props {
  /** Models discovered by `discover_provider_models` (null until a preview lands). */
  previewModels: ModelInfo[] | null;
  /** True while a discovery request is in flight. */
  discovering: boolean;
  onDiscover: (base_url: string, kind: CustomProviderDraft["kind"], api_key?: string) => void;
  onSubmit: (draft: CustomProviderDraft) => void;
  onClose: () => void;
}

const KINDS: { id: CustomProviderDraft["kind"]; label: string; hint: string }[] = [
  { id: "openai", label: "OpenAI-compatible", hint: "/chat/completions · Authorization: Bearer" },
  { id: "anthropic", label: "Anthropic", hint: "/v1/messages · x-api-key" },
];

/** Editable per-model caps (prefilled from discovery; user refines). */
interface ModelCaps {
  context_window: string;
  max_tokens: string;
  reasoning: boolean;
  thinking_levels: string;
}

export function CustomProviderModal({
  previewModels,
  discovering,
  onDiscover,
  onSubmit,
  onClose,
}: Props) {
  const closeRef = useOutsideClose(onClose);
  const trapRef = useFocusTrap<HTMLDivElement>();
  useBodyScrollLock();
  const [step, setStep] = useState<"endpoint" | "models">("endpoint");
  const [draft, setDraft] = useState<CustomProviderDraft>({
    name: "",
    kind: "openai",
    base_url: "",
    apiKey: "",
    apiKeyEnv: "",
    headersText: "",
    contextWindow: "",
    modelsOverride: [],
  });
  const [touched, setTouched] = useState(false);
  // Per-model editable caps, keyed by model id. Initialized from the preview.
  const [caps, setCaps] = useState<Record<string, ModelCaps>>({});

  // When a preview lands, seed the editable caps from the discovered values
  // and advance to the models step.
  useEffect(() => {
    if (previewModels && previewModels.length > 0) {
      const next: Record<string, ModelCaps> = {};
      for (const m of previewModels) {
        next[m.id] = {
          context_window: String(m.context_window),
          max_tokens: String(m.max_tokens),
          reasoning: m.reasoning,
          thinking_levels: m.thinking_levels.join(", "),
        };
      }
      setCaps(next);
      setStep("models");
    }
  }, [previewModels]);

  const set = (patch: Partial<CustomProviderDraft>) =>
    setDraft((d) => ({ ...d, ...patch }));

  const nameOk = draft.name.trim().length > 0;
  const urlOk = /^https?:\/\/.+/.test(draft.base_url.trim());
  const ctxOk = draft.contextWindow.trim() === "" || /^[0-9]+$/.test(draft.contextWindow.trim());
  const headersOk = draft.headersText
    .split(/\n+/)
    .filter((l) => l.trim() !== "")
    .every((l) => l.indexOf(":") > 0);
  const endpointValid = nameOk && urlOk && ctxOk && headersOk;

  const fieldErr = (ok: boolean, msg: string) =>
    touched && !ok ? <p className="mt-1 text-[11px] text-danger">{msg}</p> : null;

  const inputCls =
    "w-full rounded-lg border border-ink-700 bg-ink-950 px-3 py-2 text-[13px] text-ink-100 placeholder:text-ink-600 focus:border-accent/50 focus:outline-none";

  const discover = () => {
    setTouched(true);
    if (!endpointValid) return;
    onDiscover(draft.base_url.trim(), draft.kind, draft.apiKey.trim() || undefined);
  };

  // Build the models_override payload: only include a model when the user
  // changed a field from its discovered baseline. Unchanged models fall through
  // to the discovered/curated/flat-default caps (200k/8k for unknown ids).
  const buildOverrides = (): ModelOverride[] => {
    if (!previewModels) return [];
    const out: ModelOverride[] = [];
    for (const m of previewModels) {
      const c = caps[m.id];
      if (!c) continue;
      const ctx = parseInt(c.context_window.trim(), 10);
      const max = parseInt(c.max_tokens.trim(), 10);
      const levels = c.thinking_levels
        .split(/[,\s]+/)
        .map((s) => s.trim())
        .filter(Boolean);
      const ctxChanged = Number.isFinite(ctx) && ctx !== m.context_window;
      const maxChanged = Number.isFinite(max) && max !== m.max_tokens;
      const reasonChanged = c.reasoning !== m.reasoning;
      const levelsChanged =
        levels.join(",") !== m.thinking_levels.join(",");
      if (!ctxChanged && !maxChanged && !reasonChanged && !levelsChanged) continue;
      out.push({
        id: m.id,
        context_window: ctxChanged && Number.isFinite(ctx) ? ctx : undefined,
        max_tokens: maxChanged && Number.isFinite(max) ? max : undefined,
        reasoning: reasonChanged ? c.reasoning : undefined,
        thinking_levels: levelsChanged ? levels : undefined,
      });
    }
    return out;
  };

  const submit = () => {
    setTouched(true);
    if (!endpointValid) {
      setStep("endpoint");
      return;
    }
    onSubmit({ ...draft, modelsOverride: buildOverrides() });
    onClose();
  };

  return (
    <div className="modal-backdrop">
      <div
        ref={mergeRefs(closeRef, trapRef)}
        className="modal-sheet max-w-lg"
        role="dialog"
        aria-modal="true"
        aria-label="Add custom provider"
      >
        <div className="flex items-center justify-between border-b border-ink-800/80 px-5 py-3.5">
          <div className="flex items-center gap-2">
            {step === "models" && (
              <button
                onClick={() => setStep("endpoint")}
                className="rounded-md p-1 text-ink-500 hover:bg-ink-800 hover:text-ink-100"
                aria-label="Back to endpoint"
              >
                <ArrowLeftIcon width={15} height={15} />
              </button>
            )}
            <ShieldIcon width={16} height={16} className="text-accent-soft" />
            <h2 className="text-[15px] font-semibold text-ink-100">
              {step === "models" ? "Refine model caps" : "Add custom provider"}
            </h2>
            <span className="ml-1 rounded-full bg-ink-800 px-2 py-0.5 text-[10px] font-medium uppercase tracking-wider text-ink-400">
              {step === "models" ? "2 / 2" : "1 / 2"}
            </span>
          </div>
          <button
            onClick={onClose}
            className="rounded-md p-1.5 text-ink-500 hover:bg-ink-800 hover:text-ink-100"
            aria-label="Close"
          >
            <XIcon width={16} height={16} />
          </button>
        </div>

        {step === "endpoint" && (
          <div className="min-h-0 flex-1 space-y-4 overflow-y-auto px-5 py-4">
            <div className="grid grid-cols-2 gap-3">
              <div>
                <label className="mb-1.5 block text-[11px] font-semibold uppercase tracking-wider text-ink-500">
                  Name <span className="text-danger">*</span>
                </label>
                <input
                  autoFocus
                  value={draft.name}
                  onChange={(e) => set({ name: e.target.value })}
                  placeholder="my-provider"
                  className={inputCls + " font-mono"}
                />
                {fieldErr(nameOk, "A unique slug (e.g. my-provider).")}
              </div>
              <div>
                <label className="mb-1.5 block text-[11px] font-semibold uppercase tracking-wider text-ink-500">
                  Wire protocol
                </label>
                <div className="flex overflow-hidden rounded-lg border border-ink-700">
                  {KINDS.map((k) => (
                    <button
                      key={k.id}
                      onClick={() => set({ kind: k.id })}
                      title={k.hint}
                      className={`flex-1 px-2 py-2 text-[12px] font-medium transition-colors ${
                        draft.kind === k.id
                          ? "bg-accent/15 text-accent-soft"
                          : "bg-ink-950 text-ink-400 hover:bg-ink-850"
                      }`}
                    >
                      {k.label}
                    </button>
                  ))}
                </div>
                <p className="mt-1 text-[11px] text-ink-600">
                  {KINDS.find((k) => k.id === draft.kind)?.hint}
                </p>
              </div>
            </div>

            <div>
              <label className="mb-1.5 block text-[11px] font-semibold uppercase tracking-wider text-ink-500">
                Base URL <span className="text-danger">*</span>
              </label>
              <input
                value={draft.base_url}
                onChange={(e) => set({ base_url: e.target.value })}
                placeholder="https://api.example.com/v1"
                className={inputCls + " font-mono"}
              />
              <p className="mt-1 text-[11px] text-ink-600">
                Include the version segment — paths are appended directly (e.g. /chat/completions).
              </p>
              {fieldErr(urlOk, "Must be an http(s) URL, including the version segment (e.g. /v1).")}
            </div>

            <div className="grid grid-cols-2 gap-3">
              <div>
                <label className="mb-1.5 block text-[11px] font-semibold uppercase tracking-wider text-ink-500">
                  API key
                </label>
                <input
                  type="password"
                  autoComplete="off"
                  value={draft.apiKey}
                  onChange={(e) => set({ apiKey: e.target.value })}
                  placeholder="sk-…"
                  className={inputCls + " font-mono"}
                />
                <p className="mt-1 text-[11px] text-ink-600">
                  Stored in the 0600 user config. Wins over the env var.
                </p>
              </div>
              <div>
                <label className="mb-1.5 block text-[11px] font-semibold uppercase tracking-wider text-ink-500">
                  …or env var name
                </label>
                <input
                  value={draft.apiKeyEnv}
                  onChange={(e) => set({ apiKeyEnv: e.target.value })}
                  placeholder="MY_PROVIDER_API_KEY"
                  className={inputCls + " font-mono"}
                />
                <p className="mt-1 text-[11px] text-ink-600">
                  Secret stays in your environment; read at request time.
                </p>
              </div>
            </div>

            <div>
              <label className="mb-1.5 block text-[11px] font-semibold uppercase tracking-wider text-ink-500">
                Extra headers <span className="normal-case text-ink-600">(optional)</span>
              </label>
              <textarea
                rows={2}
                value={draft.headersText}
                onChange={(e) => set({ headersText: e.target.value })}
                placeholder={"HTTP-Referer: https://myapp.example\nX-Title: My App"}
                className={inputCls + " resize-y font-mono"}
              />
              <p className="mt-1 text-[11px] text-ink-600">One Key: value per line.</p>
              {fieldErr(headersOk, "Each line must be Key: value.")}
            </div>

            <div>
              <label className="mb-1.5 block text-[11px] font-semibold uppercase tracking-wider text-ink-500">
                Context window <span className="normal-case text-ink-600">(optional, tokens)</span>
              </label>
              <input
                inputMode="numeric"
                value={draft.contextWindow}
                onChange={(e) => set({ contextWindow: e.target.value })}
                placeholder="e.g. 128000"
                className={inputCls + " font-mono"}
              />
              <p className="mt-1 text-[11px] text-ink-600">
                Force every discovered model to this window — useful for local servers (LM Studio)
                that return bare model ids.
              </p>
              {fieldErr(ctxOk, "Digits only, or leave blank.")}
            </div>
          </div>
        )}

        {step === "models" && (
          <div className="min-h-0 flex-1 space-y-3 overflow-y-auto px-5 py-4">
            <p className="text-[12px] text-ink-400">
              Discovered <span className="font-semibold text-ink-100">{previewModels?.length ?? 0}</span>{" "}
              models from <span className="font-mono text-ink-300">{draft.base_url}</span>. Refine any
              caps below — fields left at the discovered value aren&apos;t written, so the harness
              keeps its defaults (200k context / 8k output for unknown ids).
            </p>
            {(previewModels ?? []).map((m) => {
              const c = caps[m.id];
              if (!c) return null;
              return (
                <div
                  key={m.id}
                  className="rounded-xl border border-ink-700/70 bg-ink-900/70 px-3.5 py-3"
                >
                  <div className="flex items-center justify-between gap-2">
                    <div className="min-w-0">
                      <div className="truncate font-mono text-[13px] text-ink-100">{m.id}</div>
                      <div className="truncate text-[11px] text-ink-500">{m.name}</div>
                    </div>
                    <label className="flex shrink-0 cursor-pointer items-center gap-1.5 text-[11px] text-ink-400">
                      <input
                        type="checkbox"
                        checked={c.reasoning}
                        onChange={(e) =>
                          setCaps((s) => ({ ...s, [m.id]: { ...c, reasoning: e.target.checked } }))
                        }
                        className="h-3.5 w-3.5 accent-accent"
                      />
                      reasoning
                    </label>
                  </div>
                  <div className="mt-2.5 grid grid-cols-3 gap-2">
                    <div>
                      <label className="mb-1 block text-[10px] font-semibold uppercase tracking-wider text-ink-600">
                        Context
                      </label>
                      <input
                        inputMode="numeric"
                        value={c.context_window}
                        onChange={(e) =>
                          setCaps((s) => ({ ...s, [m.id]: { ...c, context_window: e.target.value } }))
                        }
                        className={inputCls + " px-2 py-1.5 font-mono text-[12px]"}
                      />
                    </div>
                    <div>
                      <label className="mb-1 block text-[10px] font-semibold uppercase tracking-wider text-ink-600">
                        Output
                      </label>
                      <input
                        inputMode="numeric"
                        value={c.max_tokens}
                        onChange={(e) =>
                          setCaps((s) => ({ ...s, [m.id]: { ...c, max_tokens: e.target.value } }))
                        }
                        className={inputCls + " px-2 py-1.5 font-mono text-[12px]"}
                      />
                    </div>
                    <div>
                      <label className="mb-1 block text-[10px] font-semibold uppercase tracking-wider text-ink-600">
                        Effort levels
                      </label>
                      <input
                        value={c.thinking_levels}
                        onChange={(e) =>
                          setCaps((s) => ({ ...s, [m.id]: { ...c, thinking_levels: e.target.value } }))
                        }
                        placeholder="low, medium, high"
                        className={inputCls + " px-2 py-1.5 font-mono text-[12px]"}
                      />
                    </div>
                  </div>
                </div>
              );
            })}
          </div>
        )}

        <div className="flex items-center justify-between gap-3 border-t border-ink-800/80 px-5 py-3.5">
          <p className="text-[11px] text-ink-600">Saved to ~/.config/catalyst-code/config.json</p>
          {step === "endpoint" ? (
            <div className="flex items-center gap-2">
              <button
                onClick={submit}
                className="rounded-lg border border-ink-700 bg-ink-850 px-3.5 py-2 text-[13px] font-medium text-ink-100 hover:bg-ink-800"
              >
                Add without discovering
              </button>
              <button
                onClick={discover}
                disabled={!endpointValid || discovering}
                className="rounded-lg bg-accent px-4 py-2 text-[13px] font-semibold text-white hover:bg-accent-soft disabled:opacity-40"
              >
                {discovering ? "Discovering…" : "Discover models →"}
              </button>
            </div>
          ) : (
            <button
              onClick={submit}
              className="rounded-lg bg-accent px-4 py-2 text-[13px] font-semibold text-white hover:bg-accent-soft"
            >
              Add provider
            </button>
          )}
        </div>
      </div>
    </div>
  );
}
