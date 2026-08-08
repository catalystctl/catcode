"use client";

import { useEffect, useState } from "react";
import type { InstalledMarketplaceSkill, MarketplaceSkill } from "@/lib/types";
import { useBodyScrollLock } from "@/lib/use-body-scroll-lock";
import { useFocusTrap } from "@/lib/use-focus-trap";
import { mergeRefs, useOutsideClose } from "@/lib/use-outside-close";
import {
  DownloadIcon,
  GlobeIcon,
  RefreshIcon,
  SearchIcon,
  ShieldIcon,
  TrashIcon,
  XIcon,
} from "./icons";

interface Props {
  accepted: boolean;
  results: MarketplaceSkill[];
  installed: InstalledMarketplaceSkill[];
  onAccept: () => Promise<void>;
  onSearch: (query: string) => Promise<void>;
  onInstall: (source: string, name: string, scope: "project" | "global") => Promise<void>;
  onUpdate: (name: string, scope: "project" | "global") => Promise<void>;
  onRemove: (name: string, scope: "project" | "global") => Promise<void>;
  onRefresh: () => void;
  onClose: () => void;
}

function installsLabel(count: number): string {
  if (count >= 1_000_000) return `${(count / 1_000_000).toFixed(1)}M installs`;
  if (count >= 1_000) return `${(count / 1_000).toFixed(1)}K installs`;
  return `${count} install${count === 1 ? "" : "s"}`;
}

export function SkillsMarketplace({
  accepted,
  results,
  installed,
  onAccept,
  onSearch,
  onInstall,
  onUpdate,
  onRemove,
  onRefresh,
  onClose,
}: Props) {
  const [query, setQuery] = useState("");
  const [scope, setScope] = useState<"project" | "global">("project");
  const [busy, setBusy] = useState<string | null>(null);
  const [accepting, setAccepting] = useState(false);
  const closeRef = useOutsideClose(onClose, true);
  const trapRef = useFocusTrap<HTMLDivElement>();
  useBodyScrollLock();

  useEffect(() => {
    onRefresh();
  }, [onRefresh]);

  const run = async (id: string, action: () => Promise<void>) => {
    setBusy(id);
    try {
      await action();
      onRefresh();
    } finally {
      setBusy(null);
    }
  };

  const search = () => {
    const value = query.trim();
    if (value.length < 2) return;
    void run("search", () => onSearch(value));
  };

  return (
    <div className="modal-backdrop">
      <div
        ref={mergeRefs(closeRef, trapRef)}
        className="modal-sheet max-w-2xl"
        role="dialog"
        aria-modal="true"
        aria-label="Skills explorer"
      >
        <header className="flex min-h-11 items-center justify-between border-b border-ink-800 px-5 py-3.5">
          <div className="flex items-center gap-2">
            <GlobeIcon width={15} height={15} className="text-accent-soft" />
            <span className="text-[15px] font-semibold text-ink-100">Skills Explorer</span>
            <span className="rounded-sm bg-ink-800 px-2 py-0.5 font-mono text-[10px] text-ink-400">
              skills.sh
            </span>
          </div>
          <button
            onClick={onClose}
            className="focus-ring flex h-11 w-11 items-center justify-center rounded-sm text-ink-400 hover:bg-ink-800 hover:text-ink-100 sm:h-7 sm:w-7"
            aria-label="Close"
          >
            <XIcon width={16} height={16} />
          </button>
        </header>

        {!accepted ? (
          <div className="flex min-h-0 flex-1 items-center justify-center overflow-y-auto p-6">
            <div className="w-full max-w-lg rounded-sm border border-warning/40 bg-ink-900 p-5">
              <div className="mb-3 flex items-center gap-2 text-warning">
                <ShieldIcon width={18} height={18} />
                <h2 className="text-[14px] font-semibold">Third-party skill warning</h2>
              </div>
              <p className="text-[12.5px] leading-5 text-ink-300">
                Skills are third-party instructions and may include scripts, commands, or prompt content.
                A skill can be malicious, unsafe, or misleading and may cause the agent to access, modify,
                or expose data. Catalyst Code and skills.sh cannot guarantee that every skill is safe.
              </p>
              <p className="mt-3 text-[12.5px] leading-5 text-ink-300">
                Review a skill and its source before using it. Install and run skills at your own risk and
                with appropriate caution.
              </p>
              <div className="mt-5 flex justify-end gap-2">
                <button onClick={onClose} className="focus-ring rounded-sm border border-ink-700 px-3 py-2 text-[12px] text-ink-300 hover:bg-ink-800">
                  Cancel
                </button>
                <button
                  disabled={accepting}
                  onClick={() => {
                    setAccepting(true);
                    void onAccept().finally(() => setAccepting(false));
                  }}
                  className="focus-ring rounded-sm bg-warning px-3 py-2 text-[12px] font-semibold text-ink-950 hover:brightness-110 disabled:opacity-60"
                >
                  {accepting ? "Accepting..." : "I understand and accept"}
                </button>
              </div>
            </div>
          </div>
        ) : (
          <div className="min-h-0 flex-1 overflow-y-auto p-4">
            <div className="mb-4 flex flex-col gap-2 sm:flex-row">
              <div className="flex min-w-0 flex-1 items-center rounded-sm border border-ink-700 bg-ink-950 px-3">
                <SearchIcon width={14} height={14} className="shrink-0 text-ink-500" />
                <input
                  value={query}
                  onChange={(event) => setQuery(event.target.value)}
                  onKeyDown={(event) => event.key === "Enter" && search()}
                  placeholder="Search skills.sh"
                  className="min-w-0 flex-1 bg-transparent px-2 py-2 text-[12px] text-ink-100 outline-none placeholder:text-ink-600"
                />
              </div>
              <button
                onClick={search}
                disabled={query.trim().length < 2 || busy === "search"}
                className="focus-ring min-h-11 rounded-sm bg-accent px-4 text-[12px] font-medium text-white disabled:bg-ink-800 disabled:text-ink-500 sm:min-h-0"
              >
                {busy === "search" ? "Searching..." : "Search"}
              </button>
            </div>

            <div className="mb-4 flex w-fit rounded-sm border border-ink-700 bg-ink-950 p-0.5" aria-label="Install scope">
              {(["project", "global"] as const).map((value) => (
                <button
                  key={value}
                  onClick={() => setScope(value)}
                  className={`rounded-sm px-3 py-1.5 text-[11px] font-medium capitalize ${scope === value ? "bg-ink-800 text-ink-100" : "text-ink-500 hover:text-ink-300"}`}
                >
                  {value}
                </button>
              ))}
            </div>

            {installed.length > 0 && (
              <section className="mb-5">
                <div className="mb-2 flex items-center justify-between">
                  <h2 className="font-mono text-[10px] uppercase text-ink-500">Installed</h2>
                  <button onClick={onRefresh} className="focus-ring flex h-7 w-7 items-center justify-center rounded-sm text-ink-500 hover:bg-ink-800" title="Refresh installed skills" aria-label="Refresh installed skills">
                    <RefreshIcon width={13} height={13} />
                  </button>
                </div>
                <div className="space-y-1.5">
                  {installed.map((skill) => {
                    const id = `${skill.scope}:${skill.name}`;
                    return (
                      <div key={id} className="flex items-center gap-2 rounded-sm border border-ink-800 bg-ink-900 px-3 py-2">
                        <div className="min-w-0 flex-1">
                          <div className="truncate font-mono text-[12px] text-ink-100">{skill.name}</div>
                          <div className="truncate text-[10px] text-ink-500">{skill.source} · {skill.scope}</div>
                        </div>
                        <button
                          onClick={() => void run(`update:${id}`, () => onUpdate(skill.name, skill.scope))}
                          disabled={busy !== null}
                          className="focus-ring flex h-8 w-8 items-center justify-center rounded-sm text-ink-400 hover:bg-ink-800 hover:text-ink-100 disabled:opacity-50"
                          title={`Update ${skill.name}`}
                          aria-label={`Update ${skill.name}`}
                        >
                          <RefreshIcon width={13} height={13} />
                        </button>
                        <button
                          onClick={() => void run(`remove:${id}`, () => onRemove(skill.name, skill.scope))}
                          disabled={busy !== null}
                          className="focus-ring flex h-8 w-8 items-center justify-center rounded-sm text-ink-500 hover:bg-ink-800 hover:text-danger disabled:opacity-50"
                          title={`Remove ${skill.name}`}
                          aria-label={`Remove ${skill.name}`}
                        >
                          <TrashIcon width={13} height={13} />
                        </button>
                      </div>
                    );
                  })}
                </div>
              </section>
            )}

            <section>
              <h2 className="mb-2 font-mono text-[10px] uppercase text-ink-500">Results</h2>
              {results.length === 0 ? (
                <div className="py-8 text-center text-[12px] text-ink-600">Search by skill name or capability.</div>
              ) : (
                <div className="space-y-1.5">
                  {results.map((skill) => {
                    const alreadyInstalled = installed.some((entry) => entry.name === skill.name && entry.scope === scope);
                    return (
                      <div key={skill.id} className="flex items-center gap-3 rounded-sm border border-ink-800 bg-ink-900 px-3 py-2.5">
                        <div className="min-w-0 flex-1">
                          <div className="truncate font-mono text-[12px] text-ink-100">{skill.name}</div>
                          <div className="mt-0.5 flex gap-2 text-[10px] text-ink-500">
                            <span className="truncate">{skill.source}</span>
                            <span className="shrink-0">{installsLabel(skill.installs)}</span>
                          </div>
                        </div>
                        <button
                          onClick={() => void run(`install:${skill.id}`, () => onInstall(skill.source, skill.name, scope))}
                          disabled={busy !== null || alreadyInstalled}
                          className="focus-ring flex min-h-11 items-center gap-1.5 rounded-sm border border-ink-700 px-3 text-[11px] text-accent-soft hover:bg-ink-800 disabled:text-ink-600 sm:min-h-0 sm:py-1.5"
                        >
                          <DownloadIcon width={13} height={13} /> {alreadyInstalled ? "Installed" : `Install ${scope}`}
                        </button>
                      </div>
                    );
                  })}
                </div>
              )}
            </section>
          </div>
        )}
      </div>
    </div>
  );
}
