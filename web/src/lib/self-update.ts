// Start a Catalyst Code self-update from the web process.
//
// Prefer `catcode --update` (updates CLI + installed web on Linux/macOS/Windows).
// Fallbacks:
//   - Unix: download install.sh --update
//   - Windows: download packaging/windows/install-web.ps1 and re-run it
// The work is detached so a service restart mid-update does not strand the HTTP handler.

import { spawn, execFileSync } from "node:child_process";
import { existsSync, mkdirSync, openSync, readFileSync, writeFileSync } from "node:fs";
import { homedir, tmpdir } from "node:os";
import { join } from "node:path";

const GITHUB_REPO = "catalystctl/catcode";
const INSTALLER_URL =
  process.env.CATCODE_INSTALLER_URL ||
  `https://raw.githubusercontent.com/${GITHUB_REPO}/refs/heads/master/install.sh`;
const WINDOWS_INSTALLER_URL =
  process.env.CATCODE_WINDOWS_WEB_INSTALLER_URL ||
  `https://raw.githubusercontent.com/${GITHUB_REPO}/refs/heads/master/packaging/windows/install-web.ps1`;

const IS_WIN = process.platform === "win32";

function cacheDir(): string {
  if (IS_WIN) {
    const base = process.env.LOCALAPPDATA || join(homedir(), "AppData", "Local");
    return join(base, "catalyst-code", "cache");
  }
  return join(process.env.HOME || homedir() || tmpdir(), ".cache", "catalyst-code");
}

const LOCK_DIR = cacheDir();
const LOCK_FILE = join(LOCK_DIR, "web-self-update.lock");
const LOG_FILE = join(LOCK_DIR, "web-self-update.log");

export type SelfUpdateStartResult =
  | { ok: true; status: "started" | "already_running"; message: string; logPath: string }
  | { ok: false; error: string; hint?: string };

function which(bin: string): string | null {
  try {
    if (IS_WIN) {
      const out = execFileSync("where.exe", [bin], {
        encoding: "utf8",
        timeout: 5_000,
        stdio: ["ignore", "pipe", "ignore"],
        windowsHide: true,
      })
        .split(/\r?\n/)
        .map((s) => s.trim())
        .filter(Boolean)[0];
      return out || null;
    }
    const out = execFileSync("bash", ["-lc", `command -v ${bin}`], {
      encoding: "utf8",
      timeout: 5_000,
      stdio: ["ignore", "pipe", "ignore"],
    }).trim();
    return out || null;
  } catch {
    return null;
  }
}

function resolveCatcodeBinary(): string | null {
  const names = IS_WIN ? ["catcode.exe", "catcode"] : ["catcode"];
  for (const name of names) {
    const fromPath = which(name);
    if (fromPath && existsSync(fromPath)) return fromPath;
  }

  const home = process.env.HOME || homedir() || "";
  const local = process.env.LOCALAPPDATA || (home ? join(home, "AppData", "Local") : "");
  const candidates = IS_WIN
    ? [
        local ? join(local, "Programs", "catcode", "catcode.exe") : "",
        home ? join(home, "bin", "catcode.exe") : "",
      ]
    : [
        "/usr/local/bin/catcode",
        home ? join(home, ".local", "bin", "catcode") : "",
        home ? join(home, "bin", "catcode") : "",
      ];

  for (const p of candidates.filter(Boolean)) {
    if (existsSync(p)) return p;
  }
  return null;
}

function readLock(): { pid: number; startedAt: number } | null {
  try {
    const raw = readFileSync(LOCK_FILE, "utf8");
    const data = JSON.parse(raw) as { pid?: number; startedAt?: number };
    if (typeof data.pid !== "number") return null;
    return { pid: data.pid, startedAt: data.startedAt ?? 0 };
  } catch {
    return null;
  }
}

function pidAlive(pid: number): boolean {
  try {
    process.kill(pid, 0);
    return true;
  } catch {
    return false;
  }
}

async function downloadText(url: string, dest: string, marker: string): Promise<void> {
  const res = await fetch(url, {
    headers: { "User-Agent": "catalyst-code-web-updater" },
    signal: AbortSignal.timeout(30_000),
  });
  if (!res.ok) {
    throw new Error(`download failed (${url}): HTTP ${res.status}`);
  }
  const text = await res.text();
  if (!text.includes(marker)) {
    throw new Error(`downloaded file from ${url} does not look right`);
  }
  writeFileSync(dest, text, { mode: 0o755 });
}

function spawnDetached(command: string, args: string[], env: NodeJS.ProcessEnv = process.env): number {
  mkdirSync(LOCK_DIR, { recursive: true });
  const logFd = openSync(LOG_FILE, "a");
  const child = spawn(command, args, {
    detached: true,
    stdio: ["ignore", logFd, logFd],
    windowsHide: true,
    env: {
      ...env,
      SUDO_ASKPASS: undefined,
    },
  });
  child.unref();
  if (typeof child.pid !== "number") {
    throw new Error("failed to spawn update process");
  }
  writeFileSync(
    LOCK_FILE,
    JSON.stringify({ pid: child.pid, startedAt: Date.now(), command, args }, null, 2) + "\n",
  );
  return child.pid;
}

function resolvePowerShell(): string {
  return which("pwsh") || which("powershell") || "powershell.exe";
}

/**
 * Kick off an update. Returns immediately; poll GET /api/version for the new
 * commit after the service comes back.
 */
export async function startSelfUpdate(): Promise<SelfUpdateStartResult> {
  const existing = readLock();
  if (existing && pidAlive(existing.pid)) {
    return {
      ok: true,
      status: "already_running",
      message: `Update already in progress (pid ${existing.pid})`,
      logPath: LOG_FILE,
    };
  }

  mkdirSync(LOCK_DIR, { recursive: true });
  writeFileSync(LOG_FILE, `\n==== self-update ${new Date().toISOString()} ====\n`, { flag: "a" });

  const catcode = resolveCatcodeBinary();
  if (catcode) {
    try {
      const pid = spawnDetached(catcode, ["--update"]);
      return {
        ok: true,
        status: "started",
        message: `Started catcode --update (pid ${pid}). The page may disconnect while the service restarts.`,
        logPath: LOG_FILE,
      };
    } catch (err) {
      return {
        ok: false,
        error: err instanceof Error ? err.message : String(err),
        hint: IS_WIN
          ? "Try from an elevated PowerShell: catcode --update"
          : "Try: sudo catcode --update",
      };
    }
  }

  if (IS_WIN) {
    const installerPath = join(LOCK_DIR, "install-web.ps1");
    try {
      await downloadText(WINDOWS_INSTALLER_URL, installerPath, "CatalystCodeWeb");
    } catch (err) {
      return {
        ok: false,
        error: err instanceof Error ? err.message : String(err),
        hint: "Install catcode.exe on PATH, or re-run packaging/windows/install-web.ps1",
      };
    }
    const ps = resolvePowerShell();
    try {
      const pid = spawnDetached(ps, [
        "-NoProfile",
        "-ExecutionPolicy",
        "Bypass",
        "-File",
        installerPath,
      ]);
      return {
        ok: true,
        status: "started",
        message: `Started install-web.ps1 (pid ${pid}). The page may disconnect while the service restarts.`,
        logPath: LOG_FILE,
      };
    } catch (err) {
      return {
        ok: false,
        error: err instanceof Error ? err.message : String(err),
        hint: "Re-run: pwsh -ExecutionPolicy Bypass -File packaging/windows/install-web.ps1",
      };
    }
  }

  // Unix fallback: download install.sh and run --update.
  const installerPath = join(LOCK_DIR, "install.sh");
  try {
    await downloadText(INSTALLER_URL, installerPath, "install.sh");
  } catch (err) {
    return {
      ok: false,
      error: err instanceof Error ? err.message : String(err),
      hint: "Install catcode on PATH, or run: curl -fsSL …/install.sh | bash -s -- --update",
    };
  }

  const bash = which("bash") || "/bin/bash";
  try {
    const pid = spawnDetached(bash, [installerPath, "--update"]);
    return {
      ok: true,
      status: "started",
      message: `Started install.sh --update (pid ${pid}). The page may disconnect while the service restarts.`,
      logPath: LOG_FILE,
    };
  } catch (err) {
    return {
      ok: false,
      error: err instanceof Error ? err.message : String(err),
      hint: "Re-run with privileges: sudo bash install.sh --update",
    };
  }
}

export function selfUpdateLogPath(): string {
  return LOG_FILE;
}

export function updateLockActive(): boolean {
  const existing = readLock();
  return Boolean(existing && pidAlive(existing.pid));
}

function windowsWebDirCandidates(): string[] {
  const local = process.env.LOCALAPPDATA || "";
  const home = process.env.HOME || homedir() || "";
  const out: string[] = [];
  if (local) out.push(join(local, "catalyst-code", "web"));
  if (home) out.push(join(home, "AppData", "Local", "catalyst-code", "web"));
  return out;
}

function unixWebDirCandidates(): string[] {
  const home = process.env.HOME || homedir() || "";
  const out = ["/opt/catalyst-code/web"];
  if (home) {
    out.push(join(home, "Library", "Application Support", "catalyst-code", "web"));
    out.push(join(home, ".local", "share", "catalyst-code", "web"));
  }
  return out;
}

function stateSaysWebInstalled(statePath: string): boolean {
  try {
    const raw = readFileSync(statePath, "utf8");
    return /^WEB_INSTALLED="?yes"?/m.test(raw);
  } catch {
    return false;
  }
}

/** True when a release web install dir (or installer state) is present. */
export function webInstallDetected(cwd = process.cwd()): boolean {
  const statePaths = IS_WIN
    ? [
        process.env.LOCALAPPDATA
          ? join(process.env.LOCALAPPDATA, "catalyst-code", "installer.state")
          : "",
      ].filter(Boolean)
    : ["/etc/catalyst-code/installer.state", join(homedir(), ".config", "catalyst-code", "installer.state")];

  for (const statePath of statePaths) {
    if (existsSync(statePath) && stateSaysWebInstalled(statePath)) return true;
  }

  // Running from the installed bundle itself.
  if (existsSync(join(cwd, "start.js")) && existsSync(join(cwd, "version.json"))) {
    return true;
  }

  const dirs = IS_WIN ? windowsWebDirCandidates() : unixWebDirCandidates();
  for (const dir of dirs) {
    if (existsSync(join(dir, "start.js")) && existsSync(join(dir, "version.json"))) {
      return true;
    }
  }
  return false;
}
