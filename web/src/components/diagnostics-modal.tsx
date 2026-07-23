"use client";

// DiagnosticsModal — dedicated panel for /stats, /context, and /usage payloads
// instead of toast-only truncation.

import type {
  CheckpointInfo,
  ContextBreakdown,
  CostUpdate,
  ProtocolHello,
  Stats,
  UsageSnapshot,
} from "@/lib/types";
import { formatTokens } from "@/lib/format";
import { useOutsideClose, mergeRefs } from "@/lib/use-outside-close";
import { useFocusTrap } from "@/lib/use-focus-trap";
import { useBodyScrollLock } from "@/lib/use-body-scroll-lock";
import { HistoryIcon, RefreshIcon, XIcon } from "./icons";

interface Props {
  stats: Stats | null;
  context: ContextBreakdown | null;
  usage: UsageSnapshot | null;
  cost?: CostUpdate | null;
  checkpoints?: CheckpointInfo[];
  protocolHello?: ProtocolHello | null;
  onRefresh: () => void;
  onCreateCheckpoint?: () => void;
  onRestoreCheckpoint?: (id: string) => void;
  onClose: () => void;
}

export function DiagnosticsModal({
  stats,
  context,
  usage,
  cost,
  checkpoints = [],
  protocolHello,
  onRefresh,
  onCreateCheckpoint,
  onRestoreCheckpoint,
  onClose,
}: Props) {
  const closeRef = useOutsideClose(onClose);
  const trapRef = useFocusTrap<HTMLDivElement>();
  useBodyScrollLock();

  return (
    <div className="modal-backdrop">
      <div
        ref={mergeRefs(closeRef, trapRef)}
        className="modal-sheet max-w-lg"
        role="dialog"
        aria-modal="true"
        aria-label="Diagnostics"
      >
        <div className="flex items-center justify-between border-b border-ink-800 px-5 py-3.5">
          <div className="flex items-center gap-2">
            <HistoryIcon width={16} height={16} className="text-accent-soft" />
            <h2 className="text-[15px] font-semibold text-ink-100">Diagnostics</h2>
          </div>
          <div className="flex items-center gap-1">
            <button
              onClick={onRefresh}
              className="flex h-6 w-6 items-center justify-center rounded-sm text-ink-400 hover:bg-ink-800 hover:text-ink-100"
              title="Refresh stats, context, usage, and checkpoints"
              aria-label="Refresh"
            >
              <RefreshIcon width={15} height={15} />
            </button>
            <button
              onClick={onClose}
              className="flex h-6 w-6 items-center justify-center rounded-sm text-ink-400 hover:bg-ink-800 hover:text-ink-100"
              aria-label="Close"
            >
              <XIcon width={16} height={16} />
            </button>
          </div>
        </div>

        <div className="min-h-0 flex-1 space-y-4 overflow-y-auto px-5 py-4">
          <section>
            <SectionTitle>Session stats</SectionTitle>
            {stats ? (
              <div className="grid grid-cols-2 gap-2 rounded-sm border border-ink-800 bg-ink-900 p-3 font-mono text-[11px]">
                <Stat label="turns" value={String(stats.turns)} />
                <Stat label="messages" value={String(stats.messages)} />
                <Stat label="tokens in" value={formatTokens(stats.tokens_in)} />
                <Stat label="tokens out" value={formatTokens(stats.tokens_out)} />
                <Stat label="total" value={formatTokens(stats.tokens_total)} />
                <Stat label="cached" value={formatTokens(stats.cached_tokens)} />
              </div>
            ) : (
              <Empty>No stats yet — send a turn or hit refresh.</Empty>
            )}
          </section>

          <section>
            <SectionTitle>Cost</SectionTitle>
            {cost ? (
              <div className="grid grid-cols-2 gap-2 rounded-sm border border-ink-800 bg-ink-900 p-3 font-mono text-[11px]">
                {cost.estimated_usd != null && (
                  <Stat
                    label="est. USD"
                    value={`$${cost.estimated_usd.toFixed(cost.estimated_usd < 0.01 ? 4 : 3)}`}
                  />
                )}
                {cost.tokens_in != null && (
                  <Stat label="tokens in" value={formatTokens(cost.tokens_in)} />
                )}
                {cost.tokens_out != null && (
                  <Stat label="tokens out" value={formatTokens(cost.tokens_out)} />
                )}
                {cost.cached_tokens != null && (
                  <Stat label="cached" value={formatTokens(cost.cached_tokens)} />
                )}
                {cost.cache_hit_pct != null && (
                  <Stat label="cache hit" value={`${Math.round(cost.cache_hit_pct)}%`} />
                )}
                {cost.model && <Stat label="model" value={cost.model} />}
              </div>
            ) : (
              <Empty>No cost_update yet — complete a turn to see estimates.</Empty>
            )}
          </section>

          <section>
            <div className="mb-1.5 flex items-center justify-between gap-2">
              <SectionTitle>Checkpoints</SectionTitle>
              {onCreateCheckpoint && (
                <button
                  type="button"
                  onClick={onCreateCheckpoint}
                  className="rounded-sm border border-ink-700 px-2.5 py-1 text-[11px] text-ink-300 hover:bg-ink-800 hover:text-ink-100"
                >
                  Create
                </button>
              )}
            </div>
            {checkpoints.length > 0 ? (
              <div className="space-y-1.5 rounded-sm border border-ink-800 bg-ink-900 p-2">
                {checkpoints.slice(0, 12).map((c) => (
                  <div
                    key={String(c.id)}
                    className="flex items-center justify-between gap-2 rounded-sm border border-ink-800 bg-ink-950 px-2.5 py-1.5 text-[11px]"
                  >
                    <div className="min-w-0">
                      <div className="truncate font-medium text-ink-200">
                        {String(c.label || c.id)}
                      </div>
                      <div className="font-mono text-[10px] text-ink-500">
                        {String(c.kind ?? "checkpoint")}
                        {c.auto ? " · auto" : ""}
                      </div>
                    </div>
                    {onRestoreCheckpoint && (
                      <button
                        type="button"
                        onClick={() => onRestoreCheckpoint(String(c.id))}
                        className="shrink-0 rounded-sm border border-ink-700 px-2.5 py-1 text-[11px] text-ink-300 hover:bg-ink-800 hover:text-ink-100"
                      >
                        Restore
                      </button>
                    )}
                  </div>
                ))}
              </div>
            ) : (
              <Empty>No checkpoints yet.</Empty>
            )}
          </section>

          {protocolHello && (
            <section>
              <SectionTitle>Protocol</SectionTitle>
              <div className="rounded-sm border border-ink-800 bg-ink-900 p-3 font-mono text-[11px] text-ink-400">
                <div>v{protocolHello.version}</div>
                {protocolHello.capabilities.length > 0 && (
                  <div className="mt-1 text-[10px] text-ink-500">
                    {protocolHello.capabilities.join(", ")}
                  </div>
                )}
              </div>
            </section>
          )}

          <section>
            <SectionTitle>Context breakdown</SectionTitle>
            {context ? (
              <div className="space-y-2 rounded-sm border border-ink-800 bg-ink-900 p-3">
                <div className="flex items-baseline justify-between gap-2">
                  <span className="font-mono text-[12px] text-ink-200">
                    {context.total_tokens.toLocaleString()} /{" "}
                    {context.context_window.toLocaleString()} tokens
                  </span>
                  <span className="text-[11px] font-medium text-accent-soft">{context.pct}%</span>
                </div>
                <div className="h-1.5 overflow-hidden rounded-sm bg-ink-800">
                  <div
                    className="h-full bg-accent"
                    style={{ width: `${Math.min(100, Math.max(0, context.pct))}%` }}
                  />
                </div>
                <div className="grid grid-cols-2 gap-1.5 font-mono text-[10px] text-ink-400">
                  <Stat label="messages" value={String(context.messages)} />
                  <Stat label="system" value={formatTokens(context.system_tokens)} />
                  {context.digest_threshold_tokens != null && (
                    <Stat label="digest at" value={formatTokens(context.digest_threshold_tokens)} />
                  )}
                  {context.compact_threshold_tokens != null && (
                    <Stat label="compact at" value={formatTokens(context.compact_threshold_tokens)} />
                  )}
                  {context.hard_limit_tokens != null && (
                    <Stat label="hard limit" value={formatTokens(context.hard_limit_tokens)} />
                  )}
                  {context.response_reserve_tokens != null && (
                    <Stat
                      label="response reserve"
                      value={formatTokens(context.response_reserve_tokens)}
                    />
                  )}
                  {Object.entries(context.by_role ?? {}).map(([role, n]) => (
                    <Stat key={role} label={role} value={formatTokens(n)} />
                  ))}
                </div>
                {(context.top_consumers?.length ?? 0) > 0 && (
                  <div className="mt-2 space-y-1.5">
                    <div className="text-[10px] font-mono uppercase tracking-wider text-ink-500">
                      Top consumers
                    </div>
                    {context.top_consumers.slice(0, 8).map((c) => (
                      <div
                        key={`${c.role}-${c.index}`}
                        className="rounded-sm border border-ink-800 bg-ink-950 px-2.5 py-1.5"
                      >
                        <div className="flex items-center justify-between gap-2 text-[11px]">
                          <span className="font-medium text-ink-200">
                            {c.role} #{c.index}
                          </span>
                          <span className="font-mono text-ink-400">
                            {c.tokens.toLocaleString()}
                          </span>
                        </div>
                        {c.preview && (
                          <p className="mt-0.5 truncate font-mono text-[10px] text-ink-500">
                            {c.preview}
                          </p>
                        )}
                      </div>
                    ))}
                  </div>
                )}
              </div>
            ) : (
              <Empty>No context breakdown yet — hit refresh.</Empty>
            )}
          </section>

          <section>
            <SectionTitle>Provider usage</SectionTitle>
            {usage ? (
              <div className="space-y-2 rounded-sm border border-ink-800 bg-ink-900 p-3">
                <div className="flex flex-wrap items-center gap-2 text-[12px]">
                  <span className="font-medium text-ink-100">{usage.provider}</span>
                  {usage.plan && (
                    <span className="rounded-sm bg-ink-800 px-1.5 py-0.5 text-[10px] text-ink-400">
                      {usage.plan}
                    </span>
                  )}
                  {usage.model && (
                    <span className="font-mono text-[10px] text-ink-500">{usage.model}</span>
                  )}
                </div>
                {!usage.available ? (
                  <p className="text-[12px] text-ink-400">
                    {usage.message || "Usage not available for this provider."}
                  </p>
                ) : (usage.windows?.length ?? 0) === 0 ? (
                  <p className="text-[12px] text-ink-400">{usage.message || "No usage windows."}</p>
                ) : (
                  <div className="space-y-1.5">
                    {usage.windows.map((w) => (
                      <div
                        key={w.id || w.label}
                        className="flex items-center justify-between gap-2 rounded-sm border border-ink-800 bg-ink-950 px-2.5 py-1.5 text-[11px]"
                      >
                        <span className="text-ink-300">{w.label || w.id}</span>
                        <span className="font-mono text-ink-400">
                          {formatWindow(w)}
                        </span>
                      </div>
                    ))}
                  </div>
                )}
              </div>
            ) : (
              <Empty>No usage snapshot yet — hit refresh.</Empty>
            )}
          </section>
        </div>
      </div>
    </div>
  );
}

function SectionTitle({ children }: { children: React.ReactNode }) {
  return (
    <div className="mb-1.5 text-[10px] font-mono uppercase tracking-wider text-ink-500">
      {children}
    </div>
  );
}

function Empty({ children }: { children: React.ReactNode }) {
  return (
    <div className="rounded-sm border border-dashed border-ink-800 px-3 py-4 text-center text-[12px] text-ink-500">
      {children}
    </div>
  );
}

function Stat({ label, value }: { label: string; value: string }) {
  return (
    <div className="flex items-center justify-between gap-2">
      <span className="text-ink-600">{label}</span>
      <span className="text-ink-300">{value}</span>
    </div>
  );
}

function formatWindow(w: {
  used?: number;
  limit?: number;
  unit: string;
  detail?: string;
}): string {
  if (w.detail) return w.detail;
  if (w.unit === "percent" && typeof w.used === "number") return `${Math.round(w.used)}%`;
  if (typeof w.used === "number" && typeof w.limit === "number" && w.limit > 0) {
    const pct = Math.round((w.used / w.limit) * 100);
    return `${w.used}/${w.limit} (${pct}%)`;
  }
  if (typeof w.used === "number") return String(w.used);
  return w.unit || "—";
}
