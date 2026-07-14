"use client";
// File explorer tree. Per docs/IDE_PANELS_CONTRACT.md §5.1.
//   export function FileTree({ root }: { root?: string })
// Lazy one-level expand: clicking a dir toggles ide.toggleDir(path) and fetches
// its immediate children from GET /api/tree (cached in component state). Clicking
// a file calls ide.openFile(path). The active file (== ide.state.activeTabId for
// file tabs) is highlighted. Header has a refresh + new-file affordance.
import { useCallback, useEffect, useState } from "react";
import { useIdeContext } from "@/lib/ide-context";
import type { IdeApi } from "@/lib/use-ide";
import type { FileNode } from "@/lib/types";
import {
  ChevronDown,
  ChevronRight,
  FolderIcon,
  FileIcon,
  RefreshIcon,
  PlusIcon,
} from "@/components/icons";

interface TreeCtx {
  workspace: string;
  ide: IdeApi;
  cache: Record<string, FileNode[]>;
  loading: Record<string, boolean>;
  load: (dir: string) => void;
}

function TreeNode({ node, depth, ctx }: { node: FileNode; depth: number; ctx: TreeCtx }) {
  const { ide, cache, loading, load } = ctx;
  const expanded = node.dir && ide.isExpanded(node.path);
  const active = node.path === ide.state.activeTabId;

  const onClick = () => {
    if (node.dir) {
      ide.toggleDir(node.path);
      if (!expanded && !cache[node.path]) load(node.path);
    } else {
      ide.openFile(node.path);
    }
  };

  return (
    <div>
      <button
        type="button"
        onClick={onClick}
        title={node.path}
        style={{ paddingLeft: depth * 12 + 6 }}
        className={`flex w-full items-center gap-1 rounded py-0.5 pr-1.5 text-left text-[13px] ${
          active ? "bg-ink-800 text-ink-100" : "text-ink-300 hover:bg-ink-800/60"
        }`}
      >
        {node.dir ? (
          expanded ? (
            <ChevronDown width={14} height={14} className="shrink-0 text-ink-500" />
          ) : (
            <ChevronRight width={14} height={14} className="shrink-0 text-ink-500" />
          )
        ) : (
          <span className="inline-block w-[14px] shrink-0" />
        )}
        {node.dir ? (
          <FolderIcon width={14} height={14} className="shrink-0 text-ink-400" />
        ) : (
          <FileIcon width={14} height={14} className="shrink-0 text-ink-500" />
        )}
        <span className="truncate">{node.name}</span>
        {node.symlink ? <span className="ml-1 shrink-0 text-ink-600">↳</span> : null}
      </button>
      {expanded && node.dir ? (
        <div>
          {loading[node.path] && !cache[node.path] ? (
            <div className="py-0.5 text-xs text-ink-600" style={{ paddingLeft: (depth + 1) * 12 + 6 }}>
              …
            </div>
          ) : null}
          {(cache[node.path] ?? []).map((child) => (
            <TreeNode key={child.path} node={child} depth={depth + 1} ctx={ctx} />
          ))}
        </div>
      ) : null}
    </div>
  );
}

export function FileTree({ root }: { root?: string }) {
  const { workspace, ide } = useIdeContext();
  const base = root ?? "";
  const [cache, setCache] = useState<Record<string, FileNode[]>>({});
  const [loading, setLoading] = useState<Record<string, boolean>>({});
  const [creating, setCreating] = useState(false);
  const [newName, setNewName] = useState("");

  const load = useCallback(
    (dir: string) => {
      setLoading((s) => ({ ...s, [dir]: true }));
      fetch(`/api/tree?path=${encodeURIComponent(dir)}&workspace=${encodeURIComponent(workspace)}`)
        .then((r) => (r.ok ? r.json() : { nodes: [] }))
        .then((d: { nodes?: FileNode[] }) => setCache((c) => ({ ...c, [dir]: d.nodes ?? [] })))
        .catch(() => setCache((c) => ({ ...c, [dir]: [] })))
        .finally(() => setLoading((s) => ({ ...s, [dir]: false })));
    },
    [workspace],
  );

  // Load the root level on mount and whenever the workspace changes.
  useEffect(() => {
    load(base);
  }, [load, base]);

  const refresh = useCallback(() => {
    setCache({});
    load(base);
    for (const p of ide.state.expandedDirs) {
      if (p !== base) load(p);
    }
  }, [load, base, ide.state.expandedDirs]);

  const createFile = useCallback(async () => {
    const name = newName.trim();
    if (!name) {
      setCreating(false);
      return;
    }
    const path = base ? `${base}/${name}` : name;
    try {
      const r = await fetch("/api/file", {
        method: "PUT",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ path, content: "", workspace }),
      });
      if (r.ok) {
        setNewName("");
        setCreating(false);
        load(base);
        ide.openFile(path);
      }
    } catch {
      /* ignore transient network errors */
    }
  }, [newName, base, workspace, load, ide]);

  const ctx: TreeCtx = { workspace, ide, cache, loading, load };
  const rootNodes = cache[base] ?? [];

  return (
    <div className="flex h-full flex-col">
      <div className="flex items-center justify-between px-3 py-2 text-[11px] font-semibold uppercase tracking-wide text-ink-400">
        <span>Explorer</span>
        <div className="flex items-center gap-0.5">
          <button
            type="button"
            onClick={() => setCreating(true)}
            title="New file"
            className="rounded p-1 text-ink-400 hover:bg-ink-800 hover:text-ink-100"
          >
            <PlusIcon width={14} height={14} />
          </button>
          <button
            type="button"
            onClick={refresh}
            title="Refresh"
            className="rounded p-1 text-ink-400 hover:bg-ink-800 hover:text-ink-100"
          >
            <RefreshIcon width={14} height={14} />
          </button>
        </div>
      </div>

      {creating ? (
        <div className="px-2 pb-1">
          <input
            autoFocus
            value={newName}
            onChange={(e) => setNewName(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === "Enter") void createFile();
              if (e.key === "Escape") {
                setCreating(false);
                setNewName("");
              }
            }}
            placeholder="filename.ext"
            className="w-full rounded bg-ink-900 px-2 py-1 text-[13px] text-ink-100 outline-none ring-1 ring-ink-700 focus:ring-ink-500"
          />
        </div>
      ) : null}

      <div className="min-h-0 flex-1 overflow-y-auto pb-2">
        {rootNodes.map((node) => (
          <TreeNode key={node.path} node={node} depth={0} ctx={ctx} />
        ))}
        {loading[base] && rootNodes.length === 0 ? (
          <div className="px-3 py-1 text-xs text-ink-600">Loading…</div>
        ) : null}
        {!loading[base] && rootNodes.length === 0 ? (
          <div className="px-3 py-1 text-xs text-ink-600">Empty workspace</div>
        ) : null}
      </div>
    </div>
  );
}
