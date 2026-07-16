"use client";
// File explorer tree. Per docs/IDE_PANELS_CONTRACT.md §5.1.
//   export function FileTree({ root }: { root?: string })
// Lazy one-level expand: clicking a dir toggles ide.toggleDir(path) and fetches
// its immediate children from GET /api/tree (cached in component state). Clicking
// a file calls ide.openFile(path). The active file (== ide.state.activeTabId for
// file tabs) is highlighted. Header has a refresh + new-file affordance.
import { useCallback, useEffect, useRef, useState } from "react";
import { useIdeContext } from "@/lib/ide-context";
import type { IdeApi } from "@/lib/use-ide";
import type { FileEntry, FileNode } from "@/lib/types";
import {
  ChevronDown,
  ChevronRight,
  FolderIcon,
  FileIcon,
  RefreshIcon,
  PlusIcon,
  FolderPlusIcon,
  SearchIcon,
  TrashIcon,
  EditIcon,
} from "@/components/icons";

type CreateKind = "file" | "folder";
type MenuState = { node: FileNode; x: number; y: number } | null;

interface TreeCtx {
  workspace: string;
  ide: IdeApi;
  cache: Record<string, FileNode[]>;
  loading: Record<string, boolean>;
  load: (dir: string) => void;
  openMenu: (node: FileNode, x: number, y: number) => void;
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
    <div className="group/tree-node">
      <div className="flex items-center">
      <button
        type="button"
        onClick={onClick}
        title={node.path}
        style={{ paddingLeft: depth * 12 + 6 }}
        onContextMenu={(event) => { event.preventDefault(); ctx.openMenu(node, event.clientX, event.clientY); }}
        className={`flex min-w-0 flex-1 items-center gap-1 rounded py-1.5 pr-1.5 text-left text-[13px] sm:py-0.5 ${
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
      <button
        type="button"
        onClick={(event) => { const box = event.currentTarget.getBoundingClientRect(); ctx.openMenu(node, box.right, box.bottom); }}
        aria-label={`Actions for ${node.name}`}
        title="File actions"
        className="mr-1 rounded px-1 py-0.5 text-ink-600 opacity-0 hover:bg-ink-800 hover:text-ink-200 focus:opacity-100 group-hover/tree-node:opacity-100"
      >•••</button>
      </div>
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

export function FileTree({ root, refreshToken }: { root?: string; refreshToken?: number }) {
  const { workspace, ide } = useIdeContext();
  const base = root ?? "";
  const [cache, setCache] = useState<Record<string, FileNode[]>>({});
  const [loading, setLoading] = useState<Record<string, boolean>>({});
  const [creating, setCreating] = useState(false);
  const [createKind, setCreateKind] = useState<CreateKind>("file");
  const [createDir, setCreateDir] = useState(base);
  const [newName, setNewName] = useState("");
  const [query, setQuery] = useState("");
  const [searchResults, setSearchResults] = useState<FileEntry[]>([]);
  const [searching, setSearching] = useState(false);
  const [menu, setMenu] = useState<MenuState>(null);
  const [error, setError] = useState<string | null>(null);
  const menuRef = useRef<HTMLDivElement>(null);
  const workspaceRef = useRef(workspace);

  useEffect(() => {
    workspaceRef.current = workspace;
    setCache({});
    setLoading({});
    setQuery("");
    setSearchResults([]);
    setMenu(null);
    setError(null);
  }, [workspace]);

  const load = useCallback(
    (dir: string) => {
      const requestWorkspace = workspace;
      setLoading((s) => ({ ...s, [dir]: true }));
      fetch(`/api/tree?path=${encodeURIComponent(dir)}&workspace=${encodeURIComponent(workspace)}`)
        .then((r) => (r.ok ? r.json() : { nodes: [] }))
        .then((d: { nodes?: FileNode[] }) => {
          if (workspaceRef.current === requestWorkspace) setCache((c) => ({ ...c, [dir]: d.nodes ?? [] }));
        })
        .catch(() => {
          if (workspaceRef.current === requestWorkspace) setCache((c) => ({ ...c, [dir]: [] }));
        })
        .finally(() => {
          if (workspaceRef.current === requestWorkspace) setLoading((s) => ({ ...s, [dir]: false }));
        });
    },
    [workspace],
  );

  // Load the root level on mount and whenever the workspace changes.
  useEffect(() => {
    load(base);
  }, [load, base]);

  useEffect(() => {
    const q = query.trim();
    if (!q) { setSearchResults([]); setSearching(false); return; }
    const controller = new AbortController();
    const timer = setTimeout(() => {
      setSearching(true);
      fetch(`/api/files?q=${encodeURIComponent(q)}&workspace=${encodeURIComponent(workspace)}&path=${encodeURIComponent(base)}`, { signal: controller.signal })
        .then((response) => response.ok ? response.json() : { files: [] })
        .then((data: { files?: FileEntry[] }) => setSearchResults(data.files ?? []))
        .catch(() => { if (!controller.signal.aborted) setSearchResults([]); })
        .finally(() => { if (!controller.signal.aborted) setSearching(false); });
    }, 180);
    return () => { clearTimeout(timer); controller.abort(); };
  }, [base, query, workspace]);

  useEffect(() => {
    if (!menu) return;
    requestAnimationFrame(() => menuRef.current?.querySelector<HTMLButtonElement>('[role="menuitem"]')?.focus());
    const close = (event: MouseEvent) => {
      if (!menuRef.current?.contains(event.target as Node)) setMenu(null);
    };
    const onKey = (event: KeyboardEvent) => {
      if (event.key === "Escape") setMenu(null);
      if (event.key !== "ArrowDown" && event.key !== "ArrowUp" && event.key !== "Home" && event.key !== "End") return;
      const items = Array.from(menuRef.current?.querySelectorAll<HTMLButtonElement>('[role="menuitem"]') ?? []);
      if (!items.length) return;
      event.preventDefault();
      const current = items.indexOf(document.activeElement as HTMLButtonElement);
      const next = event.key === "Home" ? 0
        : event.key === "End" ? items.length - 1
          : event.key === "ArrowDown" ? (current + 1 + items.length) % items.length
            : (current - 1 + items.length) % items.length;
      items[next]?.focus();
    };
    document.addEventListener("mousedown", close);
    document.addEventListener("keydown", onKey);
    return () => { document.removeEventListener("mousedown", close); document.removeEventListener("keydown", onKey); };
  }, [menu]);

  const refresh = useCallback(() => {
    setCache({});
    load(base);
    for (const p of ide.state.expandedDirs) {
      if (p !== base) load(p);
    }
  }, [load, base, ide.state.expandedDirs]);

  // Agent file_change / worktree promote bumps refreshToken so the explorer stays current.
  useEffect(() => {
    if (refreshToken == null || refreshToken <= 0) return;
    refresh();
  }, [refreshToken]); // eslint-disable-line react-hooks/exhaustive-deps -- intentional: only on seq bump

  const beginCreate = useCallback((kind: CreateKind, dir = base) => {
    setCreateKind(kind); setCreateDir(dir); setNewName(""); setCreating(true); setMenu(null); setError(null);
  }, [base]);

  const createEntry = useCallback(async () => {
    const name = newName.trim();
    if (!name) {
      setCreating(false);
      return;
    }
    const path = createDir ? `${createDir}/${name}` : name;
    try {
      const r = await fetch("/api/file", {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ path, workspace, kind: createKind }),
      });
      if (r.ok) {
        setNewName("");
        setCreating(false);
        load(createDir);
        if (createDir !== base) load(base);
        if (createKind === "file") ide.openFile(path);
      } else {
        const data = (await r.json().catch(() => ({}))) as { error?: string };
        setError(data.error ?? `Could not create ${createKind}`);
      }
    } catch {
      setError(`Could not create ${createKind}`);
    }
  }, [newName, createDir, createKind, workspace, load, base, ide]);

  const renameEntry = useCallback(async (node: FileNode) => {
    const affectedTabs = ide.state.openTabs.filter((tab) =>
      tab.target === node.path || (node.dir && tab.target.startsWith(node.path + "/")),
    );
    if (affectedTabs.some((tab) => tab.dirty)) {
      setMenu(null);
      setError("Save or close modified files before renaming this entry.");
      return;
    }
    const name = window.prompt(`Rename ${node.name}`, node.name)?.trim();
    if (!name || name === node.name || name.includes("/") || name.includes("\\")) return;
    const parent = node.path.includes("/") ? node.path.slice(0, node.path.lastIndexOf("/")) : "";
    const newPath = parent ? `${parent}/${name}` : name;
    const response = await fetch("/api/file", { method: "PATCH", headers: { "Content-Type": "application/json" }, body: JSON.stringify({ path: node.path, newPath, workspace }) });
    setMenu(null);
    if (response.ok) {
      for (const tab of affectedTabs) ide.closeTab(tab.id);
      load(parent);
      if (!node.dir) ide.openFile(newPath);
    }
    else { const data = (await response.json().catch(() => ({}))) as { error?: string }; setError(data.error ?? "Rename failed"); }
  }, [ide, load, workspace]);

  const deleteEntry = useCallback(async (node: FileNode) => {
    const affectedTabs = ide.state.openTabs.filter((tab) =>
      tab.target === node.path || (node.dir && tab.target.startsWith(node.path + "/")),
    );
    if (affectedTabs.some((tab) => tab.dirty)) {
      setMenu(null);
      setError("Save or close modified files before deleting this entry.");
      return;
    }
    if (!window.confirm(`Permanently delete ${node.dir ? "folder" : "file"} “${node.name}”${node.dir ? " and everything inside it" : ""}?`)) return;
    const response = await fetch(`/api/file?path=${encodeURIComponent(node.path)}&workspace=${encodeURIComponent(workspace)}`, { method: "DELETE" });
    setMenu(null);
    const parent = node.path.includes("/") ? node.path.slice(0, node.path.lastIndexOf("/")) : "";
    if (response.ok) {
      for (const tab of affectedTabs) ide.closeTab(tab.id);
      load(parent);
    }
    else { const data = (await response.json().catch(() => ({}))) as { error?: string }; setError(data.error ?? "Delete failed"); }
  }, [ide, load, workspace]);

  const ctx: TreeCtx = { workspace, ide, cache, loading, load, openMenu: (node, x, y) => setMenu({ node, x, y }) };
  const rootNodes = cache[base] ?? [];

  return (
    <div className="flex h-full flex-col">
      <div className="flex items-center justify-between px-3 py-2 text-[11px] font-semibold uppercase tracking-wide text-ink-400">
        <span>Explorer</span>
        <div className="flex items-center gap-0.5">
          <button
            type="button"
            onClick={() => beginCreate("file")}
            title="New file"
            aria-label="New file"
            className="rounded p-1 text-ink-400 hover:bg-ink-800 hover:text-ink-100"
          >
            <PlusIcon width={14} height={14} />
          </button>
          <button type="button" onClick={() => beginCreate("folder")} title="New folder" aria-label="New folder" className="rounded p-1 text-ink-400 hover:bg-ink-800 hover:text-ink-100">
            <FolderPlusIcon width={14} height={14} />
          </button>
          <button
            type="button"
            onClick={refresh}
            title="Refresh"
            aria-label="Refresh"
            className="rounded p-1 text-ink-400 hover:bg-ink-800 hover:text-ink-100"
          >
            <RefreshIcon width={14} height={14} />
          </button>
        </div>
      </div>

      <div className="px-2 pb-2">
        <label className="flex items-center gap-1.5 rounded-md border border-ink-800 bg-ink-900/50 px-2 focus-within:border-ink-600">
          <SearchIcon width={12} height={12} className="shrink-0 text-ink-500" />
          <input value={query} onChange={(event) => setQuery(event.target.value)} placeholder="Filter files" aria-label="Filter workspace files" className="min-w-0 flex-1 bg-transparent py-1.5 text-[12px] text-ink-200 outline-none placeholder:text-ink-600" />
          {query && <button type="button" onClick={() => setQuery("")} aria-label="Clear filter" className="text-ink-500 hover:text-ink-200">×</button>}
        </label>
      </div>

      {creating ? (
        <div className="px-2 pb-1">
          <input
            autoFocus
            value={newName}
            onChange={(e) => setNewName(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === "Enter") void createEntry();
              if (e.key === "Escape") {
                setCreating(false);
                setNewName("");
              }
            }}
            placeholder={createKind === "folder" ? "folder name" : "filename.ext"}
            aria-label={`New ${createKind} name`}
            className="w-full rounded bg-ink-900 px-2 py-1 text-[13px] text-ink-100 outline-none ring-1 ring-ink-700 focus:ring-accent/50"
          />
        </div>
      ) : null}

      {error ? <div role="alert" className="mx-2 mb-1 rounded bg-red-950/60 px-2 py-1 text-[11px] text-red-300">{error} <button type="button" onClick={() => setError(null)} className="float-right" aria-label="Dismiss error">×</button></div> : null}

      <div className="min-h-0 flex-1 overflow-y-auto pb-2">
        {query.trim() ? searchResults.map((node) => (
          <button key={node.path} type="button" title={node.path} onClick={() => ide.openFile(node.path)} className="flex w-full items-center gap-2 px-3 py-1.5 text-left text-[12px] text-ink-300 hover:bg-ink-800/60 hover:text-ink-100">
            <FileIcon width={13} height={13} className="shrink-0 text-ink-500" /><span className="truncate">{node.path}</span>
          </button>
        )) : rootNodes.map((node) => (
          <TreeNode key={node.path} node={node} depth={0} ctx={ctx} />
        ))}
        {query.trim() && searching ? <div className="px-3 py-2 text-xs text-ink-600">Searching…</div> : null}
        {query.trim() && !searching && searchResults.length === 0 ? <div className="px-3 py-2 text-xs text-ink-600">No matching files</div> : null}
        {!query.trim() && loading[base] && rootNodes.length === 0 ? (
          <div className="px-3 py-1 text-xs text-ink-600">Loading…</div>
        ) : null}
        {!query.trim() && !loading[base] && rootNodes.length === 0 ? (
          <div className="px-3 py-3 text-xs leading-relaxed text-ink-600">Empty workspace. Create a file or folder above to get started.</div>
        ) : null}
      </div>
      {menu && (
        <div ref={menuRef} role="menu" aria-label={`Actions for ${menu.node.name}`} style={{ position: "fixed", left: Math.min(menu.x, window.innerWidth - 180), top: Math.min(menu.y, window.innerHeight - 170) }} className="z-50 w-44 rounded-lg border border-ink-700 bg-ink-900 p-1 shadow-2xl shadow-black/40">
          {!menu.node.dir && <button role="menuitem" onClick={() => { ide.openFile(menu.node.path); setMenu(null); }} className="w-full rounded px-2 py-1.5 text-left text-[12px] text-ink-200 hover:bg-ink-800">Open</button>}
          <button role="menuitem" onClick={() => beginCreate("file", menu.node.dir ? menu.node.path : menu.node.path.slice(0, Math.max(0, menu.node.path.lastIndexOf("/"))))} className="flex w-full items-center gap-2 rounded px-2 py-1.5 text-left text-[12px] text-ink-200 hover:bg-ink-800"><PlusIcon width={12} height={12} />New file here</button>
          <button role="menuitem" onClick={() => beginCreate("folder", menu.node.dir ? menu.node.path : menu.node.path.slice(0, Math.max(0, menu.node.path.lastIndexOf("/"))))} className="flex w-full items-center gap-2 rounded px-2 py-1.5 text-left text-[12px] text-ink-200 hover:bg-ink-800"><FolderPlusIcon width={12} height={12} />New folder here</button>
          <button role="menuitem" onClick={() => void renameEntry(menu.node)} className="flex w-full items-center gap-2 rounded px-2 py-1.5 text-left text-[12px] text-ink-200 hover:bg-ink-800"><EditIcon width={12} height={12} />Rename</button>
          <button role="menuitem" onClick={() => void deleteEntry(menu.node)} className="flex w-full items-center gap-2 rounded px-2 py-1.5 text-left text-[12px] text-red-300 hover:bg-red-950/60"><TrashIcon width={12} height={12} />Delete</button>
        </div>
      )}
    </div>
  );
}
