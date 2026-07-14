// web/src/app/api/terminal/route.ts
//
// The real terminal transport is a WebSocket upgraded by the custom server
// (web/src/server/server.ts) — Next app-router route handlers CANNOT upgrade
// to WebSocket. This route exists only so the path is documented and any
// stray non-WS HTTP hit gets a clear "upgrade required" response instead of a
// 404. Auth is still enforced (parity with every other route).
import { getSession } from "@/lib/auth";

export const dynamic = "force-dynamic";
export const runtime = "nodejs";

export async function GET(req: Request) {
  if (!(await getSession(req.headers)))
    return Response.json({ error: "unauthorized" }, { status: 401 });
  // The /api/terminal endpoint speaks WebSocket, not HTTP. Connect via `ws://`
  // (or `wss://`) from the terminal client; see web/src/components/ide/terminal.tsx.
  return new Response("Upgrade Required: use a WebSocket connection to /api/terminal", {
    status: 426,
    headers: { "Upgrade": "websocket", "Connection": "Upgrade" },
  });
}
