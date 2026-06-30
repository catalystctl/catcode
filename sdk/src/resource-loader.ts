// Skills + ResourceLoader — mirrors `pi-coding-agent`'s `core/skills.ts` and
// `core/resource-loader.ts` (the subset pi-web's `pi-skills.ts` uses).

import { existsSync, readFileSync, readdirSync, statSync } from "node:fs";
import { dirname, join, extname } from "node:path";
import type { TextContent, ImageContent } from "./types.js";

export type SourceScope = "user" | "project" | "temporary";
export type SourceOrigin = "package" | "top-level";
export interface SourceInfo {
  path: string;
  source: string;
  scope: SourceScope;
  origin: SourceOrigin;
  baseDir?: string;
}

export interface ResourceDiagnostic {
  type: "error" | "warning" | "info";
  path?: string;
  message: string;
}

export interface SkillFrontmatter {
  name?: string;
  description?: string;
  "disable-model-invocation"?: boolean;
  [key: string]: unknown;
}

export interface Skill {
  name: string;
  description: string;
  filePath: string;
  baseDir: string;
  sourceInfo: SourceInfo;
  disableModelInvocation: boolean;
}

export interface LoadSkillsResult {
  skills: Skill[];
  diagnostics: ResourceDiagnostic[];
}

export interface LoadSkillsFromDirOptions {
  dir: string;
  scope: SourceScope;
  origin?: SourceOrigin;
}

export interface LoadSkillsOptions {
  cwd: string;
  agentDir: string;
  skillPaths: string[];
  includeDefaults: boolean;
}

export interface PromptTemplate {
  name: string;
  description?: string;
  location?: string;
  path?: string;
  sourceInfo?: SourceInfo;
}

export interface ResourceCollision {}

export interface ResourceLoader {
  getExtensions(): any;
  getSkills(): { skills: Skill[]; diagnostics: ResourceDiagnostic[] };
  getPrompts(): { prompts: PromptTemplate[]; diagnostics: ResourceDiagnostic[] };
  getThemes(): { themes: any[]; diagnostics: ResourceDiagnostic[] };
  getAgentsFiles(): { agentsFiles: Array<{ path: string; content: string }> };
  getSystemPrompt(): string | undefined;
  getAppendSystemPrompt(): string[];
  extendResources(paths: any): void;
  reload(): Promise<void>;
}

export interface DefaultResourceLoaderOptions {
  cwd: string;
  agentDir: string;
  settingsManager?: any;
  eventBus?: any;
  skillPaths?: string[];
  extensionPaths?: string[];
  systemPromptOverride?: (base: string) => string;
  appendSystemPromptOverride?: () => string[];
  extensionFactories?: any[];
  skillsOverride?: () => LoadSkillsResult;
  agentsFilesOverride?: () => { agentsFiles: Array<{ path: string; content: string }> };
  promptsOverride?: () => { prompts: PromptTemplate[]; diagnostics: ResourceDiagnostic[] };
}

/** Parse YAML-ish frontmatter from a markdown skill file. */
export function parseFrontmatter(text: string): { frontmatter: Record<string, any>; body: string } {
  const m = text.match(/^---\r?\n([\s\S]*?)\r?\n---\r?\n?([\s\S]*)$/);
  if (!m) return { frontmatter: {}, body: text };
  const fm: Record<string, any> = {};
  for (const line of m[1].split(/\r?\n/)) {
    const idx = line.indexOf(":");
    if (idx > 0) fm[line.slice(0, idx).trim()] = line.slice(idx + 1).trim();
  }
  return { frontmatter: fm, body: m[2] };
}

export function stripFrontmatter(text: string): string {
  return parseFrontmatter(text).body;
}

function loadSkillFile(filePath: string, scope: SourceScope, origin: SourceOrigin = "top-level"): Skill | undefined {
  if (!existsSync(filePath)) return undefined;
  let text = "";
  try {
    text = readFileSync(filePath, "utf8");
  } catch {
    return undefined;
  }
  const { frontmatter } = parseFrontmatter(text);
  const name = frontmatter.name ?? basenameNoExt(filePath);
  return {
    name,
    description: frontmatter.description ?? "",
    filePath,
    baseDir: dirname(filePath),
    sourceInfo: { path: filePath, source: scope, scope, origin, baseDir: dirname(filePath) },
    disableModelInvocation: !!frontmatter["disable-model-invocation"],
  };
}

function basenameNoExt(p: string): string {
  const base = p.split(/[\\/]/).pop() ?? p;
  return base.replace(/\.md$/i, "");
}

/** Load skills from a single directory (SKILL.md or direct .md children). */
export function loadSkillsFromDir(options: LoadSkillsFromDirOptions): LoadSkillsResult {
  const { dir, scope, origin } = options;
  const skills: Skill[] = [];
  const diagnostics: ResourceDiagnostic[] = [];
  if (!existsSync(dir)) return { skills, diagnostics };
  const skillMd = join(dir, "SKILL.md");
  if (existsSync(skillMd) && statSync(skillMd).isFile()) {
    const s = loadSkillFile(skillMd, scope, origin);
    if (s) skills.push(s);
    return { skills, diagnostics };
  }
  // Direct .md children, then recurse one level for SKILL.md.
  let entries: string[] = [];
  try {
    entries = readdirSync(dir);
  } catch {
    return { skills, diagnostics };
  }
  for (const name of entries) {
    const p = join(dir, name);
    try {
      if (!statSync(p).isFile()) continue;
    } catch {
      continue;
    }
    if (extname(name).toLowerCase() === ".md") {
      const s = loadSkillFile(p, scope, origin);
      if (s) skills.push(s);
    }
  }
  return { skills, diagnostics };
}

/** Load skills from all configured locations. */
export function loadSkills(options: LoadSkillsOptions): LoadSkillsResult {
  const { cwd, agentDir, skillPaths, includeDefaults } = options;
  const skills: Skill[] = [];
  const diagnostics: ResourceDiagnostic[] = [];
  const seen = new Set<string>();
  const add = (s: Skill | undefined) => {
    if (s && !seen.has(s.filePath)) {
      seen.add(s.filePath);
      skills.push(s);
    }
  };
  if (includeDefaults) {
    for (const s of loadSkillsFromDir({ dir: join(agentDir, "skills"), scope: "user" }).skills) add(s);
    for (const s of loadSkillsFromDir({ dir: join(cwd, ".pi", "skills"), scope: "project" }).skills) add(s);
  }
  for (const sp of skillPaths) {
    try {
      if (existsSync(sp) && statSync(sp).isFile()) {
        add(loadSkillFile(sp, "project"));
      } else {
        for (const s of loadSkillsFromDir({ dir: sp, scope: "project" }).skills) add(s);
      }
    } catch (e: any) {
      diagnostics.push({ type: "warning", path: sp, message: e?.message ?? String(e) });
    }
  }
  return { skills, diagnostics };
}

/** Format skills for the system prompt (XML, per the Agent Skills standard). */
export function formatSkillsForPrompt(skills: Skill[]): string {
  const usable = skills.filter((s) => !s.disableModelInvocation);
  if (usable.length === 0) return "";
  const items = usable
    .map((s) => `  <skill name="${s.name}">\n    ${s.description.replace(/\n/g, "\n    ")}\n  </skill>`)
    .join("\n");
  return `<skills>\n${items}\n</skills>`;
}

/** Load AGENTS.md-style project context files. */
export function loadProjectContextFiles(options: { cwd: string; agentDir: string }): Array<{ path: string; content: string }> {
  const { cwd, agentDir } = options;
  const out: Array<{ path: string; content: string }> = [];
  for (const dir of [cwd, agentDir]) {
    for (const name of ["AGENTS.md", "CLAUDE.md", ".pi/AGENTS.md"]) {
      const p = join(dir, name);
      if (existsSync(p)) {
        try {
          out.push({ path: p, content: readFileSync(p, "utf8") });
        } catch {
          /* skip */
        }
      }
    }
  }
  return out;
}

/** Minimal resource loader backed by the filesystem (skills/prompts/context). */
export class DefaultResourceLoader implements ResourceLoader {
  private options: DefaultResourceLoaderOptions;
  private skillsCache: LoadSkillsResult = { skills: [], diagnostics: [] };

  constructor(options: DefaultResourceLoaderOptions) {
    this.options = options;
  }

  getExtensions(): any {
    return { extensions: [], diagnostics: [] };
  }
  getSkills(): { skills: Skill[]; diagnostics: ResourceDiagnostic[] } {
    return this.skillsCache;
  }
  getPrompts(): { prompts: PromptTemplate[]; diagnostics: ResourceDiagnostic[] } {
    return { prompts: [], diagnostics: [] };
  }
  getThemes(): { themes: any[]; diagnostics: ResourceDiagnostic[] } {
    return { themes: [], diagnostics: [] };
  }
  getAgentsFiles(): { agentsFiles: Array<{ path: string; content: string }> } {
    return { agentsFiles: loadProjectContextFiles(this.options) };
  }
  getSystemPrompt(): string | undefined {
    return undefined;
  }
  getAppendSystemPrompt(): string[] {
    return this.options.appendSystemPromptOverride?.() ?? [];
  }
  extendResources(_paths: any): void {
    /* no-op */
  }
  async reload(): Promise<void> {
    if (this.options.skillsOverride) {
      this.skillsCache = this.options.skillsOverride();
      return;
    }
    this.skillsCache = loadSkills({
      cwd: this.options.cwd,
      agentDir: this.options.agentDir,
      skillPaths: this.options.skillPaths ?? [],
      includeDefaults: true,
    });
  }
}

export type { TextContent, ImageContent };
