"use client";
// File explorer tree. Per docs/IDE_PANELS_CONTRACT.md §5.1.
// Lazy one-level expand: clicking a dir toggles ide.toggleDir(path) and fetches
// its immediate children from GET /api/tree (cached in component state). Clicking
// a file calls ide.openFile(path). The active file (== ide.state.activeTabId for
// file tabs) is highlighted. Header has refresh + new-file/folder + collapse-all.
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import type { KeyboardEvent as ReactKeyboardEvent } from "react";
import { useIdeContext } from "@/lib/ide-context";
import type { IdeApi } from "@/lib/use-ide";
import type { FileEntry, FileNode, GitStatusEntry } from "@/lib/types";
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
  CopyIcon,
  MinusIcon,
} from "@/components/icons";

type CreateKind = "file" | "folder";
type MenuState = { node: FileNode; x: number; y: number } | null;
type RenameState = { path: string; name: string } | null;

interface TreeCtx {
  workspace: string;
  ide: IdeApi;
  cache: Record<string, FileNode[]>;
  loading: Record<string, boolean>;
  load: (dir: string) => void;
  openMenu: (node: FileNode, x: number, y: number) => void;
  renaming: RenameState;
  setRenaming: (next: RenameState) => void;
  commitRename: (path: string, name: string) => void;
  gitByPath: Map<string, GitStatusEntry>;
  dirtyPaths: Set<string>;
  focusedPath: string | null;
  setFocusedPath: (path: string | null) => void;
}

function gitClass(status: GitStatusEntry["status"] | "dirty-dir" | undefined): string {
  switch (status) {
    case "modified":
    case "renamed":
      return "text-warning";
    case "added":
      return "text-emerald-400";
    case "deleted":
    case "conflicted":
      return "text-red-300";
    case "untracked":
      return "text-sky-400";
    case "dirty-dir":
      return "text-warning/80";
    default:
      return "";
  }
}

function fileIconClass(name: string): string {
  const ext = name.includes(".") ? name.slice(name.lastIndexOf(".") + 1).toLowerCase() : "";
  switch (ext) {
    case "ts":
    case "tsx":
      return "text-sky-400";
    case "js":
    case "jsx":
    case "mjs":
    case "cjs":
      return "text-amber-300";
    case "json":
    case "toml":
    case "yaml":
    case "yml":
      return "text-lime-400";
    case "md":
    case "mdx":
      return "text-ink-300";
    case "rs":
      return "text-orange-400";
    case "go":
      return "text-cyan-400";
    case "css":
    case "scss":
      return "text-pink-400";
    case "html":
      return "text-orange-300";
    case "py":
      return "text-yellow-300";
    default:
      return "text-ink-500";
  }
}

function nodeGitStatus(
  node: FileNode,
  gitByPath: Map<string, GitStatusEntry>,
): GitStatusEntry["status"] | "dirty-dir" | undefined {
  const direct = gitByPath.get(node.path);
  if (direct) return direct.status;
  if (!node.dir) return undefined;
  const prefix = node.path + "/";
  for (const key of gitByPath.keys()) {
    if (key.startsWith(prefix)) return "dirty-dir";
  }
  return undefined;
}

function TreeNode({ node, depth, ctx }: { node: FileNode; depth: number; ctx: TreeCtx }) {
  const { ide, cache, loading, load, renaming, focusedPath } = ctx;
  const expanded = node.dir && ide.isExpanded(node.path);
  const active = node.path === ide.state.activeTabId;
  const focused = focusedPath === node.path;
  const gitStatus = nodeGitStatus(node, ctx.gitByPath);
  const dirty = !node.dir && ctx.dirtyPaths.has(node.path);
  const isRenaming = renaming?.path === node.path;

  const onClick = () => {
    ctx.setFocusedPath(node.path);
    if (node.dir) {
      ide.toggleDir(node.path);
      if (!expanded && !cache[node.path]) load(node.path);
    } else {
      ide.openFile(node.path);
    }
  };

  return (
    <div className="group/tree-node" role="treeitem" aria-expanded={node.dir ? !!expanded : undefined} aria-selected={active || focused}>
      <div className="flex items-center">
        {isRenaming ? (
          <div className="flex min-w-0 flex-1 items-center gap-1 py-0.5 pr-1.5" style={{ paddingLeft: depth * 12 + 6 }}>
            {node.dir ? <FolderIcon width={14} height={14} className="shrink-0 text-ink-400" /> : <FileIcon width={14} height={14} className={`shrink-0 ${fileIconClass(node.name)}`} />}
            <input
              autoFocus
              defaultValue={renaming.name}
              aria-label={`Rename ${node.name}`}
              className="min-w-0 flex-1 rounded bg-ink-900 px-1.5 py-0.5 text-[13px] text-ink-100 outline-none ring-1 ring-ink-700 focus:ring-accent/50"
              onBlur={(event) => ctx.commitRename(node.path, event.currentTarget.value)}
              onKeyDown={(event) => {
                if (event.key === "Enter") {
                  event.preventDefault();
                  ctx.commitRename(node.path, event.currentTarget.value);
                }
                if (event.key === "Escape") {
                  event.preventDefault();
                  ctx.setRenaming(null);
                }
              }}
              onClick={(event) => event.stopPropagation()}
            />
          </div>
        ) : (
          <button
            type="button"
            data-tree-path={node.path}
            onClick={onClick}
            title={node.path}
            style={{ paddingLeft: depth * 12 + 6 }}
            onContextMenu={(event) => {
              event.preventDefault();
              ctx.setFocusedPath(node.path);
              ctx.openMenu(node, event.clientX, event.clientY);
            }}
            className={`flex min-w-0 flex-1 items-center gap-1 rounded py-1.5 pr-1.5 text-left text-[13px] sm:py-0.5 ${
              active ? "bg-ink-800 text-ink-100" : focused ? "bg-ink-800/40 text-ink-100" : "text-ink-300 hover:bg-ink-800/60"
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
              <FolderIcon width={14} height={14} className={`shrink-0 ${gitClass(gitStatus) || "text-ink-400"}`} />
            ) : (
              <FileIcon width={14} height={14} className={`shrink-0 ${gitClass(gitStatus) || fileIconClass(node.name)}`} />
            )}
            <span className={`truncate ${gitClass(gitStatus)}`}>{node.name}</span>
            {dirty ? <span className="ml-1 shrink-0 text-[10px] text-warning" title="Unsaved changes">●</span> : null}
            {node.symlink ? <span className="ml-1 shrink-0 text-ink-600">↳</span> : null}
          </button>
        )}
        <button
          type="button"
          onClick={(event) => {
            const box = event.currentTarget.getBoundingClientRect();
            ctx.setFocusedPath(node.path);
            ctx.openMenu(node, box.right, box.bottom);
          }}
          aria-label={`Actions for ${node.name}`}
          title="File actions"
          className="mr-1 rounded px-1 py-0.5 text-ink-600 opacity-0 hover:bg-ink-800 hover:text-ink-200 focus:opacity-100 group-hover/tree-node:opacity-100"
        >•••</button>
      </div>
      {expanded && node.dir ? (
        <div role="group">
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

function flattenVisible(
  nodes: FileNode[],
  cache: Record<string, FileNode[]>,
  isExpanded: (path: string) => boolean,
): FileNode[] {
  const out: FileNode[] = [];
  const walk = (list: FileNode[]) => {
    for (const node of list) {
      out.push(node);
      if (node.dir && isExpanded(node.path)) walk(cache[node.path] ?? []);
    }
  };
  walk(nodes);
  return out;
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
  const [renaming, setRenaming] = useState<RenameState>(null);
  const [error, setError] = useState<string | null>(null);
  const [focusedPath, setFocusedPath] = useState<string | null>(null);
  const menuRef = useRef<HTMLDivElement>(null);
  const treeRef = useRef<HTMLDivElement>(null);
  const workspaceRef = useRef(workspace);
  const expandedDirsRef = useRef(ide.state.expandedDirs);
  expandedDirsRef.current = ide.state.expandedDirs;

  useEffect(() => {
    workspaceRef.current = workspace;
    setCache({});
    setLoading({});
    setQuery("");
    setSearchResults([]);
    setMenu(null);
    setRenaming(null);
    setError(null);
    setFocusedPath(null);
  }, [workspace]);

  const load = useCallback(
    (dir: string) => {
      const requestWorkspace = workspace;
      setLoading((s) => ({ ...s, [dir]: true }));
      fetch(`/api/tree?path=${encodeURIComponent(dir)}&workspace=${encodeURIComponent(workspace)}`)
        .then(async (r) => {
          if (!r.ok) {
            const data = (await r.json().catch(() => ({}))) as { error?: string };
            throw new Error(data.error ?? `Could not load folder (${r.status})`);
          }
          return r.json() as Promise<{ nodes?: FileNode[] }>;
        })
        .then((d) => {
          if (workspaceRef.current === requestWorkspace) {
            setCache((c) => ({ ...c, [dir]: d.nodes ?? [] }));
            setError(null);
          }
        })
        .catch((reason: unknown) => {
          if (workspaceRef.current !== requestWorkspace) return;
          setCache((c) => ({ ...c, [dir]: c[dir] ?? [] }));
          setError(reason instanceof Error ? reason.message : "Could not load folder");
        })
        .finally(() => {
          if (workspaceRef.current === requestWorkspace) setLoading((s) => ({ ...s, [dir]: false }));
        });
    },
    [workspace],
  );

  // Load root + any persisted expanded dirs on mount / workspace change.
  useEffect(() => {
    load(base);
    for (const path of expandedDirsRef.current) {
      if (path !== base) load(path);
    }
  }, [load, base]);

  // Soft-load git status so tree decorations work without opening Source Control.
  useEffect(() => {
    let cancelled = false;
    void fetch(`/api/git?workspace=${encodeURIComponent(workspace)}`)
      .then((r) => (r.ok ? r.json() : null))
      .then((data) => {
        if (!cancelled && data && typeof data === "object" && "entries" in data) {
          ide.setGitStatus(data as import("@/lib/types").GitStatus);
        }
      })
      .catch(() => { /* decorations optional */ });
    return () => { cancelled = true; };
  }, [workspace]); // eslint-disable-line react-hooks/exhaustive-deps -- load once per workspace

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
    setError(null);
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

  // Reveal the active editor file in the tree (expand parents + scroll into view).
  useEffect(() => {
    const tab = ide.state.openTabs.find((item) => item.id === ide.state.activeTabId);
    if (!tab || tab.kind !== "file") return;
    const parts = tab.target.split("/").filter(Boolean);
    let acc = "";
    for (let i = 0; i < parts.length - 1; i++) {
      acc = acc ? `${acc}/${parts[i]}` : parts[i];
      ide.expandDir(acc);
      load(acc);
    }
    setFocusedPath(tab.target);
    const handle = window.setTimeout(() => {
      const el = treeRef.current?.querySelector(`[data-tree-path="${CSS.escape(tab.target)}"]`);
      el?.scrollIntoView({ block: "nearest" });
    }, 80);
    return () => window.clearTimeout(handle);
  }, [ide.state.activeTabId]); // eslint-disable-line react-hooks/exhaustive-deps -- reveal on active tab change only

  const beginCreate = useCallback((kind: CreateKind, dir = base) => {
    setCreateKind(kind);
    setCreateDir(dir);
    setNewName("");
    setCreating(true);
    setMenu(null);
    setError(null);
    if (dir) {
      ide.expandDir(dir);
      load(dir);
    }
  }, [base, ide, load]);

  const createEntry = useCallback(async () => {
    const name = newName.trim();
    if (!name) {
      setCreating(false);
      return;
    }
    if (name.includes("/") || name.includes("\\")) {
      setError("Name cannot contain path separators");
      return;
    }
    const path = createDir ? `${createDir}/${name}` : name;
    const apiKind = createKind === "folder" ? "dir" : "file";
    try {
      const r = await fetch("/api/file", {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ path, workspace, kind: apiKind }),
      });
      if (r.ok) {
        setNewName("");
        setCreating(false);
        if (createDir) ide.expandDir(createDir);
        load(createDir);
        if (createDir !== base) load(base);
        if (createKind === "file") ide.openFile(path);
        else {
          ide.expandDir(path);
          load(path);
        }
      } else {
        const data = (await r.json().catch(() => ({}))) as { error?: string };
        setError(data.error ?? `Could not create ${createKind}`);
      }
    } catch {
      setError(`Could not create ${createKind}`);
    }
  }, [newName, createDir, createKind, workspace, load, base, ide]);

  const commitRename = useCallback(async (oldPath: string, rawName: string) => {
    const name = rawName.trim();
    const current = oldPath.includes("/") ? oldPath.slice(oldPath.lastIndexOf("/") + 1) : oldPath;
    setRenaming(null);
    if (!name || name === current || name.includes("/") || name.includes("\\")) return;

    const affectedTabs = ide.state.openTabs.filter((tab) =>
      tab.target === oldPath || tab.target.startsWith(oldPath + "/"),
    );
    if (affectedTabs.some((tab) => tab.dirty)) {
      setError("Save or close modified files before renaming this entry.");
      return;
    }

    const parent = oldPath.includes("/") ? oldPath.slice(0, oldPath.lastIndexOf("/")) : "";
    const newPath = parent ? `${parent}/${name}` : name;
    try {
      const response = await fetch("/api/file", {
        method: "PATCH",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ path: oldPath, newPath, workspace }),
      });
      if (response.ok) {
        ide.remapFileTabs(oldPath, newPath);
        ide.remapExpandedDirs(oldPath, newPath);
        setCache((c) => {
          const next: Record<string, FileNode[]> = {};
          for (const [dir, nodes] of Object.entries(c)) {
            let key = dir;
            if (dir === oldPath) key = newPath;
            else if (dir.startsWith(oldPath + "/")) key = newPath + dir.slice(oldPath.length);
            next[key] = nodes.map((node) => {
              if (node.path === oldPath) return { ...node, path: newPath, name };
              if (node.path.startsWith(oldPath + "/")) {
                const childPath = newPath + node.path.slice(oldPath.length);
                return { ...node, path: childPath, name: childPath.includes("/") ? childPath.slice(childPath.lastIndexOf("/") + 1) : childPath };
              }
              return node;
            });
          }
          return next;
        });
        load(parent);
        setFocusedPath(newPath);
      } else {
        const data = (await response.json().catch(() => ({}))) as { error?: string };
        setError(data.error ?? "Rename failed");
      }
    } catch {
      setError("Rename failed");
    }
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
      ide.pruneExpandedDirs(node.path);
      setCache((c) => {
        const next = { ...c };
        delete next[node.path];
        for (const key of Object.keys(next)) {
          if (key.startsWith(node.path + "/")) delete next[key];
        }
        if (next[parent]) next[parent] = next[parent].filter((n) => n.path !== node.path);
        return next;
      });
      load(parent);
    } else {
      const data = (await response.json().catch(() => ({}))) as { error?: string };
      setError(data.error ?? "Delete failed");
    }
  }, [ide, load, workspace]);

  const copyPath = useCallback(async (path: string, absolute: boolean) => {
    const text = absolute ? `${workspace.replace(/\\/g, "/").replace(/\/$/, "")}/${path}` : path;
    try {
      await navigator.clipboard.writeText(text);
      setMenu(null);
    } catch {
      setError("Could not copy path to clipboard");
      setMenu(null);
    }
  }, [workspace]);

  const gitByPath = useMemo(() => {
    const map = new Map<string, GitStatusEntry>();
    for (const entry of ide.state.gitStatus?.entries ?? []) {
      map.set(entry.path, entry);
      if (entry.oldPath) map.set(entry.oldPath, entry);
    }
    return map;
  }, [ide.state.gitStatus]);

  const dirtyPaths = useMemo(() => {
    const set = new Set<string>();
    for (const tab of ide.state.openTabs) {
      if (tab.kind === "file" && tab.dirty) set.add(tab.target);
    }
    return set;
  }, [ide.state.openTabs]);

  const rootNodes = cache[base] ?? [];
  const visible = useMemo(
    () => flattenVisible(rootNodes, cache, ide.isExpanded),
    [rootNodes, cache, ide.isExpanded, ide.state.expandedDirs],
  );

  const onTreeKeyDown = useCallback((event: ReactKeyboardEvent<HTMLDivElement>) => {
    if (query.trim() || creating || renaming) return;
    const current = focusedPath ?? ide.state.activeTabId;
    const idx = current ? visible.findIndex((n) => n.path === current) : -1;
    const node = idx >= 0 ? visible[idx] : null;

    if (event.key === "ArrowDown") {
      event.preventDefault();
      const next = visible[Math.min(visible.length - 1, Math.max(0, idx + 1))];
      if (next) setFocusedPath(next.path);
      return;
    }
    if (event.key === "ArrowUp") {
      event.preventDefault();
      const next = visible[Math.max(0, idx <= 0 ? 0 : idx - 1)];
      if (next) setFocusedPath(next.path);
      return;
    }
    if (event.key === "Home") {
      event.preventDefault();
      if (visible[0]) setFocusedPath(visible[0].path);
      return;
    }
    if (event.key === "End") {
      event.preventDefault();
      if (visible.length) setFocusedPath(visible[visible.length - 1].path);
      return;
    }
    if (!node) return;

    if (event.key === "ArrowRight") {
      event.preventDefault();
      if (node.dir) {
        if (!ide.isExpanded(node.path)) {
          ide.expandDir(node.path);
          load(node.path);
        } else {
          const child = (cache[node.path] ?? [])[0];
          if (child) setFocusedPath(child.path);
        }
      }
      return;
    }
    if (event.key === "ArrowLeft") {
      event.preventDefault();
      if (node.dir && ide.isExpanded(node.path)) {
        ide.toggleDir(node.path);
      } else {
        const parent = node.path.includes("/") ? node.path.slice(0, node.path.lastIndexOf("/")) : "";
        if (parent) setFocusedPath(parent);
      }
      return;
    }
    if (event.key === "Enter") {
      event.preventDefault();
      if (node.dir) {
        ide.toggleDir(node.path);
        if (!ide.isExpanded(node.path)) load(node.path);
      } else {
        ide.openFile(node.path);
      }
      return;
    }
    if (event.key === "F2") {
      event.preventDefault();
      setRenaming({ path: node.path, name: node.name });
      setMenu(null);
      return;
    }
    if (event.key === "Delete" || event.key === "Backspace") {
      if (event.metaKey || event.ctrlKey || event.key === "Delete") {
        event.preventDefault();
        void deleteEntry(node);
      }
    }
  }, [query, creating, renaming, focusedPath, ide, visible, cache, load, deleteEntry]);

  const ctx: TreeCtx = {
    workspace,
    ide,
    cache,
    loading,
    load,
    openMenu: (node, x, y) => setMenu({ node, x, y }),
    renaming,
    setRenaming,
    commitRename: (path, name) => { void commitRename(path, name); },
    gitByPath,
    dirtyPaths,
    focusedPath,
    setFocusedPath,
  };

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
            onClick={() => ide.collapseAllDirs()}
            title="Collapse all"
            aria-label="Collapse all"
            className="rounded p-1 text-ink-400 hover:bg-ink-800 hover:text-ink-100"
          >
            <MinusIcon width={14} height={14} />
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
          {createDir ? <div className="mt-1 truncate px-0.5 text-[10px] text-ink-600">in {createDir}/</div> : null}
        </div>
      ) : null}

      {error ? <div role="alert" className="mx-2 mb-1 rounded bg-red-950/60 px-2 py-1 text-[11px] text-red-300">{error} <button type="button" onClick={() => setError(null)} className="float-right" aria-label="Dismiss error">×</button></div> : null}

      <div
        ref={treeRef}
        role="tree"
        aria-label="Workspace files"
        tabIndex={0}
        onKeyDown={onTreeKeyDown}
        className="min-h-0 flex-1 overflow-y-auto pb-2 outline-none focus-visible:ring-1 focus-visible:ring-inset focus-visible:ring-accent/40"
      >
        {query.trim() ? searchResults.map((node) => (
          <button key={node.path} type="button" title={node.path} onClick={() => ide.openFile(node.path)} className="flex w-full items-center gap-2 px-3 py-1.5 text-left text-[12px] text-ink-300 hover:bg-ink-800/60 hover:text-ink-100">
            <FileIcon width={13} height={13} className={`shrink-0 ${fileIconClass(node.path)}`} /><span className="truncate">{node.path}</span>
          </button>
        )) : rootNodes.map((node) => (
          <TreeNode key={node.path} node={node} depth={0} ctx={ctx} />
        ))}
        {query.trim() && searching ? <div className="px-3 py-2 text-xs text-ink-600">Searching…</div> : null}
        {query.trim() && !searching && searchResults.length === 0 ? <div className="px-3 py-2 text-xs text-ink-600">No matching files</div> : null}
        {!query.trim() && loading[base] && rootNodes.length === 0 ? (
          <div className="px-3 py-1 text-xs text-ink-600">Loading…</div>
        ) : null}
        {!query.trim() && !loading[base] && rootNodes.length === 0 && !error ? (
          <div className="px-3 py-3 text-xs leading-relaxed text-ink-600">Empty workspace. Create a file or folder above to get started.</div>
        ) : null}
      </div>
      {menu && (
        <div ref={menuRef} role="menu" aria-label={`Actions for ${menu.node.name}`} style={{ position: "fixed", left: Math.min(menu.x, window.innerWidth - 180), top: Math.min(menu.y, window.innerHeight - 220) }} className="z-50 w-48 rounded-lg border border-ink-700 bg-ink-900 p-1 shadow-2xl shadow-black/40">
          {!menu.node.dir && <button role="menuitem" onClick={() => { ide.openFile(menu.node.path); setMenu(null); }} className="w-full rounded px-2 py-1.5 text-left text-[12px] text-ink-200 hover:bg-ink-800">Open</button>}
          <button role="menuitem" onClick={() => beginCreate("file", menu.node.dir ? menu.node.path : menu.node.path.slice(0, Math.max(0, menu.node.path.lastIndexOf("/"))))} className="flex w-full items-center gap-2 rounded px-2 py-1.5 text-left text-[12px] text-ink-200 hover:bg-ink-800"><PlusIcon width={12} height={12} />New file here</button>
          <button role="menuitem" onClick={() => beginCreate("folder", menu.node.dir ? menu.node.path : menu.node.path.slice(0, Math.max(0, menu.node.path.lastIndexOf("/"))))} className="flex w-full items-center gap-2 rounded px-2 py-1.5 text-left text-[12px] text-ink-200 hover:bg-ink-800"><FolderPlusIcon width={12} height={12} />New folder here</button>
          <button role="menuitem" onClick={() => { setRenaming({ path: menu.node.path, name: menu.node.name }); setMenu(null); }} className="flex w-full items-center gap-2 rounded px-2 py-1.5 text-left text-[12px] text-ink-200 hover:bg-ink-800"><EditIcon width={12} height={12} />Rename</button>
          <button role="menuitem" onClick={() => void copyPath(menu.node.path, false)} className="flex w-full items-center gap-2 rounded px-2 py-1.5 text-left text-[12px] text-ink-200 hover:bg-ink-800"><CopyIcon width={12} height={12} />Copy path</button>
          <button role="menuitem" onClick={() => void copyPath(menu.node.path, true)} className="w-full rounded px-2 py-1.5 text-left text-[12px] text-ink-200 hover:bg-ink-800">Copy absolute path</button>
          <button role="menuitem" onClick={() => void deleteEntry(menu.node)} className="flex w-full items-center gap-2 rounded px-2 py-1.5 text-left text-[12px] text-red-300 hover:bg-red-950/60"><TrashIcon width={12} height={12} />Delete</button>
        </div>
      )}
    </div>
  );
}
