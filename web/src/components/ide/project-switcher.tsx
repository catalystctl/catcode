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
  GitBranchIcon,
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

type Mode = "recent" | "browse" | "create" | "clone";

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

  // ── Create mode state ──
  const [createName, setCreateName] = useState("");
  const [createParent, setCreateParent] = useState<string>("");
  const [createInitGit, setCreateInitGit] = useState(true);
  const [createReadme, setCreateReadme] = useState(true);
  const [creating, setCreating] = useState(false);
  const [createError, setCreateError] = useState<string | null>(null);

  // ── Clone mode state ──
  const [cloneUrl, setCloneUrl] = useState("");
  const [cloneParent, setCloneParent] = useState<string>("");
  const [cloneName, setCloneName] = useState("");
  const [cloneBranch, setCloneBranch] = useState("");
  const [cloning, setCloning] = useState(false);
  const [cloneError, setCloneError] = useState<string | null>(null);

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

  const createProject = useCallback(async () => {
    const name = createName.trim();
    if (!name) { setCreateError("Enter a project name"); return; }
    setCreating(true);
    setCreateError(null);
    try {
      const res = await fetch("/api/project", {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({
          action: "create",
          name,
          parentDir: createParent.trim() || undefined,
          initGit: createInitGit,
          createReadme: createReadme,
        }),
      });
      const data = (await res.json().catch(() => ({}))) as { ok?: boolean; path?: string; name?: string; error?: string; exists?: boolean };
      if (!res.ok || !data.ok) {
        setCreateError(data.error ?? `Could not create project (${res.status})`);
        return;
      }
      onSwitchWorkspace(data.path ?? "");
      onClose();
    } catch (e) {
      setCreateError(e instanceof Error ? e.message : "Could not create project");
    } finally {
      setCreating(false);
    }
  }, [createName, createParent, createInitGit, createReadme, onSwitchWorkspace, onClose]);

  const cloneRepo = useCallback(async () => {
    const url = cloneUrl.trim();
    if (!url) { setCloneError("Enter a repository URL"); return; }
    setCloning(true);
    setCloneError(null);
    try {
      const res = await fetch("/api/project", {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({
          action: "clone",
          url,
          parentDir: cloneParent.trim() || undefined,
          name: cloneName.trim() || undefined,
          branch: cloneBranch.trim() || undefined,
        }),
      });
      const data = (await res.json().catch(() => ({}))) as { ok?: boolean; path?: string; name?: string; error?: string };
      if (!res.ok || !data.ok) {
        setCloneError(data.error ?? `Could not clone (${res.status})`);
        return;
      }
      onSwitchWorkspace(data.path ?? "");
      onClose();
    } catch (e) {
      setCloneError(e instanceof Error ? e.message : "Could not clone repository");
    } finally {
      setCloning(false);
    }
  }, [cloneUrl, cloneParent, cloneName, cloneBranch, onSwitchWorkspace, onClose]);

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
      className="fixed inset-0 z-[70] flex justify-center bg-ink-950/80 px-2 pt-[max(env(safe-area-inset-top),0.75rem)] sm:px-4"
      onMouseDown={onClose}
    >
    <section
      ref={trapRef}
      role="dialog"
      aria-modal="true"
      aria-label="Switch project"
      onMouseDown={(event) => event.stopPropagation()}
      className={`flex max-h-[min(36rem,calc(100dvh-4.5rem))] w-full flex-col overflow-hidden rounded-sm border border-ink-700 bg-ink-900 shadow-elev-2 animate-fade-in ${
        mobile ? "max-w-lg" : "max-w-md sm:absolute sm:bottom-3 sm:left-14 sm:max-w-none sm:w-[26rem]"
      }`}
    >
      <header className="border-b border-ink-800 px-3 pt-3 pb-0">
        <div className="mb-2.5 flex items-start gap-2">
          <div className="flex h-8 w-8 shrink-0 items-center justify-center rounded-sm border border-ink-700 bg-ink-850 text-accent-soft">
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
            className="flex min-h-11 min-w-11 items-center justify-center rounded-sm p-1 text-ink-500 hover:bg-ink-800 hover:text-ink-100 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-accent sm:min-h-0 sm:min-w-0"
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
              { id: "create", label: "Create" },
              { id: "clone", label: "Clone" },
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
                className={`relative min-h-11 flex-1 rounded-t-sm px-3 py-2 font-mono text-[11px] font-medium uppercase tracking-wider transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-accent ${
                  active
                    ? "bg-ink-950 text-ink-100"
                    : "text-ink-500 hover:bg-ink-850 hover:text-ink-300"
                }`}
              >
                {tab.label}
                {active ? (
                  <span className="absolute inset-x-3 -bottom-px h-0.5 bg-accent" />
                ) : null}
              </button>
            );
          })}
        </div>
      </header>

      {mode === "create" ? (
        <div className="flex min-h-0 flex-1 flex-col overflow-y-auto p-3">
          <div className="space-y-3">
            <label className="block">
              <span className="mb-1 block text-[11px] font-medium text-ink-400">Project name</span>
              <input
                autoFocus={!mobile}
                value={createName}
                disabled={creating}
                onChange={(e) => setCreateName(e.target.value)}
                onKeyDown={(e) => { if (e.key === "Enter") void createProject(); }}
                placeholder="my-project"
                aria-label="Project name"
                className="min-h-11 w-full rounded-sm border border-ink-700 bg-ink-950 px-2.5 py-1.5 text-[16px] text-ink-100 outline-none placeholder:text-ink-600 focus:border-accent focus-visible:ring-2 focus-visible:ring-accent disabled:opacity-50 sm:min-h-0 sm:text-[12px]"
              />
            </label>
            <label className="block">
              <span className="mb-1 block text-[11px] font-medium text-ink-400">Parent directory</span>
              <div className="flex gap-1.5">
                <input
                  value={createParent}
                  disabled={creating}
                  onChange={(e) => setCreateParent(e.target.value)}
                  placeholder={browseHome ?? "~"}
                  aria-label="Parent directory"
                  className="min-h-11 min-w-0 flex-1 rounded-sm border border-ink-700 bg-ink-950 px-2.5 py-1.5 font-mono text-[16px] text-ink-200 outline-none placeholder:text-ink-600 focus:border-accent focus-visible:ring-2 focus-visible:ring-accent disabled:opacity-50 sm:min-h-0 sm:text-[11px]"
                />
                <button type="button" disabled={creating} onClick={() => setMode("browse")} className="min-h-11 shrink-0 rounded-sm border border-ink-700 px-3 py-1.5 text-[11px] text-ink-300 hover:bg-ink-850 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-accent disabled:opacity-40 sm:min-h-0 sm:px-2">Browse</button>
              </div>
              {createParent.trim() || createName.trim() ? (
                <span className="mt-1 block truncate font-mono text-[10px] text-ink-600">
                  → {createParent.trim() || (browseHome ?? "~")}/{createName.trim() || "my-project"}
                </span>
              ) : null}
            </label>
            <div className="flex flex-col gap-1.5">
              <label className="flex min-h-11 items-center gap-2 text-[12px] text-ink-300 sm:min-h-0">
                <input type="checkbox" checked={createInitGit} disabled={creating} onChange={(e) => setCreateInitGit(e.target.checked)} className="h-5 w-5 accent-accent focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-accent" />
                Initialize a Git repository
              </label>
              <label className="flex min-h-11 items-center gap-2 text-[12px] text-ink-300 sm:min-h-0">
                <input type="checkbox" checked={createReadme} disabled={creating} onChange={(e) => setCreateReadme(e.target.checked)} className="h-5 w-5 accent-accent focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-accent" />
                Create a README.md
              </label>
            </div>
            {createError ? (
              <div role="alert" className="rounded-sm border border-danger bg-ink-950 px-2.5 py-2 text-[12px] text-danger">{createError}</div>
            ) : null}
          </div>
          <div className="mt-auto flex items-center gap-2 border-t border-ink-800 pt-3">
            <button type="button" disabled={creating || switching} onClick={() => void createProject()} className="flex min-h-11 flex-1 items-center justify-center gap-2 rounded-sm bg-accent px-3 py-2 text-[12px] font-semibold text-white hover:bg-accent-soft focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-accent disabled:opacity-40">
              <FolderPlusIcon width={14} height={14} />
              {creating ? "Creating…" : "Create project"}
            </button>
          </div>
        </div>
      ) : mode === "clone" ? (
        <div className="flex min-h-0 flex-1 flex-col overflow-y-auto p-3">
          <div className="space-y-3">
            <label className="block">
              <span className="mb-1 block text-[11px] font-medium text-ink-400">Repository URL</span>
              <input
                autoFocus={!mobile}
                value={cloneUrl}
                disabled={cloning}
                onChange={(e) => setCloneUrl(e.target.value)}
                placeholder="https://github.com/user/repo.git"
                aria-label="Repository URL"
                className="min-h-11 w-full rounded-sm border border-ink-700 bg-ink-950 px-2.5 py-1.5 font-mono text-[16px] text-ink-100 outline-none placeholder:text-ink-600 focus:border-accent focus-visible:ring-2 focus-visible:ring-accent disabled:opacity-50 sm:min-h-0 sm:text-[11px]"
              />
            </label>
            <label className="block">
              <span className="mb-1 block text-[11px] font-medium text-ink-400">Parent directory</span>
              <div className="flex gap-1.5">
                <input
                  value={cloneParent}
                  disabled={cloning}
                  onChange={(e) => setCloneParent(e.target.value)}
                  placeholder={browseHome ?? "~"}
                  aria-label="Parent directory"
                  className="min-h-11 min-w-0 flex-1 rounded-sm border border-ink-700 bg-ink-950 px-2.5 py-1.5 font-mono text-[16px] text-ink-200 outline-none placeholder:text-ink-600 focus:border-accent focus-visible:ring-2 focus-visible:ring-accent disabled:opacity-50 sm:min-h-0 sm:text-[11px]"
                />
                <button type="button" disabled={cloning} onClick={() => setMode("browse")} className="min-h-11 shrink-0 rounded-sm border border-ink-700 px-3 py-1.5 text-[11px] text-ink-300 hover:bg-ink-850 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-accent disabled:opacity-40 sm:min-h-0 sm:px-2">Browse</button>
              </div>
            </label>
            <div className="grid grid-cols-2 gap-2">
              <label className="block">
                <span className="mb-1 block text-[11px] font-medium text-ink-400">Folder name (optional)</span>
                <input
                  value={cloneName}
                  disabled={cloning}
                  onChange={(e) => setCloneName(e.target.value)}
                  placeholder="auto from URL"
                  aria-label="Folder name"
                  className="min-h-11 w-full rounded-sm border border-ink-700 bg-ink-950 px-2.5 py-1.5 text-[16px] text-ink-100 outline-none placeholder:text-ink-600 focus:border-accent focus-visible:ring-2 focus-visible:ring-accent disabled:opacity-50 sm:min-h-0 sm:text-[12px]"
                />
              </label>
              <label className="block">
                <span className="mb-1 block text-[11px] font-medium text-ink-400">Branch (optional)</span>
                <input
                  value={cloneBranch}
                  disabled={cloning}
                  onChange={(e) => setCloneBranch(e.target.value)}
                  placeholder="default"
                  aria-label="Branch"
                  className="min-h-11 w-full rounded-sm border border-ink-700 bg-ink-950 px-2.5 py-1.5 text-[16px] text-ink-100 outline-none placeholder:text-ink-600 focus:border-accent focus-visible:ring-2 focus-visible:ring-accent disabled:opacity-50 sm:min-h-0 sm:text-[12px]"
                />
              </label>
            </div>
            {cloneError ? (
              <div role="alert" className="rounded-sm border border-danger bg-ink-950 px-2.5 py-2 text-[12px] text-danger">{cloneError}</div>
            ) : null}
          </div>
          <div className="mt-auto flex items-center gap-2 border-t border-ink-800 pt-3">
            <button type="button" disabled={cloning || switching || !cloneUrl.trim()} onClick={() => void cloneRepo()} className="flex min-h-11 flex-1 items-center justify-center gap-2 rounded-sm bg-accent px-3 py-2 text-[12px] font-semibold text-white hover:bg-accent-soft focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-accent disabled:opacity-40">
              <GitBranchIcon width={14} height={14} />
              {cloning ? "Cloning…" : "Clone repository"}
            </button>
          </div>
        </div>
      ) : mode === "recent" ? (
        <>
          <div className="border-b border-ink-800 px-2.5 py-2">
            <label className="flex items-center gap-2 rounded-sm border border-ink-700 bg-ink-950 px-2.5 py-1.5">
              <SearchIcon width={13} height={13} className="shrink-0 text-ink-500" />
              <input
                autoFocus={!mobile}
                value={filter}
                onChange={(event) => setFilter(event.target.value)}
                placeholder="Filter recent projects…"
                aria-label="Filter recent projects"
                className="min-h-11 min-w-0 flex-1 bg-transparent text-[16px] text-ink-100 outline-none placeholder:text-ink-600 focus-visible:ring-2 focus-visible:ring-accent sm:min-h-0 sm:text-[12px]"
              />
              {filter ? (
                <button
                  type="button"
                  onClick={() => setFilter("")}
                  className="flex min-h-11 min-w-11 items-center justify-center rounded-sm p-0.5 text-ink-500 hover:text-ink-200 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-accent sm:min-h-0 sm:min-w-0"
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
                  className="mt-3 inline-flex min-h-11 items-center gap-1.5 rounded-sm border border-ink-700 px-3 py-1.5 text-[11px] font-medium text-ink-300 hover:bg-ink-850 hover:text-ink-100 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-accent sm:min-h-0"
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
                  className={`group/project flex items-center gap-1 rounded-sm px-1.5 py-1 ${
                    active ? "border-l-2 border-l-accent bg-ink-850" : "border-l-2 border-l-transparent hover:bg-ink-850"
                  }`}
                >
                  <button
                    type="button"
                    disabled={switching}
                    onClick={() => (active ? onClose() : switchTo(project.path))}
                    className="flex min-h-11 min-w-0 flex-1 items-center gap-2.5 rounded-sm px-1.5 py-1.5 text-left focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-accent disabled:opacity-50"
                  >
                    <span
                      className={`flex h-7 w-7 shrink-0 items-center justify-center rounded-sm ${
                        active ? "bg-ink-800 text-accent-soft" : "bg-ink-950 text-ink-500"
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
                    className="flex min-h-11 min-w-11 items-center justify-center rounded-sm p-1 text-ink-600 opacity-100 hover:bg-ink-800 hover:text-danger focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-danger sm:min-h-0 sm:min-w-0 sm:opacity-0 sm:group-hover/project:opacity-100 sm:focus:opacity-100"
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
              className="flex min-h-11 w-full items-center justify-center gap-2 rounded-sm border border-ink-700 bg-ink-950 px-3 py-2 text-[12px] font-medium text-ink-200 transition-colors hover:border-ink-600 hover:bg-ink-850 hover:text-ink-100 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-accent disabled:opacity-50"
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
                className="min-h-11 min-w-0 flex-1 rounded-sm border border-ink-700 bg-ink-950 px-2.5 py-1.5 font-mono text-[16px] text-ink-200 placeholder:text-ink-600 focus:border-accent focus:outline-none focus-visible:ring-2 focus-visible:ring-accent disabled:opacity-50 sm:min-h-0 sm:text-[11px]"
              />
              <button
                type="submit"
                disabled={switching || browseLoading}
                className="min-h-11 rounded-sm border border-ink-700 px-3 py-1.5 text-[11px] font-medium text-ink-300 hover:bg-ink-850 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-accent disabled:opacity-40 sm:min-h-0 sm:px-2.5"
              >
                Go
              </button>
            </form>

            <div className="flex items-center gap-1">
              <button
                type="button"
                disabled={!browseParent || browseLoading || switching}
                onClick={() => browseParent && void loadBrowse(browseParent)}
                className="flex min-h-11 min-w-11 items-center justify-center rounded-sm p-1.5 text-ink-400 hover:bg-ink-850 hover:text-ink-100 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-accent disabled:opacity-30 sm:min-h-0 sm:min-w-0"
                title="Go up"
                aria-label="Go up one directory"
              >
                <ChevronLeft width={14} height={14} />
              </button>
              <button
                type="button"
                disabled={!browseHome || browseLoading || switching}
                onClick={() => browseHome && void loadBrowse(browseHome)}
                className="flex min-h-11 min-w-11 items-center justify-center rounded-sm p-1.5 text-ink-400 hover:bg-ink-850 hover:text-ink-100 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-accent disabled:opacity-30 sm:min-h-0 sm:min-w-0"
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
                    className={`flex min-h-11 w-full items-center gap-2.5 rounded-sm px-2.5 py-2 text-left transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-accent disabled:opacity-50 ${
                      active ? "border-l-2 border-l-accent bg-ink-850" : "border-l-2 border-l-transparent hover:bg-ink-850"
                    }`}
                  >
                    <span className="flex h-7 w-7 shrink-0 items-center justify-center rounded-sm bg-ink-950 text-ink-400">
                      <FolderIcon width={14} height={14} />
                    </span>
                    <span className="min-w-0 flex-1 truncate text-[12px] font-medium text-ink-100">
                      {entry.name}
                    </span>
                    {already ? (
                      <span className="shrink-0 rounded-sm bg-ink-800 px-1.5 py-0.5 text-[9px] uppercase tracking-wide text-ink-400">
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
              className="flex min-h-11 min-w-0 flex-1 items-center justify-center gap-2 rounded-sm bg-accent px-3 py-2 text-[12px] font-semibold text-white transition-colors hover:bg-accent-soft focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-accent disabled:opacity-40"
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
      className={`min-h-11 max-w-[7rem] truncate rounded-sm px-2 py-0.5 text-[11px] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-accent sm:min-h-0 sm:px-1 ${
        active
          ? "font-medium text-ink-100"
          : "text-ink-400 hover:bg-ink-850 hover:text-ink-200"
      } disabled:opacity-40`}
    >
      {label}
    </button>
  );
}
