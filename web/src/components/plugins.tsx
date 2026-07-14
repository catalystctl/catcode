"use client";

// PluginsPanel — install/remove/enable/disable core plugins, and inspect what
// each plugin does. Each card expands to reveal its version, registered hook
// points, description, and source path — so you can make an informed decision
// before enabling/disabling.

import { useState } from "react";
import type { PluginEntry } from "@/lib/types";
import { useOutsideClose, mergeRefs } from "@/lib/use-outside-close";
import { useFocusTrap } from "@/lib/use-focus-trap";
import { AppDialogHost, useAppDialog } from "./app-dialog";
import {
  TerminalIcon,
  PlusIcon,
  TrashIcon,
  XIcon,
  ChevronDown,
  BoltIcon,
  CheckIcon,
} from "./icons";

interface Props {
  plugins: PluginEntry[];
  onInstall: (path: string, scope?: "workspace" | "global") => void;
  onRemove: (name: string) => void;
  onEnable: (name: string) => void;
  onDisable: (name: string) => void;
  onClose: () => void;
}

const HOOK_DESCRIPTIONS: Record<string, string> = {
  pre_bash: "inspect/modify bash before run",
  post_bash: "react after bash completes",
  pre_write: "inspect/modify file writes",
  post_write: "react after file writes",
  pre_read: "redirect or gate reads",
  post_read: "react after reads",
  pre_tool: "catch-all: any tool (pre)",
  post_tool: "catch-all: any tool (post)",
  session_start: "fires when a session begins",
  session_stop: "fires when a session ends",
  pre_compact: "before context compaction",
  pre_turn: "remap model per turn",
};

function PluginCard({
  p,
  onRemove,
  onEnable,
  onDisable,
}: {
  p: PluginEntry;
  onRemove: (name: string) => void;
  onEnable: (name: string) => void;
  onDisable: (name: string) => void;
}) {
  const [expanded, setExpanded] = useState(false);
  const hooks = p.hooks ?? [];
  const hasDetails = hooks.length > 0 || !!p.version || !!p.path;

  return (
    <div className="group rounded-lg border border-ink-800 bg-ink-925/40">
      {/* Header row */}
      <div className="flex items-center gap-2 px-3 py-2.5">
        <button
          onClick={() => hasDetails && setExpanded((e) => !e)}
          className={`flex min-w-0 flex-1 items-center gap-2 text-left ${
            hasDetails ? "cursor-pointer" : "cursor-default"
          }`}
        >
          {hasDetails && (
            <ChevronDown
              width={13}
              height={13}
              className={`shrink-0 text-ink-500 transition-transform ${expanded ? "rotate-180" : ""}`}
            />
          )}
          <span className="truncate font-mono text-[12.5px] text-ink-100">{p.name}</span>
          {p.version && (
            <span className="shrink-0 rounded bg-ink-800 px-1.5 py-0.5 text-[9px] text-ink-400">
              v{p.version}
            </span>
          )}
          <span
            className={`flex shrink-0 items-center gap-1 rounded px-1.5 py-0.5 text-[10px] font-medium uppercase tracking-wide ${
              p.enabled
                ? "bg-success/10 text-success"
                : "bg-ink-800 text-ink-500"
            }`}
          >
            {p.enabled ? <CheckIcon width={9} height={9} /> : null}
            {p.enabled ? "on" : "off"}
          </span>
          {hooks.length > 0 && !expanded && (
            <span className="ml-auto hidden truncate text-[10px] text-ink-600 sm:block">
              {hooks.length} hook{hooks.length === 1 ? "" : "s"}
            </span>
          )}
        </button>
        <div className="flex shrink-0 items-center gap-1">
          {p.enabled ? (
            <button
              onClick={() => onDisable(p.name)}
              className="rounded-md border border-ink-700 px-2 py-1 text-[11px] text-ink-300 transition-colors hover:border-ink-600 hover:bg-ink-850"
            >
              Disable
            </button>
          ) : (
            <button
              onClick={() => onEnable(p.name)}
              className="rounded-md border border-success/30 bg-success/10 px-2 py-1 text-[11px] text-success transition-colors hover:bg-success/20"
            >
              Enable
            </button>
          )}
          <button
            onClick={() => void onRemove(p.name)}
            className="rounded-md p-1 text-ink-600 opacity-100 transition-opacity hover:bg-danger/10 hover:text-danger sm:opacity-0 sm:group-hover:opacity-100"
            title="Remove"
            aria-label={`Remove ${p.name}`}
          >
            <TrashIcon width={13} height={13} />
          </button>
        </div>
      </div>

      {/* One-line description (collapsed) */}
      {p.description && !expanded && (
        <div className="px-3 pb-2 text-[11px] text-ink-500">{p.description}</div>
      )}
      {p.error && !expanded && (
        <div className="px-3 pb-2 text-[11px] text-danger">{p.error}</div>
      )}

      {/* Expanded details */}
      {expanded && hasDetails && (
        <div className="space-y-2.5 border-t border-ink-800/60 px-3 py-2.5">
          {p.description && (
            <div>
              <div className="text-[10px] font-medium uppercase tracking-wider text-ink-600">
                Description
              </div>
              <p className="mt-0.5 text-[12px] text-ink-300">{p.description}</p>
            </div>
          )}
          {hooks.length > 0 && (
            <div>
              <div className="text-[10px] font-medium uppercase tracking-wider text-ink-600">
                Hooks
              </div>
              <div className="mt-1 flex flex-wrap gap-1.5">
                {hooks.map((h) => (
                  <span
                    key={h}
                    title={HOOK_DESCRIPTIONS[h]}
                    className="inline-flex items-center gap-1 rounded border border-ink-700/70 bg-ink-900 px-1.5 py-0.5 font-mono text-[10px] text-ink-300"
                  >
                    <BoltIcon width={9} height={9} className="text-accent-soft" />
                    {h}
                  </span>
                ))}
              </div>
            </div>
          )}
          {p.path && (
            <div>
              <div className="text-[10px] font-medium uppercase tracking-wider text-ink-600">
                Source
              </div>
              <p className="mt-0.5 break-all font-mono text-[10px] text-ink-500">{p.path}</p>
            </div>
          )}
          {p.error && (
            <div>
              <div className="text-[10px] font-medium uppercase tracking-wider text-danger">
                Error
              </div>
              <p className="mt-0.5 text-[11px] text-danger">{p.error}</p>
            </div>
          )}
        </div>
      )}
    </div>
  );
}

export function PluginsPanel({
  plugins,
  onInstall,
  onRemove,
  onEnable,
  onDisable,
  onClose,
}: Props) {
  const [path, setPath] = useState("");
  const [scope, setScope] = useState<"workspace" | "global">("workspace");
  const { confirm, dialog } = useAppDialog();
  const closeRef = useOutsideClose(onClose);
  const trapRef = useFocusTrap<HTMLDivElement>();

  const install = () => {
    const p = path.trim();
    if (!p) return;
    onInstall(p, scope);
    setPath("");
  };

  const remove = async (name: string) => {
    const ok = await confirm({
      title: "Remove plugin",
      message: `Remove plugin "${name}"?`,
      confirmLabel: "Remove",
      danger: true,
    });
    if (ok) onRemove(name);
  };

  const enabledCount = plugins.filter((p) => p.enabled).length;

  return (
    <>
      <AppDialogHost dialog={dialog} />
      <div className="modal-backdrop">
      <div
        ref={mergeRefs(closeRef, trapRef)}
        className="modal-sheet max-w-xl"
        role="dialog"
        aria-modal="true"
        aria-label="Plugins"
      >
        <div className="flex items-center justify-between border-b border-ink-800/80 px-4 py-3">
          <div className="flex items-center gap-2">
            <TerminalIcon width={15} height={15} className="text-accent-soft" />
            <span className="text-[13px] font-semibold text-ink-100">Plugins</span>
            {plugins.length > 0 && (
              <span className="rounded-full bg-ink-800 px-2 py-0.5 text-[10px] text-ink-400">
                {enabledCount}/{plugins.length} on
              </span>
            )}
          </div>
          <button
            onClick={onClose}
            className="rounded-md p-1 text-ink-500 hover:bg-ink-800 hover:text-ink-100"
          >
            <XIcon width={16} height={16} />
          </button>
        </div>

        <div className="min-h-0 flex-1 overflow-y-auto p-4">
          {/* Install */}
          <div className="mb-3 space-y-2 rounded-xl border border-ink-800 bg-ink-925/40 p-3">
            <div className="flex items-center gap-2">
              <input
                value={path}
                onChange={(e) => setPath(e.target.value)}
                onKeyDown={(e) => {
                  if (e.key === "Enter") install();
                }}
                placeholder="/path/to/plugin or owner/repo@v1.2.0"
                className="flex-1 rounded-lg border border-ink-700 bg-ink-950 px-3 py-1.5 font-mono text-[12px] text-ink-200 placeholder:text-ink-600 focus:border-accent/50 focus:outline-none"
              />
              <button
                onClick={install}
                disabled={!path.trim()}
                className="flex items-center gap-1.5 rounded-lg bg-accent px-3 py-1.5 text-[12px] font-semibold text-white transition-colors hover:bg-accent-soft disabled:cursor-not-allowed disabled:bg-ink-800 disabled:text-ink-500"
              >
                <PlusIcon width={13} height={13} /> Install
              </button>
            </div>
            <div className="flex rounded-lg border border-ink-700 bg-ink-950 p-0.5 w-fit">
              {(["workspace", "global"] as const).map((s) => (
                <button
                  key={s}
                  type="button"
                  onClick={() => setScope(s)}
                  className={`rounded-md px-2.5 py-1 text-[11px] font-medium capitalize transition-colors ${
                    scope === s
                      ? "bg-ink-800 text-ink-100"
                      : "text-ink-500 hover:text-ink-300"
                  }`}
                >
                  {s}
                </button>
              ))}
            </div>
          </div>

          {/* List */}
          {plugins.length === 0 ? (
            <div className="px-3 py-8 text-center text-[12px] text-ink-600">
              No plugins installed. Drop a plugin directory into{" "}
              <code className="font-mono text-ink-500">.catalyst-code/plugins/</code>,
              install from a path, or paste a GitHub Release URL (
              <code className="font-mono text-ink-500">owner/repo@v1.2.0</code>
              ).
            </div>
          ) : (
            <div className="space-y-2">
              {plugins.map((p) => (
                <PluginCard
                  key={p.name}
                  p={p}
                  onRemove={remove}
                  onEnable={onEnable}
                  onDisable={onDisable}
                />
              ))}
            </div>
          )}
        </div>
      </div>
    </div>
    </>
  );
}
