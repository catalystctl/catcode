// POST /api/command — forward a core command to a SPECIFIC session's core.
//
// The body is a raw core command ({ type: "send", ... }) plus optional routing
// metadata:
//   session=<absolute session file>   which session to target
//   workspace=<workspace dir>         needed when the session isn't live yet
//
// The response arrives asynchronously over the SSE stream (e.g. list_sessions →
// sessions), so this endpoint just acknowledges acceptance — EXCEPT for the
// bridge-intercepted commands (switch_workspace / new_session / load_session /
// rename_session / list_projects / add_project / remove_project), which the
// bridge handles itself and which may return a `session`/`workspace` so the
// client can switch its active session.

import { getBridge } from "@/server/core-bridge";
import { authorized } from "@/lib/auth";
import type { CoreCommand } from "@/lib/types";

export const dynamic = "force-dynamic";
export const runtime = "nodejs";

export async function POST(req: Request) {
  if (!authorized(req)) return Response.json({ ok: false, error: "unauthorized" }, { status: 401 });
  const bridge = getBridge();

  let body: Record<string, unknown>;
  try {
    body = (await req.json()) as Record<string, unknown>;
  } catch {
    return Response.json({ ok: false, error: "invalid JSON body" }, { status: 400 });
  }
  if (!body || typeof body.type !== "string") {
    return Response.json({ ok: false, error: "missing command type" }, { status: 400 });
  }

  // Split routing metadata from the core command.
  const { session, workspace, ...cmdRest } = body;
  const sessionFile = typeof session === "string" ? session : undefined;
  const workspaceDir = typeof workspace === "string" ? workspace : undefined;
  const cmd = cmdRest as CoreCommand;

  // ── Bridge-intercepted commands (never forwarded to a core) ──
  try {
    switch (cmd.type) {
      case "switch_workspace": {
        const path = (cmd as { path: string }).path;
        const { session: file, workspace: ws } = await bridge.switchWorkspace(path);
        return Response.json({ ok: true, session: file, workspace: ws });
      }
      case "new_session": {
        const ws = workspaceDir ?? bridge.getDefaultWorkspace();
        const { session: file, workspace: ws2 } = await bridge.newSession(ws);
        return Response.json({ ok: true, session: file, workspace: ws2 });
      }
      case "load_session": {
        // Switching sessions is client-driven (reopen the stream); the core for
        // each session loads its own file on start. Echo the path back so the
        // client can switch its active session if it sent this command.
        const path = (cmd as { path: string }).path;
        return Response.json({ ok: true, session: path });
      }
      case "rename_session": {
        const { name, title } = cmd as { name: string; title: string };
        bridge.renameSession(workspaceDir ?? bridge.getDefaultWorkspace(), name, title);
        return Response.json({ ok: true });
      }
      case "list_projects":
        bridge.broadcastProjects();
        return Response.json({ ok: true });
      case "add_project": {
        const path = (cmd as { path: string }).path;
        bridge.addProject(path);
        return Response.json({ ok: true });
      }
      case "remove_project": {
        const path = (cmd as { path: string }).path;
        bridge.removeProjectEntry(path);
        return Response.json({ ok: true });
      }
      case "delete_session": {
        const path = (cmd as { path: string }).path;
        const ws = workspaceDir ?? bridge.getWorkspaceForSession(path) ?? bridge.getDefaultWorkspace();
        const { session: file, workspace: ws2 } = await bridge.deleteSession(ws, path);
        return Response.json({ ok: true, session: file, workspace: ws2 });
      }
    }

    // ── Route to the target session's core ──
    const ws = workspaceDir ?? bridge.getWorkspaceForSession(sessionFile) ?? bridge.getDefaultWorkspace();
    const file = sessionFile ?? bridge.mostRecentSession(ws);
    const live = bridge.getOrCreate(ws, file);
    await live.ensure();
    live.send(cmd);
    return Response.json({ ok: true });
  } catch (err: any) {
    return Response.json(
      { ok: false, error: err?.message ?? "send failed" },
      { status: 500 },
    );
  }
}
