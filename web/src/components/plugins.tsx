"use client";

// PluginsPanel — install/remove/enable/disable core plugins.

import { useEffect, useRef, useState } from "react";
import type { PluginEntry } from "@/lib/types";
import { TerminalIcon, PlusIcon, TrashIcon, XIcon } from "./icons";

interface Props {
  plugins: PluginEntry[];
  onInstall: (path: string) => void;
  onRemove: (name: string) => void;
  onEnable: (name: string) => void;
  onDisable: (name: string) => void;
  onClose: () => void;
}

function useOutsideClose(onClose: () => void) {
  const ref = useRef<HTMLDivElement>(null);
  useEffect(() => {
    const h = (e: MouseEvent) => {
      if (ref.current && !ref.current.contains(e.target as Node)) onClose();
    };
    const k = (e: KeyboardEvent) => {
      if (e.key === "Escape") onClose();
    };
    document.addEventListener("mousedown", h);
    document.addEventListener("keydown", k);
    return () => {
      document.removeEventListener("mousedown", h);
      document.removeEventListener("keydown", k);
    };
  }, [onClose]);
  return ref;
}

export function PluginsPanel({ plugins, onInstall, onRemove, onEnable, onDisable, onClose }: Props) {
  const [path, setPath] = useState("");
  const ref = useOutsideClose(onClose);

  const install = () => {
    const p = path.trim();
    if (!p) return;
    onInstall(p);
    setPath("");
  };

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/60 p-4 backdrop-blur-sm">
      <div
        ref={ref}
        className="flex max-h-[80vh] w-full max-w-lg flex-col rounded-2xl border border-ink-700 bg-ink-900 shadow-2xl animate-fade-in"
      >
        <div className="flex items-center justify-between border-b border-ink-800/80 px-4 py-3">
          <div className="flex items-center gap-2">
            <TerminalIcon width={15} height={15} className="text-accent-soft" />
            <span className="text-[13px] font-semibold text-ink-100">Plugins</span>
          </div>
          <button
            onClick={onClose}
            className="rounded-md p-1 text-ink-500 hover:bg-ink-800 hover:text-ink-100"
          >
            <XIcon width={16} height={16} />
          </button>
        </div>

        <div className="flex-1 overflow-y-auto p-4">
          {/* Install */}
          <div className="mb-3 flex items-center gap-2 rounded-xl border border-ink-800 bg-ink-925/40 p-3">
            <input
              value={path}
              onChange={(e) => setPath(e.target.value)}
              onKeyDown={(e) => {
                if (e.key === "Enter") install();
              }}
              placeholder="/abs/path/to/plugin or ./plugin.wasm"
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

          {/* List */}
          {plugins.length === 0 ? (
            <div className="px-3 py-6 text-center text-[12px] text-ink-600">
              No plugins installed.
            </div>
          ) : (
            <div className="space-y-2">
              {plugins.map((p) => (
                <div
                  key={p.name}
                  className="group rounded-lg border border-ink-800 bg-ink-925/40 px-3 py-2"
                >
                  <div className="flex items-center gap-2">
                    <span className="font-mono text-[12px] text-ink-100">{p.name}</span>
                    <span
                      className={`rounded px-1.5 py-0.5 text-[10px] font-medium uppercase tracking-wide ${
                        p.enabled
                          ? "bg-emerald-500/10 text-emerald-300"
                          : "bg-ink-800 text-ink-500"
                      }`}
                    >
                      {p.enabled ? "on" : "off"}
                    </span>
                    <div className="ml-auto flex items-center gap-1">
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
                          className="rounded-md border border-emerald-500/30 bg-emerald-500/10 px-2 py-1 text-[11px] text-emerald-300 transition-colors hover:bg-emerald-500/20"
                        >
                          Enable
                        </button>
                      )}
                      <button
                        onClick={() => {
                          if (window.confirm(`Remove plugin "${p.name}"?`)) onRemove(p.name);
                        }}
                        className="rounded-md p-1 text-ink-600 opacity-0 transition-opacity hover:bg-rose-500/10 hover:text-rose-400 group-hover:opacity-100"
                        title="Remove"
                      >
                        <TrashIcon width={13} height={13} />
                      </button>
                    </div>
                  </div>
                  {p.description && (
                    <p className="mt-1 text-[11px] text-ink-500">{p.description}</p>
                  )}
                  {p.error && (
                    <p className="mt-1 text-[11px] text-rose-300">{p.error}</p>
                  )}
                </div>
              ))}
            </div>
          )}
        </div>
      </div>
    </div>
  );
}
