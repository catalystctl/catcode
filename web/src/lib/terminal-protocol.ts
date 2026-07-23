export interface TerminalOpenEnvelope {
  type: "open";
  sessionId: string;
  workspace: string;
  cwd: string;
  cols: number;
  rows: number;
  attachOnly: boolean;
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
): TerminalOpenEnvelope {
  return { type: "open", sessionId, workspace, cwd, cols, rows, attachOnly };
}

export function terminalTerminateEnvelope(
  sessionId: string,
  workspace: string,
): TerminalTerminateEnvelope {
  return { type: "terminate", sessionId, workspace };
}
