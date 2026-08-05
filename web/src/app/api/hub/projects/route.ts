// GET/POST /api/hub/projects — the /hub frontend's project registry.
//
//   GET  /api/hub/projects
//   → 200 { projects: ProjectEntry[], defaultWorkspace: string }
//
//   POST /api/hub/projects  body: { action: "add" | "remove", path: string }
//   → 200 { ok: true, projects: ProjectEntry[] }
//   → 400/404 { error: "…" }
//
// "add" registers an EXISTING directory (the browse flow already validated it);
// it reuses the shared projects store so the IDE's switcher sees the same list
// and the terminal WS workspace allowlist (server.ts / workspace.ts) accepts
// the path immediately. "remove" only unregisters — files are never touched.

import { existsSync, statSync } from "node:fs";
import { resolve } from "node:path";
import { getSession } from "@/lib/auth";
import { loadProjects, removeProject, touchProject } from "@/lib/projects";
import { getBridge } from "@/server/core-bridge";

export const dynamic = "force-dynamic";
export const runtime = "nodejs";

export async function GET(req: Request) {
  if (!(await getSession(req.headers)))
    return Response.json({ error: "unauthorized" }, { status: 401 });
  return Response.json({
    projects: loadProjects(),
    defaultWorkspace: getBridge().getDefaultWorkspace(),
  });
}

export async function POST(req: Request) {
  if (!(await getSession(req.headers)))
    return Response.json({ error: "unauthorized" }, { status: 401 });

  let body: Record<string, unknown>;
  try {
    body = await req.json();
  } catch {
    return Response.json({ error: "invalid body" }, { status: 400 });
  }

  const action = typeof body.action === "string" ? body.action : "";
  const rawPath = typeof body.path === "string" ? body.path.trim() : "";
  if (!rawPath) return Response.json({ error: "missing path" }, { status: 400 });
  const abs = resolve(rawPath);

  if (action === "add") {
    // Must be an existing directory. The terminal WS + git routes both refuse
    // unallowlisted workspaces, so this registration IS the capability grant —
    // validate strictly before granting it.
    try {
      if (!existsSync(abs)) {
        return Response.json({ error: "path not found" }, { status: 404 });
      }
      if (!statSync(abs).isDirectory()) {
        return Response.json({ error: "path is not a directory" }, { status: 400 });
      }
    } catch (e) {
      return Response.json(
        { error: `cannot open path: ${(e as Error).message}` },
        { status: 400 },
      );
    }
    return Response.json({ ok: true, projects: touchProject(abs) });
  }

  if (action === "remove") {
    return Response.json({ ok: true, projects: removeProject(abs) });
  }

  return Response.json({ error: "unknown action" }, { status: 400 });
}
