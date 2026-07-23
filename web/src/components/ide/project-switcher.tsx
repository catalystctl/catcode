"use client";

import { useCallback, useEffect, useMemo, useState } from "react";
import type { ProjectEntry } from "@/lib/types";
import { basename } from "@/lib/format";
import { useFocusTrap } from "@/lib/use-focus-trap";
import {
  CheckIcon,
  ChevronLeft,
  ChevronRight,
  FolderIcon,
  FolderPlusIcon,
  HomeIcon,
  SearchIcon,
  XIcon,
} from "@/components/icons";

interface ProjectSwitcherProps {
  workspace: string;
  projects: ProjectEntry[];
  switching: boolean;
  mobile?: boolean;
  /** Return false when the switch was cancelled (for example by a dirty-file guard). */
  onSwitchWorkspace: (path: string) => boolean | void;
  onRemoveProject: (path: string) => void;
  onClose: () => void;
}

type BrowseEntry = { name: string; path: string };
type BrowseResponse = {
  path: string;
  parent: string | null;
  home: string;
  entries: BrowseEntry[];
  error?: string;
};

type Mode = "recent" | "browse";

function pathSegments(abs: string): Array<{ label: string; path: string }> {
  if (!abs) return [{ label: "/", path: "/" }];

  // Absolute Windows path: C:\Users\foo or C:/Users/foo
  const winMatch = /^([A-Za-z]:)([\\/].*)?$/.exec(abs);
  if (winMatch) {
    const drive = winMatch[1]; // "C:"
    const rest = (winMatch[2] ?? "").replace(/\//g, "\\");
    const parts = rest.split("\\").filter(Boolean);
    const out: Array<{ label: string; path: string }> = [
      { label: drive, path: `${drive}\\` },
    ];
    let acc = `${drive}\\`;
    for (const part of parts) {
      acc = acc.endsWith("\\") ? `${acc}${part}` : `${acc}\\${part}`;
      out.push({ label: part, path: acc });
    }
    return out;
  }

  const normalized = abs.replace(/\\/g, "/");
  if (normalized === "/") return [{ label: "/", path: "/" }];
  const parts = normalized.split("/").filter(Boolean);
  const out: Array<{ label: string; path: string }> = [];
  let acc = "";
  for (const part of parts) {
    acc = `${acc}/${part}`;
    out.push({ label: part, path: acc });
  }
  return out;
}

export function ProjectSwitcher({
  workspace,
  projects,
  switching,
  mobile = false,
  onSwitchWorkspace,
  onRemoveProject,
  onClose,
}: ProjectSwitcherProps) {
  const [mode, setMode] = useState<Mode>("recent");
  const [filter, setFilter] = useState("");
  const [pathInput, setPathInput] = useState("");
  const [browsePath, setBrowsePath] = useState<string | null>(null);
  const [browseHome, setBrowseHome] = useState<string | null>(null);
  const [browseParent, setBrowseParent] = useState<string | null>(null);
  const [entries, setEntries] = useState<BrowseEntry[]>([]);
  const [browseLoading, setBrowseLoading] = useState(false);
  const [browseError, setBrowseError] = useState<string | null>(null);
  const trapRef = useFocusTrap<HTMLDivElement>(true);
  const current = projects.find((project) => project.path === workspace);

  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape") onClose();
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [onClose]);

  const filteredProjects = useMemo(() => {
    const q = filter.trim().toLowerCase();
    if (!q) return projects;
    return projects.filter(
      (p) => p.name.toLowerCase().includes(q) || p.path.toLowerCase().includes(q),
    );
  }, [projects, filter]);

  const loadBrowse = useCallback(async (path?: string) => {
    setBrowseLoading(true);
    setBrowseError(null);
    try {
      const qs = path ? `?path=${encodeURIComponent(path)}` : "";
      const res = await fetch(`/api/browse${qs}`, { cache: "no-store" });
      const data = (await res.json()) as BrowseResponse;
      if (!res.ok) {
        setBrowseError(data.error || `Browse failed (${res.status})`);
        setEntries([]);
        return;
      }
      setBrowsePath(data.path);
      setBrowseHome(data.home);
      setBrowseParent(data.parent);
      setEntries(data.entries);
      setPathInput(data.path);
    } catch (err) {
      setBrowseError(err instanceof Error ? err.message : String(err));
      setEntries([]);
    } finally {
      setBrowseLoading(false);
    }
  }, []);

  useEffect(() => {
    if (mode !== "browse") return;
    if (browsePath) return;
    void loadBrowse(workspace || undefined);
  }, [mode, browsePath, workspace, loadBrowse]);

  const switchTo = (path: string) => {
    const next = path.trim();
    if (!next) return;
    if (onSwitchWorkspace(next) === false) return;
    onClose();
  };

  const crumbs = browsePath ? pathSegments(browsePath) : [];
  const visibleCrumbs =
    crumbs.length > 4 ? [crumbs[0], ...crumbs.slice(-3)] : crumbs;

  return (
    <div
      className="fixed inset-0 z-[70] flex justify-center bg-black/55 px-2 pt-[max(env(safe-area-inset-top),0.75rem)] backdrop-blur-[2px] sm:px-4"
      onMouseDown={onClose}
    >
    <section
      ref={trapRef}
      role="dialog"
      aria-modal="true"
      aria-label="Switch project"
      onMouseDown={(event) => event.stopPropagation()}
      className={`flex max-h-[min(36rem,calc(100dvh-4.5rem))] w-full flex-col overflow-hidden rounded-xl border border-ink-700 bg-ink-925 shadow-2xl shadow-black/50 animate-fade-in ${
        mobile ? "max-w-lg" : "max-w-md sm:absolute sm:bottom-3 sm:left-14 sm:max-w-none sm:w-[26rem]"
      }`}
    >
      <header className="border-b border-ink-800 px-3 pt-3 pb-0">
        <div className="mb-2.5 flex items-start gap-2">
          <div className="flex h-8 w-8 shrink-0 items-center justify-center rounded-lg bg-accent/15 text-accent-soft">
            <FolderIcon width={15} height={15} />
          </div>
          <div className="min-w-0 flex-1">
            <h2 className="truncate text-[13px] font-semibold text-ink-100">
              {current?.name ?? basename(workspace) ?? "Project"}
            </h2>
            <p className="truncate font-mono text-[10px] text-ink-500">
              {switching ? "Switching project…" : workspace}
            </p>
          </div>
          <button
            type="button"
            onClick={onClose}
            className="rounded-md p-1 text-ink-500 hover:bg-ink-800 hover:text-ink-100"
            aria-label="Close project switcher"
          >
            <XIcon width={14} height={14} />
          </button>
        </div>

        <div className="flex gap-1" role="tablist" aria-label="Project views">
          {(
            [
              { id: "recent", label: "Recent" },
              { id: "browse", label: "Browse" },
            ] as const
          ).map((tab) => {
            const active = mode === tab.id;
            return (
              <button
                key={tab.id}
                type="button"
                role="tab"
                aria-selected={active}
                onClick={() => setMode(tab.id)}
                className={`relative flex-1 rounded-t-md px-3 py-2 text-[12px] font-medium transition-colors ${
                  active
                    ? "bg-ink-900 text-ink-100"
                    : "text-ink-500 hover:bg-ink-900/50 hover:text-ink-300"
                }`}
              >
                {tab.label}
                {active ? (
                  <span className="absolute inset-x-3 -bottom-px h-0.5 rounded-full bg-accent" />
                ) : null}
              </button>
            );
          })}
        </div>
      </header>

      {mode === "recent" ? (
        <>
          <div className="border-b border-ink-800 px-2.5 py-2">
            <label className="flex items-center gap-2 rounded-lg border border-ink-800 bg-ink-950 px-2.5 py-1.5">
              <SearchIcon width={13} height={13} className="shrink-0 text-ink-500" />
              <input
                autoFocus
                value={filter}
                onChange={(event) => setFilter(event.target.value)}
                placeholder="Filter recent projects…"
                aria-label="Filter recent projects"
                className="min-w-0 flex-1 bg-transparent text-[12px] text-ink-100 outline-none placeholder:text-ink-600"
              />
              {filter ? (
                <button
                  type="button"
                  onClick={() => setFilter("")}
                  className="rounded p-0.5 text-ink-500 hover:text-ink-200"
                  aria-label="Clear filter"
                >
                  <XIcon width={12} height={12} />
                </button>
              ) : null}
            </label>
          </div>

          <div className="min-h-0 flex-1 overflow-y-auto p-1.5">
            {filteredProjects.length === 0 && (
              <div className="px-3 py-8 text-center">
                <p className="text-[12px] text-ink-500">
                  {projects.length === 0 ? "No recent projects yet." : "No matching projects."}
                </p>
                <button
                  type="button"
                  onClick={() => setMode("browse")}
                  className="mt-3 inline-flex items-center gap-1.5 rounded-lg border border-ink-700 px-3 py-1.5 text-[11px] font-medium text-ink-300 hover:bg-ink-850 hover:text-ink-100"
                >
                  <FolderPlusIcon width={12} height={12} />
                  Browse for a folder
                </button>
              </div>
            )}
            {filteredProjects.map((project) => {
              const active = project.path === workspace;
              return (
                <div
                  key={project.path}
                  className={`group/project flex items-center gap-1 rounded-lg px-1.5 py-1 ${
                    active ? "bg-accent/12" : "hover:bg-ink-850"
                  }`}
                >
                  <button
                    type="button"
                    disabled={switching}
                    onClick={() => (active ? onClose() : switchTo(project.path))}
                    className="flex min-w-0 flex-1 items-center gap-2.5 rounded-md px-1.5 py-1.5 text-left disabled:opacity-50"
                  >
                    <span
                      className={`flex h-7 w-7 shrink-0 items-center justify-center rounded-md ${
                        active ? "bg-accent/20 text-accent-soft" : "bg-ink-900 text-ink-500"
                      }`}
                    >
                      <FolderIcon width={14} height={14} />
                    </span>
                    <span className="min-w-0 flex-1">
                      <span className="block truncate text-[12px] font-medium text-ink-100">
                        {project.name}
                      </span>
                      <span className="block truncate font-mono text-[10px] text-ink-500">
                        {project.path}
                      </span>
                    </span>
                    {active && (
                      <CheckIcon width={13} height={13} className="shrink-0 text-accent-soft" />
                    )}
                  </button>
                  <button
                    type="button"
                    onClick={() => onRemoveProject(project.path)}
                    className="rounded p-1 text-ink-600 opacity-100 hover:bg-danger/10 hover:text-danger sm:opacity-0 sm:group-hover/project:opacity-100 sm:focus:opacity-100"
                    title="Remove from recent projects"
                    aria-label={`Remove ${project.name} from recent projects`}
                  >
                    <XIcon width={12} height={12} />
                  </button>
                </div>
              );
            })}
          </div>

          <div className="border-t border-ink-800 p-2">
            <button
              type="button"
              disabled={switching}
              onClick={() => setMode("browse")}
              className="flex w-full items-center justify-center gap-2 rounded-lg border border-ink-700 bg-ink-900 px-3 py-2 text-[12px] font-medium text-ink-200 transition-colors hover:border-ink-600 hover:bg-ink-850 hover:text-ink-100 disabled:opacity-50"
            >
              <FolderPlusIcon width={14} height={14} className="text-accent-soft" />
              Add project by browsing…
            </button>
          </div>
        </>
      ) : (
        <>
          <div className="space-y-2 border-b border-ink-800 px-2.5 py-2">
            <form
              className="flex items-center gap-1.5"
              onSubmit={(event) => {
                event.preventDefault();
                void loadBrowse(pathInput.trim() || undefined);
              }}
            >
              <input
                value={pathInput}
                disabled={switching || browseLoading}
                onChange={(event) => setPathInput(event.target.value)}
                placeholder="/path/to/project"
                aria-label="Directory path"
                className="min-w-0 flex-1 rounded-lg border border-ink-700 bg-ink-950 px-2.5 py-1.5 font-mono text-[11px] text-ink-200 placeholder:text-ink-600 focus:border-accent/50 focus:outline-none disabled:opacity-50"
              />
              <button
                type="submit"
                disabled={switching || browseLoading}
                className="rounded-lg border border-ink-700 px-2.5 py-1.5 text-[11px] font-medium text-ink-300 hover:bg-ink-850 disabled:opacity-40"
              >
                Go
              </button>
            </form>

            <div className="flex items-center gap-1">
              <button
                type="button"
                disabled={!browseParent || browseLoading || switching}
                onClick={() => browseParent && void loadBrowse(browseParent)}
                className="rounded-md p-1.5 text-ink-400 hover:bg-ink-850 hover:text-ink-100 disabled:opacity-30"
                title="Go up"
                aria-label="Go up one directory"
              >
                <ChevronLeft width={14} height={14} />
              </button>
              <button
                type="button"
                disabled={!browseHome || browseLoading || switching}
                onClick={() => browseHome && void loadBrowse(browseHome)}
                className="rounded-md p-1.5 text-ink-400 hover:bg-ink-850 hover:text-ink-100 disabled:opacity-30"
                title="Home"
                aria-label="Go to home directory"
              >
                <HomeIcon width={14} height={14} />
              </button>
              <div className="min-w-0 flex-1 overflow-x-auto">
                <div className="flex items-center gap-0.5 whitespace-nowrap px-0.5">
                  {crumbs.length > 4 ? (
                    <>
                      <Crumb
                        label={visibleCrumbs[0]?.label ?? "/"}
                        onClick={() => visibleCrumbs[0] && void loadBrowse(visibleCrumbs[0].path)}
                        disabled={browseLoading || switching}
                      />
                      <span className="px-0.5 text-[10px] text-ink-600">…</span>
                      {visibleCrumbs.slice(1).map((c, i) => (
                        <span key={c.path} className="flex items-center">
                          {i > 0 || visibleCrumbs.length > 1 ? (
                            <ChevronRight width={10} height={10} className="text-ink-600" />
                          ) : null}
                          <Crumb
                            label={c.label}
                            onClick={() => void loadBrowse(c.path)}
                            disabled={browseLoading || switching}
                            active={c.path === browsePath}
                          />
                        </span>
                      ))}
                    </>
                  ) : (
                    crumbs.map((c, i) => (
                      <span key={c.path} className="flex items-center">
                        {i > 0 ? (
                          <ChevronRight width={10} height={10} className="text-ink-600" />
                        ) : null}
                        <Crumb
                          label={c.label}
                          onClick={() => void loadBrowse(c.path)}
                          disabled={browseLoading || switching}
                          active={c.path === browsePath}
                        />
                      </span>
                    ))
                  )}
                </div>
              </div>
            </div>
          </div>

          <div className="min-h-0 flex-1 overflow-y-auto p-1.5">
            {browseLoading && (
              <div className="px-3 py-8 text-center text-[12px] text-ink-500">Loading…</div>
            )}
            {!browseLoading && browseError && (
              <div className="px-3 py-6 text-center text-[12px] text-danger">{browseError}</div>
            )}
            {!browseLoading && !browseError && entries.length === 0 && (
              <div className="px-3 py-8 text-center text-[12px] text-ink-500">
                No subfolders here.
              </div>
            )}
            {!browseLoading &&
              !browseError &&
              entries.map((entry) => {
                const already = projects.some((p) => p.path === entry.path);
                const active = entry.path === workspace;
                return (
                  <button
                    key={entry.path}
                    type="button"
                    disabled={switching}
                    onClick={() => void loadBrowse(entry.path)}
                    onDoubleClick={() => switchTo(entry.path)}
                    className={`flex w-full items-center gap-2.5 rounded-lg px-2.5 py-2 text-left transition-colors disabled:opacity-50 ${
                      active ? "bg-accent/12" : "hover:bg-ink-850"
                    }`}
                  >
                    <span className="flex h-7 w-7 shrink-0 items-center justify-center rounded-md bg-ink-900 text-ink-400">
                      <FolderIcon width={14} height={14} />
                    </span>
                    <span className="min-w-0 flex-1 truncate text-[12px] font-medium text-ink-100">
                      {entry.name}
                    </span>
                    {already ? (
                      <span className="shrink-0 rounded bg-ink-800 px-1.5 py-0.5 text-[9px] uppercase tracking-wide text-ink-400">
                        recent
                      </span>
                    ) : null}
                    <ChevronRight width={12} height={12} className="shrink-0 text-ink-600" />
                  </button>
                );
              })}
          </div>

          <div className="flex items-center gap-2 border-t border-ink-800 p-2">
            <button
              type="button"
              disabled={switching || !browsePath}
              onClick={() => browsePath && switchTo(browsePath)}
              className="flex min-w-0 flex-1 items-center justify-center gap-2 rounded-lg bg-accent px-3 py-2 text-[12px] font-semibold text-white transition-colors hover:bg-accent-soft disabled:opacity-40"
            >
              <FolderPlusIcon width={14} height={14} />
              <span className="truncate">
                Open {browsePath ? basename(browsePath) || browsePath : "folder"}
              </span>
            </button>
          </div>
        </>
      )}
    </section>
    </div>
  );
}

function Crumb({
  label,
  onClick,
  disabled,
  active,
}: {
  label: string;
  onClick: () => void;
  disabled?: boolean;
  active?: boolean;
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      disabled={disabled}
      className={`max-w-[7rem] truncate rounded px-1 py-0.5 text-[11px] ${
        active
          ? "font-medium text-ink-100"
          : "text-ink-400 hover:bg-ink-850 hover:text-ink-200"
      } disabled:opacity-40`}
    >
      {label}
    </button>
  );
}
