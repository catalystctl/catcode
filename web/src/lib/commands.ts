// Command catalog — the single source of truth for slash commands available in
// the composer flyout. Each entry has a slash label, a human description, an
// optional category, and the action key dispatched to the shell's onCommand.
// This replaces the duplicated SLASH set (composer.tsx) + onCommand switch
// (chat.tsx) so they can never drift apart.

export interface CommandDef {
  /** The full slash token, e.g. "/reset". */
  label: string;
  /** Short description shown in the flyout. */
  desc: string;
  /** Category for grouping in the flyout / help. */
  category: "session" | "config" | "agent" | "tools" | "help";
  /** Action key passed to onCommand. May differ from label (e.g. "/model" → "model"). */
  action: string;
  /** Whether this command also works while streaming (steer-class). */
  streaming?: boolean;
}

export const COMMANDS: CommandDef[] = [
  // ── Session ──
  { label: "/new", desc: "start a fresh session file", category: "session", action: "new" },
  { label: "/sessions", desc: "refresh session list", category: "session", action: "sessions" },
  { label: "/reset", desc: "wipe conversation + session file", category: "session", action: "reset" },
  { label: "/clear", desc: "clear view (keep session file)", category: "session", action: "clear" },
  { label: "/undo", desc: "drop last turn", category: "session", action: "undo" },
  { label: "/compact", desc: "force compaction (opt: instructions)", category: "session", action: "compact" },
  { label: "/stats", desc: "token + turn totals", category: "session", action: "stats" },
  { label: "/context", desc: "token-usage breakdown (top consumers)", category: "session", action: "context" },
  { label: "/usage", desc: "provider plan / rate-limit usage", category: "session", action: "usage" },
  { label: "/abort", desc: "stop running turn", category: "session", action: "abort", streaming: true },

  // ── Config ──
  { label: "/model", desc: "switch model", category: "config", action: "model" },
  { label: "/reasoning", desc: "set reasoning effort", category: "config", action: "reasoning" },
  { label: "/approval", desc: "never · destructive · always", category: "config", action: "approval" },
  { label: "/sandbox", desc: "none · firejail · seatbelt (bash hard sandbox)", category: "config", action: "sandbox" },
  { label: "/auto-compact", desc: "toggle auto context compaction", category: "config", action: "auto-compact" },
  { label: "/bash-timeout", desc: "set bash tool timeout (seconds)", category: "config", action: "bash-timeout" },
  { label: "/settings", desc: "open settings modal", category: "config", action: "settings" },
  { label: "/theme", desc: "switch colour theme", category: "config", action: "theme" },
  { label: "/login", desc: "log in / switch provider (OpenAI · Gemini · Anthropic)", category: "config", action: "login" },
  { label: "/logout", desc: "log out of a provider", category: "config", action: "logout" },
  { label: "/oauth-code", desc: "complete a manual OAuth login (paste code/URL)", category: "config", action: "oauth-code" },

  // ── Agent ──
  { label: "/steer", desc: "steer an in-flight turn", category: "agent", action: "steer", streaming: true },
  { label: "/goal", desc: "goal mode — plan & deploy subagents", category: "agent", action: "goal" },
  { label: "/cancel-goal", desc: "cancel active goal mode", category: "agent", action: "cancel-goal" },
  { label: "/run", desc: "delegate to a subagent", category: "agent", action: "run" },
  { label: "/parallel", desc: "run subagents in parallel", category: "agent", action: "parallel" },
  { label: "/chain", desc: "run a subagent chain", category: "agent", action: "chain" },
  { label: "/subagents", desc: "subagent runs + available agents", category: "agent", action: "subagents" },
  { label: "/index", desc: "bootstrap repo knowledge → memories + skills", category: "agent", action: "index" },
  { label: "/reflect", desc: "reflect on this session, persist learnings", category: "agent", action: "reflect" },

  // ── Tools ──
  { label: "/memory", desc: "list saved memories", category: "tools", action: "memory" },
  { label: "/remember", desc: "save a memory note", category: "tools", action: "remember" },
  { label: "/forget", desc: "forget a memory", category: "tools", action: "forget" },
  { label: "/plugins", desc: "manage plugins", category: "tools", action: "plugins" },
  { label: "/vision", desc: "configure vision handoff", category: "tools", action: "vision" },
  { label: "/attach", desc: "attach an image", category: "tools", action: "attach" },

  // ── Help / utility ──
  { label: "/help", desc: "keybindings & commands", category: "help", action: "help" },
  { label: "/copy", desc: "copy last assistant reply", category: "help", action: "copy" },
  { label: "/export", desc: "export transcript (markdown)", category: "help", action: "export" },
];

/** Quick lookup by label. */
export const COMMAND_BY_LABEL = new Map(COMMANDS.map((c) => [c.label, c]));

/** Fuzzy-filter commands by a query (matches label or desc, case-insensitive). */
export function filterCommands(query: string): CommandDef[] {
  const q = query.toLowerCase().replace(/^\//, "");
  if (!q) return COMMANDS;
  return COMMANDS.filter(
    (c) => c.label.includes(q) || c.desc.toLowerCase().includes(q) || c.action.includes(q),
  );
}
