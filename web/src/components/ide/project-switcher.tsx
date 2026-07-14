"use client";

import { useEffect, useState } from "react";
import type { ProjectEntry } from "@/lib/types";
import { basename } from "@/lib/format";
import { useOutsideClose } from "@/lib/use-outside-close";
import {
  CheckIcon,
  FolderIcon,
  FolderPlusIcon,
  XIcon,
} from "@/components/icons";

interface ProjectSwitcherProps {
  workspace: string;
  projects: ProjectEntry[];
  switching: boolean;
  mobile?: boolean;
  onSwitchWorkspace: (path: string) => void;
  onRemoveProject: (path: string) => void;
  onClose: () => void;
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
  const [newProjectPath, setNewProjectPath] = useState("");
  const ref = useOutsideClose(onClose);
  const current = projects.find((project) => project.path === workspace);

  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape") onClose();
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [onClose]);

  const switchTo = (path: string) => {
    const next = path.trim();
    if (!next) return;
    onSwitchWorkspace(next);
    setNewProjectPath("");
    onClose();
  };

  return (
    <section
      ref={ref}
      role="dialog"
      aria-label="Switch project"
      className={`fixed z-[70] flex max-h-[min(32rem,calc(100dvh-5rem))] flex-col overflow-hidden rounded-xl border border-ink-700 bg-ink-925 shadow-2xl shadow-black/50 animate-fade-in ${
        mobile
          ? "left-2 right-2 top-[calc(env(safe-area-inset-top)+3rem)]"
          : "bottom-3 left-14 w-[22rem]"
      }`}
    >
      <header className="flex items-center gap-2 border-b border-ink-800 px-3 py-2.5">
        <FolderIcon width={15} height={15} className="text-accent-soft" />
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
          className="rounded p-1 text-ink-500 hover:bg-ink-800 hover:text-ink-100"
          aria-label="Close project switcher"
        >
          <XIcon width={14} height={14} />
        </button>
      </header>

      <div className="min-h-0 flex-1 overflow-y-auto p-1.5">
        {projects.length === 0 && (
          <div className="px-3 py-5 text-center text-[12px] text-ink-500">
            No recent projects yet.
          </div>
        )}
        {projects.map((project) => {
          const active = project.path === workspace;
          return (
            <div
              key={project.path}
              className="group/project flex items-center gap-1 rounded-lg px-2 py-1.5 hover:bg-ink-850"
            >
              <button
                type="button"
                disabled={switching}
                onClick={() => active ? onClose() : switchTo(project.path)}
                className="flex min-w-0 flex-1 items-center gap-2 text-left disabled:opacity-50"
              >
                <FolderIcon
                  width={14}
                  height={14}
                  className={active ? "shrink-0 text-accent-soft" : "shrink-0 text-ink-500"}
                />
                <span className="min-w-0 flex-1">
                  <span className="block truncate text-[12px] font-medium text-ink-100">
                    {project.name}
                  </span>
                  <span className="block truncate font-mono text-[10px] text-ink-500">
                    {project.path}
                  </span>
                </span>
                {active && <CheckIcon width={13} height={13} className="shrink-0 text-accent-soft" />}
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

      <div className="flex items-center gap-1.5 border-t border-ink-800 p-2">
        <input
          autoFocus
          value={newProjectPath}
          disabled={switching}
          onChange={(event) => setNewProjectPath(event.target.value)}
          onKeyDown={(event) => {
            if (event.key === "Enter") switchTo(newProjectPath);
          }}
          placeholder="/path/to/project"
          aria-label="Project path"
          className="min-w-0 flex-1 rounded-lg border border-ink-700 bg-ink-950 px-2.5 py-1.5 font-mono text-[11px] text-ink-200 placeholder:text-ink-600 focus:border-accent/50 focus:outline-none disabled:opacity-50"
        />
        <button
          type="button"
          disabled={switching || !newProjectPath.trim()}
          onClick={() => switchTo(newProjectPath)}
          className="flex h-8 w-8 shrink-0 items-center justify-center rounded-lg bg-accent text-white disabled:opacity-40"
          title="Add and switch to project"
          aria-label="Add and switch to project"
        >
          <FolderPlusIcon width={14} height={14} />
        </button>
      </div>
    </section>
  );
}
