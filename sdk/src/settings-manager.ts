// SettingsManager — mirrors `pi-coding-agent`'s `core/settings-manager.ts`.
// Minimal implementation covering the surface pi-web actually uses
// (`create`, `getSkillPaths`, `setSkillPaths`, `setProjectSkillPaths`).
// Settings persist to `<configDir>/settings.json` (0600), matching the TUI.

import { existsSync, readFileSync, writeFileSync, mkdirSync } from "node:fs";
import { join } from "node:path";
import { configDir, ensureDir } from "./config.js";

export interface CompactionSettings {
  enabled: boolean;
  reserveTokens: number;
  keepRecentTokens: number;
}
export interface RetrySettings {
  enabled: boolean;
  maxRetries: number;
  baseDelayMs: number;
}
export interface ImageSettings {
  autoResize: boolean;
  blockImages: boolean;
}
export interface PackageSource {
  type: string;
  path?: string;
}
export type TransportSetting = string | undefined;

interface Settings {
  defaultProvider?: string;
  defaultModel?: string;
  defaultThinkingLevel?: string;
  steeringMode?: "all" | "one-at-a-time";
  followUpMode?: "all" | "one-at-a-time";
  compactionEnabled?: boolean;
  retryEnabled?: boolean;
  sessionDir?: string;
  httpIdleTimeoutMs?: number;
  skillPaths?: string[];
  skills?: string[];
  projectSkillPaths?: string[];
  extensionPaths?: string[];
  promptTemplatePaths?: string[];
  themePaths?: string[];
  enabledModels?: string[];
  showImages?: boolean;
  imageAutoResize?: boolean;
  blockImages?: boolean;
  shellPath?: string;
  packages?: PackageSource[];
  [key: string]: unknown;
}

export class SettingsManager {
  private cwd: string;
  private agentDir: string;
  private global: Settings;
  private project: Settings;
  private overrides: Partial<Settings> = {};

  private constructor(cwd: string, agentDir: string) {
    this.cwd = cwd;
    this.agentDir = agentDir;
    this.global = load(join(agentDir, "settings.json"));
    this.project = load(join(cwd, ".pi", "settings.json"));
  }

  static create(cwd: string, agentDir?: string): SettingsManager {
    return new SettingsManager(cwd, agentDir ?? configDir());
  }

  static fromStorage(_storage: any): SettingsManager {
    return new SettingsManager(process.cwd(), configDir());
  }

  static inMemory(settings?: Partial<Settings>): SettingsManager {
    const sm = new SettingsManager(process.cwd(), configDir());
    if (settings) sm.applyOverrides(settings);
    return sm;
  }

  getGlobalSettings(): Settings {
    return { ...this.global };
  }
  getProjectSettings(): Settings {
    return { ...this.project } as Settings;
  }

  reload(): Promise<void> {
    this.global = load(join(this.agentDir, "settings.json"));
    this.project = load(join(this.cwd, ".pi", "settings.json"));
    return Promise.resolve();
  }

  applyOverrides(overrides: Partial<Settings>): void {
    this.overrides = { ...this.overrides, ...overrides };
  }

  flush(): Promise<void> {
    save(join(this.agentDir, "settings.json"), this.global);
    save(join(this.cwd, ".pi", "settings.json"), this.project);
    return Promise.resolve();
  }

  drainErrors(): any[] {
    return [];
  }

  private merged(): Settings {
    return { ...this.global, ...this.project, ...this.overrides };
  }

  // ── representative getters/setters (covers pi-web usage) ──
  getDefaultProvider(): string | undefined {
    return this.merged().defaultProvider;
  }
  setDefaultProvider(p?: string): void {
    this.global.defaultProvider = p;
  }
  getDefaultModel(): string | undefined {
    return this.merged().defaultModel;
  }
  setDefaultModel(id?: string): void {
    this.global.defaultModel = id;
  }
  setDefaultModelAndProvider(p: string, id: string): void {
    this.global.defaultProvider = p;
    this.global.defaultModel = id;
  }
  getDefaultThinkingLevel(): string | undefined {
    return this.merged().defaultThinkingLevel;
  }
  setDefaultThinkingLevel(level?: string): void {
    this.global.defaultThinkingLevel = level;
  }
  getSteeringMode(): "all" | "one-at-a-time" {
    return this.merged().steeringMode ?? "all";
  }
  setSteeringMode(mode: "all" | "one-at-a-time"): void {
    this.global.steeringMode = mode;
  }
  getFollowUpMode(): "all" | "one-at-a-time" {
    return this.merged().followUpMode ?? "all";
  }
  setFollowUpMode(mode: "all" | "one-at-a-time"): void {
    this.global.followUpMode = mode;
  }
  getCompactionEnabled(): boolean {
    return this.merged().compactionEnabled ?? true;
  }
  setCompactionEnabled(b: boolean): void {
    this.global.compactionEnabled = b;
  }
  getCompactionSettings(): CompactionSettings {
    return { enabled: true, reserveTokens: 8192, keepRecentTokens: 32768 };
  }
  getRetryEnabled(): boolean {
    return this.merged().retryEnabled ?? true;
  }
  setRetryEnabled(b: boolean): void {
    this.global.retryEnabled = b;
  }
  getRetrySettings(): RetrySettings {
    return { enabled: true, maxRetries: 3, baseDelayMs: 1000 };
  }
  getSessionDir(): string | undefined {
    return this.merged().sessionDir;
  }
  getHttpIdleTimeoutMs(): number {
    return 60000;
  }
  setHttpIdleTimeoutMs(n: number): void {
    this.global.httpIdleTimeoutMs = n;
  }
  getPackages(): PackageSource[] {
    return this.merged().packages ?? [];
  }
  setPackages(p: PackageSource[]): void {
    this.global.packages = p;
  }
  setProjectPackages(p: PackageSource[]): void {
    this.project.packages = p;
  }
  getExtensionPaths(): string[] {
    return this.merged().extensionPaths ?? [];
  }
  setExtensionPaths(p: string[]): void {
    this.global.extensionPaths = p;
  }
  setProjectExtensionPaths(p: string[]): void {
    this.project.extensionPaths = p;
  }
  getSkillPaths(): string[] {
    return this.merged().skillPaths ?? [];
  }
  setSkillPaths(p: string[]): void {
    this.global.skillPaths = p;
  }
  setProjectSkillPaths(p: string[]): void {
    this.project.skills = p;
  }
  getProjectSkillPaths(): string[] {
    return this.project.skills ?? [];
  }
  getPromptTemplatePaths(): string[] {
    return this.merged().promptTemplatePaths ?? [];
  }
  setPromptTemplatePaths(p: string[]): void {
    this.global.promptTemplatePaths = p;
  }
  setProjectPromptTemplatePaths(p: string[]): void {
    this.project.promptTemplatePaths = p;
  }
  getThemePaths(): string[] {
    return this.merged().themePaths ?? [];
  }
  setThemePaths(p: string[]): void {
    this.global.themePaths = p;
  }
  setProjectThemePaths(p: string[]): void {
    this.project.themePaths = p;
  }
  getEnabledModels(): string[] | undefined {
    return this.merged().enabledModels;
  }
  setEnabledModels(p?: string[]): void {
    this.global.enabledModels = p;
  }
  getShowImages(): boolean {
    return this.merged().showImages ?? true;
  }
  setShowImages(b: boolean): void {
    this.global.showImages = b;
  }
  getImageAutoResize(): boolean {
    return this.merged().imageAutoResize ?? true;
  }
  setImageAutoResize(b: boolean): void {
    this.global.imageAutoResize = b;
  }
  getBlockImages(): boolean {
    return this.merged().blockImages ?? false;
  }
  setBlockImages(b: boolean): void {
    this.global.blockImages = b;
  }
  getShellPath(): string | undefined {
    return this.merged().shellPath;
  }
  setShellPath(p?: string): void {
    this.global.shellPath = p;
  }
}

function load(path: string): Settings {
  if (!existsSync(path)) return {};
  try {
    return JSON.parse(readFileSync(path, "utf8")) as Settings;
  } catch {
    return {};
  }
}

function save(path: string, settings: Settings): void {
  try {
    ensureDir(join(path, ".."));
    mkdirSync(join(path, ".."), { recursive: true });
    writeFileSync(path, JSON.stringify(settings, null, 2), { mode: 0o600 });
  } catch {
    /* best-effort */
  }
}
