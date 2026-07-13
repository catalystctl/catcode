"use client";

// ProviderLoginModal — pick a first-party provider preset and either start
// OAuth or paste an API key. Replaces the numbered window.prompt /login flow.

import { useState } from "react";
import type { ProviderPreset } from "@/lib/types";
import { useOutsideClose, mergeRefs } from "@/lib/use-outside-close";
import { useFocusTrap } from "@/lib/use-focus-trap";
import { CheckIcon, XIcon, ShieldIcon } from "./icons";

interface Props {
  presets: ProviderPreset[];
  mode: "login" | "logout";
  onLoginKey: (presetId: string, apiKey: string) => void;
  onLoginOauth: (presetId: string) => void;
  onLoginSaved: (presetId: string) => void;
  onSwitchProvider: (presetId: string) => void;
  onLogout: (presetId: string) => void;
  onClose: () => void;
}

export function ProviderLoginModal({
  presets,
  mode,
  onLoginKey,
  onLoginOauth,
  onLoginSaved,
  onSwitchProvider,
  onLogout,
  onClose,
}: Props) {
  const closeRef = useOutsideClose(onClose);
  const trapRef = useFocusTrap<HTMLDivElement>();
  const [selected, setSelected] = useState<string | null>(null);
  const [keyInput, setKeyInput] = useState("");
  const list =
    mode === "logout" ? presets.filter((p) => p.loggedIn) : presets;
  const current = list.find((p) => p.id === selected) ?? null;

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/60 p-4 backdrop-blur-sm">
      <div
        ref={mergeRefs(closeRef, trapRef)}
        className="flex max-h-[85vh] w-full max-w-md flex-col overflow-hidden rounded-2xl border border-ink-700 bg-ink-900 shadow-2xl animate-fade-in"
        role="dialog"
        aria-modal="true"
        aria-label={mode === "logout" ? "Log out of provider" : "Log in to provider"}
      >
        <div className="flex items-center justify-between border-b border-ink-800/80 px-5 py-3.5">
          <div className="flex items-center gap-2">
            <ShieldIcon width={16} height={16} className="text-accent-soft" />
            <h2 className="text-[15px] font-semibold text-ink-100">
              {mode === "logout" ? "Log out" : "Log in / switch provider"}
            </h2>
          </div>
          <button
            onClick={onClose}
            className="rounded-md p-1.5 text-ink-500 hover:bg-ink-800 hover:text-ink-100"
            aria-label="Close"
          >
            <XIcon width={16} height={16} />
          </button>
        </div>

        <div className="min-h-0 flex-1 space-y-1 overflow-y-auto px-5 py-4">
          {list.length === 0 ? (
            <p className="text-[13px] text-ink-500">
              {mode === "logout" ? "Not logged into any provider." : "No provider presets available."}
            </p>
          ) : (
            list.map((p) => {
              const active = selected === p.id;
              return (
                <button
                  key={p.id}
                  onClick={() => {
                    setSelected(p.id);
                    setKeyInput("");
                  }}
                  className={`flex w-full items-start gap-2.5 rounded-xl border px-3.5 py-2.5 text-left transition-colors ${
                    active
                      ? "border-accent/40 bg-accent/10"
                      : "border-ink-700/70 bg-ink-900/70 hover:border-ink-600 hover:bg-ink-850"
                  }`}
                >
                  <div className="min-w-0 flex-1">
                    <div className="flex items-center gap-2 text-[13px] font-medium text-ink-100">
                      {p.loggedIn && <CheckIcon width={12} height={12} className="text-success" />}
                      {p.label}
                    </div>
                    <div className="mt-0.5 text-[11px] text-ink-500">{p.description}</div>
                  </div>
                </button>
              );
            })
          )}
        </div>

        {current && mode === "login" && (
          <div className="space-y-3 border-t border-ink-800/80 px-5 py-4">
            {current.loggedIn && (
              <button
                onClick={() => {
                  onSwitchProvider(current.id);
                  onClose();
                }}
                className="w-full rounded-lg bg-accent px-3.5 py-2 text-[13px] font-semibold text-white hover:bg-accent-soft"
              >
                Switch to {current.label}
              </button>
            )}
            {!current.loggedIn && current.hasKey && (
              <button
                onClick={() => {
                  onLoginSaved(current.id);
                  onClose();
                }}
                className="w-full rounded-lg bg-accent px-3.5 py-2 text-[13px] font-semibold text-white hover:bg-accent-soft"
              >
                Use saved credentials
              </button>
            )}
            {!current.loggedIn && current.supportsOauth && (
              <button
                onClick={() => {
                  onLoginOauth(current.id);
                  onClose();
                }}
                className="w-full rounded-lg border border-accent/40 bg-accent/10 px-3.5 py-2 text-[13px] font-semibold text-accent-soft hover:bg-accent/20"
              >
                Continue with OAuth
              </button>
            )}
            <div>
              <label className="mb-1.5 block text-[11px] font-semibold uppercase tracking-wider text-ink-500">
                {current.loggedIn ? "Override API key" : "API key"}
                {current.supportsOauth && !current.loggedIn ? " (optional)" : ""}
              </label>
              <input
                type="password"
                value={keyInput}
                onChange={(e) => setKeyInput(e.target.value)}
                placeholder={current.envVar || "sk-…"}
                className="w-full rounded-lg border border-ink-700 bg-ink-950 px-3 py-2 font-mono text-[13px] text-ink-100 placeholder:text-ink-600 focus:border-accent/50 focus:outline-none"
                onKeyDown={(e) => {
                  if (e.key === "Enter" && keyInput.trim()) {
                    onLoginKey(current.id, keyInput.trim());
                    onClose();
                  }
                }}
              />
            </div>
            <button
              disabled={!keyInput.trim()}
              onClick={() => {
                if (!keyInput.trim()) return;
                onLoginKey(current.id, keyInput.trim());
                onClose();
              }}
              className="w-full rounded-lg border border-ink-700 bg-ink-850 px-3.5 py-2 text-[13px] font-medium text-ink-100 hover:bg-ink-800 disabled:opacity-40"
            >
              Save API key
            </button>
          </div>
        )}

        {current && mode === "logout" && (
          <div className="border-t border-ink-800/80 px-5 py-4">
            <button
              onClick={() => {
                onLogout(current.id);
                onClose();
              }}
              className="w-full rounded-lg border border-danger/40 bg-danger/10 px-3.5 py-2 text-[13px] font-semibold text-danger hover:bg-danger/20"
            >
              Log out of {current.label}
            </button>
          </div>
        )}
      </div>
    </div>
  );
}
