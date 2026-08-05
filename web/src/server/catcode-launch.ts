// Resolves the catcode TUI binary for /hub terminal panes. The hub asks the
// terminal WebSocket to spawn `catcode` (the Go TUI) instead of the user's
// shell; this module finds that binary on the SERVER machine.
//
// Resolution order:
//   1. $CATCODE_WEB_TUI_BIN — explicit path override (absolute, or relative
//      to the web server's cwd). A non-empty override that does not exist
//      fails fast (null) instead of silently falling back to PATH.
//   2. A PATH walk for `catcode` (POSIX) or `catcode.exe` / `catcode.cmd` /
//      `catcode.bat` (Windows — install.ps1 puts catcode.exe on the user
//      PATH; npm shims may be .cmd).
//
// Pure Node (fs/path only) so it is unit-testable without a running server.

import { statSync } from "node:fs";
import { delimiter, isAbsolute, join } from "node:path";

export const CATCODE_BIN_ENV = "CATCODE_WEB_TUI_BIN";

function isSpawnableFile(absPath: string, checkExecBit: boolean): boolean {
  try {
    const st = statSync(absPath);
    if (!st.isFile()) return false;
    if (checkExecBit && (st.mode & 0o111) === 0) return false;
    return true;
  } catch {
    return false;
  }
}

/** Names tried in each PATH dir, in order. Windows installs ship catcode.exe;
 *  package-manager shims may be .cmd/.bat wrappers. */
function candidateNames(platform: NodeJS.Platform): string[] {
  return platform === "win32"
    ? ["catcode.exe", "catcode.cmd", "catcode.bat"]
    : ["catcode"];
}

/** Resolve the catcode TUI binary, or null when it cannot be found. */
export function resolveCatcodeBinary(
  env: Record<string, string | undefined> = process.env,
  platform: NodeJS.Platform = process.platform,
  cwd: string = process.cwd(),
): string | null {
  const override = env[CATCODE_BIN_ENV];
  if (override && override.trim()) {
    const abs = isAbsolute(override) ? override : join(cwd, override);
    return isSpawnableFile(abs, platform !== "win32") ? abs : null;
  }

  const pathValue = env.PATH ?? env.Path ?? "";
  for (const dir of pathValue.split(delimiter)) {
    if (!dir) continue;
    for (const name of candidateNames(platform)) {
      const cand = join(dir, name);
      if (isSpawnableFile(cand, platform !== "win32")) return cand;
    }
  }
  return null;
}

/** Error shown in the pane when resolution fails (surfaced via the WS
 *  `fail()` path so the user sees it inside the terminal). */
export const CATCODE_NOT_FOUND_ERROR =
  "catcode TUI binary not found on the server — install catcode, add it to the server PATH, or set CATCODE_WEB_TUI_BIN";
