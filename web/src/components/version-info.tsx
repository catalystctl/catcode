"use client";

// About / version panel — shows the running git commit and whether this
// install is up to date, out of date, or has uncommitted changes. Can also
// kick off `catcode --update` (CLI + web if installed) from the UI.

import { useCallback, useEffect, useRef, useState } from "react";
import type { VersionInfo, VersionUpdateStatus } from "@/lib/version-types";
import { AppDialogHost, useAppDialog } from "./app-dialog";
import { RefreshIcon } from "./icons";

type VersionResponse = ({ ok: true } & VersionInfo) | { ok: false; error?: string };

type UpdateResponse =
  | {
      ok: true;
      status: "started" | "already_running";
      message: string;
      logPath?: string;
    }
  | { ok: false; error?: string; hint?: string };

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
  const { confirm, dialog } = useAppDialog();
  const [info, setInfo] = useState<VersionInfo | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);
  const [updating, setUpdating] = useState(false);
  const [updateMessage, setUpdateMessage] = useState<string | null>(null);
  const baselineCommit = useRef<string | null>(null);
  const pollTimer = useRef<ReturnType<typeof setInterval> | null>(null);

  const stopPolling = useCallback(() => {
    if (pollTimer.current) {
      clearInterval(pollTimer.current);
      pollTimer.current = null;
    }
  }, []);

  const refresh = useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      const res = await fetch("/api/version", { cache: "no-store" });
      const data = (await res.json()) as VersionResponse;
      if (!res.ok || !data.ok) {
        setInfo(null);
        setError(!data.ok ? data.error || `Request failed (${res.status})` : `Request failed (${res.status})`);
        return null;
      }
      setInfo(data);
      if (data.updating) setUpdating(true);
      return data;
    } catch (err) {
      setInfo(null);
      setError(err instanceof Error ? err.message : String(err));
      return null;
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    void refresh();
    return () => stopPolling();
  }, [refresh, stopPolling]);

  const startPollingForNewCommit = useCallback(
    (previousCommit: string) => {
      stopPolling();
      let attempts = 0;
      pollTimer.current = setInterval(() => {
        void (async () => {
          attempts += 1;
          try {
            const res = await fetch("/api/version", { cache: "no-store" });
            if (!res.ok) return; // service still restarting
            const data = (await res.json()) as VersionResponse;
            if (!data.ok) return;
            setInfo(data);
            if (data.commit !== previousCommit || data.status === "up_to_date") {
              setUpdating(false);
              setUpdateMessage(`Updated to ${data.commit}`);
              stopPolling();
            }
          } catch {
            // Expected while the web service restarts.
          }
          if (attempts >= 60) {
            setUpdating(false);
            setUpdateMessage("Update started — refresh the page once the service is back.");
            stopPolling();
          }
        })();
      }, 2_000);
    },
    [stopPolling],
  );

  const runUpdate = useCallback(async () => {
    const ok = await confirm({
      title: "Update Catalyst Code?",
      message:
        "This runs catcode --update (or install.sh --update). It refreshes the CLI and, when installed, the web frontend + core, then restarts the web service. The page may disconnect briefly.",
      confirmLabel: "Update now",
    });
    if (!ok) return;

    setUpdating(true);
    setUpdateMessage(null);
    setError(null);
    baselineCommit.current = info?.commit ?? null;

    try {
      const res = await fetch("/api/version/update", { method: "POST" });
      const data = (await res.json()) as UpdateResponse;
      if (!res.ok || !data.ok) {
        setUpdating(false);
        const errMsg = !data.ok ? data.error || `Update failed (${res.status})` : `Update failed (${res.status})`;
        const hint = !data.ok && "hint" in data && data.hint ? ` — ${data.hint}` : "";
        setError(`${errMsg}${hint}`);
        return;
      }
      setUpdateMessage(data.message);
      startPollingForNewCommit(baselineCommit.current || info?.commit || "");
    } catch (err) {
      // Connection drop is common once the service restarts mid-request.
      setUpdateMessage(
        err instanceof Error
          ? `${err.message} — waiting for the service to come back…`
          : "Update started — waiting for the service to come back…",
      );
      startPollingForNewCommit(baselineCommit.current || info?.commit || "");
    }
  }, [confirm, info?.commit, startPollingForNewCommit]);

  const showUpdateButton =
    info &&
    info.canSelfUpdate !== false &&
    (info.status === "out_of_date" || info.status === "unknown" || info.status === "ahead");

  return (
    <div>
      <AppDialogHost dialog={dialog} />
      <div className="flex items-start justify-between gap-2">
        <div>
          <div className="text-[13px] font-medium text-ink-100">Catalyst Code</div>
          <div className="mt-0.5 text-[11px] text-ink-500">Running build vs latest release</div>
        </div>
        <button
          type="button"
          onClick={() => void refresh()}
          className="rounded-md p-1.5 text-ink-500 hover:bg-ink-800 hover:text-ink-100"
          title="Refresh version status"
          aria-label="Refresh version status"
          disabled={loading || updating}
        >
          <RefreshIcon width={14} height={14} className={loading ? "animate-spin" : ""} />
        </button>
      </div>

      {error && <div className="mt-2 text-[11px] text-danger">{error}</div>}
      {updateMessage && <div className="mt-2 text-[11px] text-ink-300">{updateMessage}</div>}

      {info && (
        <div className="mt-3 space-y-2.5">
          <div className="flex flex-wrap items-center gap-2">
            <span
              className={`inline-flex items-center rounded-md border px-2 py-0.5 text-[11px] font-medium ${
                STATUS_STYLES[info.status]
              }`}
            >
              {updating ? "Updating…" : info.statusLabel}
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
            {info.webInstallDetected != null && (
              <>
                <dt className="text-ink-500">Web install</dt>
                <dd className="text-ink-300">{info.webInstallDetected ? "detected" : "not detected"}</dd>
              </>
            )}
          </dl>

          {showUpdateButton && (
            <button
              type="button"
              onClick={() => void runUpdate()}
              disabled={updating}
              className="mt-1 w-full rounded-lg bg-accent px-3.5 py-2 text-[12px] font-semibold text-white transition-colors hover:bg-accent-soft disabled:cursor-not-allowed disabled:bg-ink-800 disabled:text-ink-500"
            >
              {updating ? "Updating CLI + frontend…" : "Update CLI + frontend"}
            </button>
          )}

          {info.status === "out_of_date" && !updating && (
            <p className="text-[11px] leading-snug text-ink-500">
              A newer release is available. Use the button above, or run{" "}
              <code className="rounded bg-ink-900 px-1 py-0.5 font-mono text-[10px] text-ink-300">
                catcode --update
              </code>
              {" "}
              /{" "}
              <code className="rounded bg-ink-900 px-1 py-0.5 font-mono text-[10px] text-ink-300">
                bash install.sh --update
              </code>
              .
            </p>
          )}
          {info.status === "uncommitted" && (
            <p className="text-[11px] leading-snug text-ink-500">
              This checkout has local changes that are not committed. The running build may not match
              git HEAD.
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
