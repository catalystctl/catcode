// GET/PUT /api/hub/layout — account-scoped hub workspace layout.
//
//   GET  /api/hub/layout
//   → 200 { layout: HubPersistState, updatedAt: number }
//     layout is the default empty state when nothing has been saved yet.
//
//   PUT  /api/hub/layout  body: HubPersistState (or partial — sanitized)
//   → 200 { ok: true, layout: HubPersistState, updatedAt: number }
//
// The layout remembers open projects + last-viewed chat session per project
// so every signed-in device reopens the same chats and reattaches live cores.

import { getSession } from "@/lib/auth";
import {
  defaultHubState,
  loadHubLayout,
  saveHubLayout,
} from "@/server/hub-layout-store";

export const dynamic = "force-dynamic";
export const runtime = "nodejs";

export async function GET(req: Request) {
  const session = await getSession(req.headers);
  if (!session) return Response.json({ error: "unauthorized" }, { status: 401 });

  const loaded = loadHubLayout();
  return Response.json({
    layout: loaded?.state ?? defaultHubState(),
    updatedAt: loaded?.updatedAt ?? 0,
  });
}

export async function PUT(req: Request) {
  const session = await getSession(req.headers);
  if (!session) return Response.json({ error: "unauthorized" }, { status: 401 });

  let body: unknown;
  try {
    body = await req.json();
  } catch {
    return Response.json({ error: "invalid body" }, { status: 400 });
  }

  const userId = session.user.id;
  const saved = saveHubLayout(userId, body);
  return Response.json({ ok: true, layout: saved.state, updatedAt: saved.updatedAt });
}
