// Small display/format helpers (no deps).

export function formatTokens(n: number | undefined | null): string {
  if (n == null) return "—";
  if (n < 1000) return String(n);
  if (n < 1_000_000) return `${(n / 1000).toFixed(n < 10_000 ? 1 : 0)}k`;
  return `${(n / 1_000_000).toFixed(2)}M`;
}

export function formatMs(ms: number | undefined | null): string {
  if (ms == null) return "—";
  if (ms < 1000) return `${Math.round(ms)}ms`;
  const s = ms / 1000;
  if (s < 60) return `${s.toFixed(s < 10 ? 1 : 0)}s`;
  const m = Math.floor(s / 60);
  const rem = Math.round(s % 60);
  return `${m}m ${rem}s`;
}

export function formatTps(tps: number | undefined | null): string {
  if (tps == null) return "—";
  return `${tps.toFixed(1)} t/s`;
}

export function relativeTime(ts: number): string {
  const diff = Date.now() - ts;
  if (diff < 60_000) return "just now";
  const mins = Math.floor(diff / 60_000);
  if (mins < 60) return `${mins}m ago`;
  const hrs = Math.floor(mins / 60);
  if (hrs < 24) return `${hrs}h ago`;
  const days = Math.floor(hrs / 24);
  if (days < 7) return `${days}d ago`;
  return new Date(ts).toLocaleDateString();
}

export function shortPath(p: string | null | undefined): string {
  if (!p) return "—";
  const parts = p.split("/");
  if (parts.length <= 2) return p;
  return ".../" + parts.slice(-2).join("/");
}

export function basename(p: string | null | undefined): string {
  if (!p) return "";
  const parts = p.replace(/\\/g, "/").split("/");
  return parts[parts.length - 1] || p;
}

/** Truncate a long string for one-line previews. */
export function truncate(s: string, n = 80): string {
  const flat = s.replace(/\s+/g, " ").trim();
  return flat.length > n ? flat.slice(0, n - 1) + "…" : flat;
}

// Mirrors the core's ToolKind::Destructive classification (core/src/tools.rs
// classify()). Read-only tools (read_file, grep, glob, …) are intentionally
// absent — only tools that write/execute get the destructive badge + amber
// approval styling.
const DANGEROUS_TOOLS = new Set([
  "bash",
  "write_file",
  "edit",
  "patch",
  "bulk_write",
  "bulk_edit",
  "todo_write",
  "spawn",
  "subagent",
]);

export function isDangerousTool(name: string): boolean {
  return DANGEROUS_TOOLS.has(name);
}

const TOOL_ICONS: Record<string, string> = {
  read_file: "📄",
  list_dir: "📁",
  grep: "🔍",
  glob: "🗂",
  bulk_read: "📚",
  write_file: "✍️",
  edit: "✎",
  patch: "✎",
  bulk_write: "📝",
  bulk_edit: "📝",
  bash: "⌘",
  diagnostics: "✦",
  todo_write: "☑",
  todo_read: "☑",
  finish: "✓",
  spawn: "↳",
  subagent: "↳",
  contact_supervisor: "💬",
  intercom: "💬",
  memory: "🧠",
  fetch: "🌐",
  git_status: "⎇",
  git_diff: "⎇",
  git_log: "⎇",
};

export function toolIcon(name: string): string {
  return TOOL_ICONS[name] ?? "🔧";
}

/** Pretty-print a tool's args JSON, capped to keep the card compact. */
export function prettyArgs(args: Record<string, unknown>): string {
  try {
    const json = JSON.stringify(args, null, 2);
    return json.length > 4000 ? json.slice(0, 4000) + "\n…(truncated)" : json;
  } catch {
    return String(args);
  }
}
