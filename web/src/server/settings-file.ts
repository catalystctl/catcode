// Shared read/write for ~/.config/catalyst-code/settings.json — the same file
// the TUI owns. Web must spawn cores with the persisted approval (and write it
// back on set_approval) so the gate doesn't silently reset to "destructive".
//
// Path derived from the SDK's configDir() so the directory resolves identically
// to the SDK, TUI, and Rust core. Field names match the TUI's on-disk JSON keys
// (model, base_url, provider, approval); SDK-style names (defaultModel,
// defaultProvider) are also tried as fallbacks so the web reads correctly
// regardless of which component wrote the file last.

import { existsSync, readFileSync, writeFileSync, mkdirSync, renameSync } from "node:fs";
import { join, dirname } from "node:path";
import { configDir } from "@catalyst-code/coding-agent";

export type ApprovalMode = "never" | "destructive" | "always";

export interface HarnessSettings {
  apiKey?: string;
  model?: string;
  baseUrl?: string;
  provider?: string;
  approval?: ApprovalMode;
  /** Web-only: show OS desktop notifications for cross-session events.
   *  Stored alongside TUI settings; the TUI ignores the key. */
  desktopNotifications?: boolean;
}

function settingsPath(): string {
  return join(configDir(), "settings.json");
}

function asApproval(v: unknown): ApprovalMode | undefined {
  if (v === "never" || v === "destructive" || v === "always") return v;
  return undefined;
}

/** Read TUI settings.json. API keys are not forwarded via env — the core loads
 *  provider_keys from the file itself for returning users. */
export function loadSettings(): HarnessSettings {
  const path = settingsPath();
  try {
    if (!existsSync(path)) return {};
    const raw = readFileSync(path, "utf8");
    const s = JSON.parse(raw) as Record<string, unknown>;
    const out: HarnessSettings = {};
    // Read TUI field names first; fall back to SDK-style names (defaultModel / defaultProvider)
    // so the web is compatible with whichever component wrote settings.json last.
    if (typeof s.model === "string" && s.model) out.model = s.model;
    else if (typeof s.defaultModel === "string" && s.defaultModel) out.model = s.defaultModel;
    if (typeof s.base_url === "string" && s.base_url) out.baseUrl = s.base_url;
    if (typeof s.provider === "string" && s.provider) out.provider = s.provider;
    else if (typeof s.defaultProvider === "string" && s.defaultProvider) out.provider = s.defaultProvider;
    else if (typeof s.active_provider === "string" && s.active_provider) out.provider = s.active_provider;
    const approval = asApproval(s.approval);
    if (approval) out.approval = approval;
    if (typeof s.desktop_notifications === "boolean") out.desktopNotifications = s.desktop_notifications;
    return out;
  } catch {
    return {};
  }
}

/** Merge-write a single approval mode into settings.json (atomic replace). */
export function saveApproval(mode: ApprovalMode): void {
  const path = settingsPath();
  let doc: Record<string, unknown> = {};
  try {
    if (existsSync(path)) {
      doc = JSON.parse(readFileSync(path, "utf8")) as Record<string, unknown>;
      if (!doc || typeof doc !== "object") doc = {};
    }
  } catch {
    doc = {};
  }
  doc.approval = mode;
  const dir = dirname(path);
  mkdirSync(dir, { recursive: true });
  const tmp = join(dir, `.settings.${process.pid}.${Date.now()}.tmp`);
  writeFileSync(tmp, `${JSON.stringify(doc, null, 2)}\n`, { mode: 0o600 });
  renameSync(tmp, path);
}

/** Merge-write the web-only desktop_notifications flag into settings.json
 *  (atomic replace). Stored under a `web`-namespaced key the TUI ignores. */
export function saveDesktopNotifications(enabled: boolean): void {
  const path = settingsPath();
  let doc: Record<string, unknown> = {};
  try {
    if (existsSync(path)) {
      doc = JSON.parse(readFileSync(path, "utf8")) as Record<string, unknown>;
      if (!doc || typeof doc !== "object") doc = {};
    }
  } catch {
    doc = {};
  }
  doc.desktop_notifications = enabled;
  const dir = dirname(path);
  mkdirSync(dir, { recursive: true });
  const tmp = join(dir, `.settings.${process.pid}.${Date.now()}.tmp`);
  writeFileSync(tmp, `${JSON.stringify(doc, null, 2)}\n`, { mode: 0o600 });
  renameSync(tmp, path);
}
