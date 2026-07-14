// Shared read/write for ~/.config/catalyst-code/settings.json — the same file
// the TUI owns. Web must spawn cores with the persisted approval (and write it
// back on set_approval) so the gate doesn't silently reset to "destructive".

import { existsSync, readFileSync, writeFileSync, mkdirSync, renameSync } from "node:fs";
import { homedir } from "node:os";
import { join, dirname } from "node:path";

export type ApprovalMode = "never" | "destructive" | "always";

export interface HarnessSettings {
  apiKey?: string;
  model?: string;
  baseUrl?: string;
  provider?: string;
  approval?: ApprovalMode;
}

function settingsPath(): string {
  return join(homedir() || ".", ".config", "catalyst-code", "settings.json");
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
    if (typeof s.model === "string" && s.model) out.model = s.model;
    if (typeof s.base_url === "string" && s.base_url) out.baseUrl = s.base_url;
    if (typeof s.provider === "string" && s.provider) out.provider = s.provider;
    const approval = asApproval(s.approval);
    if (approval) out.approval = approval;
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
