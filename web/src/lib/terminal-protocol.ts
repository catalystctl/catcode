/** What the server should spawn inside the PTY. "shell" (default) runs the
 *  user's login shell; "catcode" runs the Catalyst Code TUI in the project
 *  root (used by the /hub terminal workspace). */
export type TerminalLaunch = "shell" | "catcode";

export interface TerminalOpenEnvelope {
  type: "open";
  sessionId: string;
  workspace: string;
  cwd: string;
  cols: number;
  rows: number;
  attachOnly: boolean;
  /** Omitted or "shell" keeps the historical behavior. */
  launch?: TerminalLaunch;
}

export interface TerminalTerminateEnvelope {
  type: "terminate";
  sessionId: string;
  workspace: string;
}

/** Collision-free in-memory identity for one user's terminal in one workspace. */
export function terminalSessionKey(
  ownerId: string,
  workspace: string,
  sessionId: string,
): string {
  return JSON.stringify([ownerId, workspace, sessionId]);
}

export function terminalOpenEnvelope(
  sessionId: string,
  workspace: string,
  cwd: string,
  cols: number,
  rows: number,
  attachOnly: boolean,
  launch?: TerminalLaunch,
): TerminalOpenEnvelope {
  // Only carry the field when it differs from the historical default so old
  // servers (which ignore unknown fields anyway) see an identical envelope.
  return launch && launch !== "shell"
    ? { type: "open", sessionId, workspace, cwd, cols, rows, attachOnly, launch }
    : { type: "open", sessionId, workspace, cwd, cols, rows, attachOnly };
}

export function terminalTerminateEnvelope(
  sessionId: string,
  workspace: string,
): TerminalTerminateEnvelope {
  return { type: "terminate", sessionId, workspace };
}
