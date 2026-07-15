"use client";

// About / version panel — shows the running git commit and whether this
// install is up to date, out of date, or has uncommitted changes.

import { useCallback, useEffect, useState } from "react";
import type { VersionInfo, VersionUpdateStatus } from "@/lib/version-types";
import { RefreshIcon } from "./icons";

type VersionResponse = ({ ok: true } & VersionInfo) | { ok: false; error?: string };

const STATUS_STYLES: Record<VersionUpdateStatus, string> = {
  up_to_date: "border-success/40 bg-success/10 text-success",
  out_of_date: "border-warning/40 bg-warning/10 text-warning",
  uncommitted: "border-accent/40 bg-accent/10 text-accent-soft",
  ahead: "border-ink-600 bg-ink-850 text-ink-200",
  unknown: "border-ink-700 bg-ink-900 text-ink-400",
};

function formatBuiltAt(iso: string | null): string | null {
  if (!iso) return null;
  const d = new Date(iso);
  if (Number.isNaN(d.getTime())) return iso;
  return d.toLocaleString(undefined, {
    year: "numeric",
    month: "short",
    day: "numeric",
    hour: "2-digit",
    minute: "2-digit",
  });
}

export function VersionInfoPanel() {
  const [info, setInfo] = useState<VersionInfo | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);

  const refresh = useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      const res = await fetch("/api/version", { cache: "no-store" });
      const data = (await res.json()) as VersionResponse;
      if (!res.ok || !data.ok) {
        setInfo(null);
        setError(!data.ok ? data.error || `Request failed (${res.status})` : `Request failed (${res.status})`);
        return;
      }
      setInfo(data);
    } catch (err) {
      setInfo(null);
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  return (
    <div>
      <div className="flex items-start justify-between gap-2">
        <div>
          <div className="text-[11px] font-medium uppercase tracking-wider text-ink-500">
            About
          </div>
          <div className="mt-0.5 text-[13px] font-medium text-ink-100">Catalyst Code</div>
        </div>
        <button
          type="button"
          onClick={() => void refresh()}
          className="rounded-md p-1.5 text-ink-500 hover:bg-ink-800 hover:text-ink-100"
          title="Refresh version status"
          aria-label="Refresh version status"
          disabled={loading}
        >
          <RefreshIcon width={14} height={14} className={loading ? "animate-spin" : ""} />
        </button>
      </div>

      {error && (
        <div className="mt-2 text-[11px] text-danger">{error}</div>
      )}

      {info && (
        <div className="mt-3 space-y-2.5">
          <div className="flex flex-wrap items-center gap-2">
            <span
              className={`inline-flex items-center rounded-md border px-2 py-0.5 text-[11px] font-medium ${STATUS_STYLES[info.status]}`}
            >
              {info.statusLabel}
            </span>
            <span className="font-mono text-[12px] text-ink-200">
              {info.commitUrl ? (
                <a
                  href={info.commitUrl}
                  target="_blank"
                  rel="noreferrer"
                  className="hover:text-accent-soft hover:underline"
                  title={info.commitFull}
                >
                  {info.commit}
                </a>
              ) : (
                <span title={info.commitFull}>{info.commit}</span>
              )}
              {info.dirty ? <span className="text-accent-soft">*</span> : null}
            </span>
          </div>

          <dl className="grid grid-cols-[auto_1fr] gap-x-3 gap-y-1 text-[11px]">
            <dt className="text-ink-500">Source</dt>
            <dd className="font-mono text-ink-300">{info.source}</dd>
            {info.latest && (
              <>
                <dt className="text-ink-500">Latest release</dt>
                <dd className="font-mono text-ink-300">
                  {info.latestUrl ? (
                    <a
                      href={info.latestUrl}
                      target="_blank"
                      rel="noreferrer"
                      className="hover:text-accent-soft hover:underline"
                    >
                      {info.latest}
                    </a>
                  ) : (
                    info.latest
                  )}
                </dd>
              </>
            )}
            {formatBuiltAt(info.builtAt) && (
              <>
                <dt className="text-ink-500">Built</dt>
                <dd className="text-ink-300">{formatBuiltAt(info.builtAt)}</dd>
              </>
            )}
          </dl>

          {info.status === "out_of_date" && (
            <p className="text-[11px] leading-snug text-ink-500">
              A newer release is available. Update with{" "}
              <code className="rounded bg-ink-900 px-1 py-0.5 font-mono text-[10px] text-ink-300">
                bash install.sh --update
              </code>
              {" "}(or re-run with{" "}
              <code className="rounded bg-ink-900 px-1 py-0.5 font-mono text-[10px] text-ink-300">
                --with-web
              </code>
              ).
            </p>
          )}
          {info.status === "uncommitted" && (
            <p className="text-[11px] leading-snug text-ink-500">
              This checkout has local changes that are not committed. The running
              build may not match git HEAD.
            </p>
          )}
        </div>
      )}

      {!info && !error && loading && (
        <div className="mt-2 text-[11px] text-ink-500">Loading version…</div>
      )}
    </div>
  );
}
