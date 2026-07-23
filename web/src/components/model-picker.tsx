"use client";

// ModelPicker — a reusable model selector with provider filtering + name search.
//
// Used in two places:
//  (1) the Header's inline dropdown (compact, `variant="popover"`)
//  (2) the Settings modal's Model section (full-width, `variant="inline"`)
//
// Both share the same filtering logic: a live text search over name + id, plus
// single-select provider chips derived from the models' `provider` field. When
// the list is long the results scroll inside a bounded region.

import { useMemo, useState, type ReactNode } from "react";
import type { ModelInfo } from "@/lib/types";
import { formatTokens } from "@/lib/format";
import { CheckIcon, ModelIcon, SearchIcon, BrainIcon, XIcon } from "./icons";

interface Props {
  models: ModelInfo[];
  selectedModel: string | null;
  onSelect: (id: string) => void;
  /** "popover" = compact (header dropdown); "inline" = full-width (settings). */
  variant?: "popover" | "inline";
  /** Called after a selection is made (e.g. to close the popover). */
  onClose?: () => void;
}

const PROVIDER_LABELS: Record<string, string> = {
  openai: "OpenAI",
  anthropic: "Anthropic",
  gemini: "Gemini",
  google: "Google",
  xai: "xAI",
  umans: "Umans",
  deepseek: "DeepSeek",
  groq: "Groq",
  mistral: "Mistral",
  openrouter: "OpenRouter",
};

function prettyProvider(p: string): string {
  if (!p) return "default";
  return PROVIDER_LABELS[p] ?? p;
}

export function ModelPicker({
  models,
  selectedModel,
  onSelect,
  variant = "inline",
  onClose,
}: Props) {
  const [query, setQuery] = useState("");
  const [provider, setProvider] = useState<string | null>(null);

  const providers = useMemo(() => {
    const seen: string[] = [];
    for (const m of models) {
      const p = m.provider || "";
      if (p && !seen.includes(p)) seen.push(p);
    }
    return seen;
  }, [models]);

  const filtered = useMemo(() => {
    const q = query.trim().toLowerCase();
    return models.filter((m) => {
      if (provider !== null && (m.provider || "") !== provider) return false;
      if (!q) return true;
      const hay = `${m.name} ${m.id} ${m.provider}`.toLowerCase();
      return hay.includes(q);
    });
  }, [models, query, provider]);

  const isPopover = variant === "popover";

  return (
    <div className="flex flex-col">
      {/* Search + provider filters */}
      <div className={`flex flex-col gap-2 ${isPopover ? "p-2" : "p-2"}`}>
        <div className="relative">
          <SearchIcon
            width={13}
            height={13}
            className="pointer-events-none absolute left-2.5 top-1/2 -translate-y-1/2 text-ink-600"
          />
          <input
            autoFocus={true}
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            placeholder="Search models…"
            className="w-full rounded-sm border border-ink-700 bg-ink-950 py-1.5 pl-8 pr-7 text-[12px] text-ink-100 placeholder:text-ink-600 focus:border-accent/60 focus:outline-none"
          />
          {query && (
            <button
              onClick={() => setQuery("")}
              className="absolute right-2 top-1/2 -translate-y-1/2 text-ink-600 hover:text-ink-300"
              aria-label="Clear search"
            >
              <XIcon width={12} height={12} />
            </button>
          )}
        </div>
        {providers.length > 1 && (
          <div className="flex flex-wrap gap-1">
            <ProviderChip
              label="all"
              active={provider === null}
              onClick={() => setProvider(null)}
            />
            {providers.map((p) => (
              <ProviderChip
                key={p}
                label={prettyProvider(p)}
                active={provider === p}
                onClick={() => setProvider(provider === p ? null : p)}
              />
            ))}
          </div>
        )}
      </div>

      {/* Results */}
      <div
        className={`overflow-y-auto border-t border-ink-800/60 ${
          isPopover ? "max-h-64" : "max-h-[40vh]"
        }`}
      >
        {filtered.length === 0 ? (
          <div className="px-3 py-4 text-center text-[12px] text-ink-500">
            {models.length === 0
              ? "No models — set an API key."
              : "No models match your filters."}
          </div>
        ) : (
          filtered.map((mo, i) => {
            const active = selectedModel === mo.id;
            return (
              <button
                key={mo.id}
                onClick={() => {
                  onSelect(mo.id);
                  onClose?.();
                }}
                className={`flex w-full items-center gap-2.5 border-l-2 px-3 py-2 text-left transition-colors hover:bg-ink-850 ${
                  active ? "border-l-accent bg-ink-850" : "border-l-transparent"
                } ${i > 0 ? "border-t border-ink-800/50" : ""}`}
              >
                <ModelIcon
                  width={13}
                  height={13}
                  className={active ? "shrink-0 text-accent-soft" : "shrink-0 text-ink-500"}
                />
                <div className="min-w-0 flex-1">
                  <div className="flex items-center gap-1.5">
                    <span className="truncate text-[12px] font-medium text-ink-100">
                      {mo.name || mo.id}
                    </span>
                    {mo.reasoning && (
                      <BrainIcon width={11} height={11} className="shrink-0 text-accent-soft" />
                    )}
                  </div>
                  <div className="flex items-center gap-1.5 truncate font-mono text-[10px] text-ink-500">
                    {mo.provider && (
                      <span className="rounded-sm bg-ink-800 px-1 font-mono text-[10px] uppercase text-ink-400">
                        {prettyProvider(mo.provider)}
                      </span>
                    )}
                    <span className="truncate">{mo.id}</span>
                    {mo.context_window > 0 && (
                      <span className="shrink-0 text-ink-600">{formatTokens(mo.context_window)}</span>
                    )}
                  </div>
                </div>
                {active && (
                  <CheckIcon width={13} height={13} className="shrink-0 text-accent-soft" />
                )}
              </button>
            );
          })
        )}
      </div>
      {/* Result count footer */}
      {models.length > 0 && (
        <div className="border-t border-ink-800/60 px-3 py-1.5 text-[10px] text-ink-600">
          {filtered.length} of {models.length} models
          {provider && ` · ${prettyProvider(provider)}`}
        </div>
      )}
    </div>
  );
}

function ProviderChip({
  label,
  active,
  onClick,
}: {
  label: string;
  active: boolean;
  onClick: () => void;
}) {
  return (
    <button
      onClick={onClick}
      className={`rounded-sm border px-2 py-0.5 font-mono text-[10px] uppercase tracking-wider transition-colors ${
        active
          ? "border-accent/60 bg-ink-850 text-accent-soft"
          : "border-ink-700 bg-ink-900 text-ink-400 hover:border-ink-600 hover:text-ink-200"
      }`}
    >
      {label}
    </button>
  );
}

/** A standalone button + popover wrapper for the header's compact model menu.
 *  Renders the trigger (children) and positions the ModelPicker below it. */
export function ModelPopover({
  models,
  selectedModel,
  onSelect,
  trigger,
  closeRef,
}: {
  models: ModelInfo[];
  selectedModel: string | null;
  onSelect: (id: string) => void;
  trigger: ReactNode;
  closeRef: React.RefCallback<HTMLDivElement>;
}) {
  return (
    <>
      {trigger}
      <div
        ref={closeRef}
        role="menu"
        className="absolute right-0 z-30 mt-1 w-80 overflow-hidden rounded-sm border border-ink-700 bg-ink-900 shadow-elev-2 animate-fade-in"
      >
        <ModelPicker
          models={models}
          selectedModel={selectedModel}
          onSelect={onSelect}
          variant="popover"
        />
      </div>
    </>
  );
}
