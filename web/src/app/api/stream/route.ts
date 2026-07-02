// GET /api/stream — a Server-Sent Events stream of raw core events for ONE
// session.
//
// Query params:
//   session=<absolute session file>   the session to view (live, in-flight
//                                      tool calls / streaming included)
//   workspace=<workspace dir>          needed to start a never-seen session
//
// On connect: ensure the target session's core is running (starting it fresh —
// loading its history from disk — if it isn't already live), atomically capture
// a snapshot of that session's AgentState and subscribe to its live events, then
// emit the snapshot followed by every live core event as `data: <json>\n\n`.
// Sessions keep running when the client disconnects, so returning to a session
// (or switching to another and back) shows it still live. A 15s keepalive comment
// prevents proxies from closing the idle connection.

import { getBridge } from "@/server/core-bridge";
import { authorized } from "@/lib/auth";
import type { CoreEvent, ServerToClient } from "@/lib/types";

export const dynamic = "force-dynamic";
export const runtime = "nodejs";

export async function GET(req: Request) {
  if (!authorized(req)) return new Response("unauthorized", { status: 401 });
  const bridge = getBridge();

  const url = new URL(req.url);
  const session = url.searchParams.get("session") ?? undefined;
  const workspace = url.searchParams.get("workspace") ?? undefined;

  let live;
  try {
    // Ensure the session's core is running. If `session` is omitted, fall back to
    // the default workspace's most-recent session (the initial connection).
    live = await bridge.ensure(workspace, session);
  } catch (err: any) {
    return new Response(
      JSON.stringify({ error: err?.message ?? "failed to start umans-core" }),
      { status: 502, headers: { "Content-Type": "application/json" } },
    );
  }

  const encoder = new TextEncoder();
  let unsubscribe: (() => void) | null = null;
  let keepalive: ReturnType<typeof setInterval> | null = null;

  const stream = new ReadableStream<Uint8Array>({
    start(controller) {
      let closed = false;
      const safeEnqueue = (chunk: string) => {
        if (closed) return;
        try {
          controller.enqueue(encoder.encode(chunk));
        } catch {
          closed = true;
          unsubscribe?.();
          if (keepalive) clearInterval(keepalive);
        }
      };
      const send = (obj: ServerToClient) =>
        safeEnqueue(`data: ${JSON.stringify(obj)}\n\n`);

      // 1) Atomically subscribe + snapshot (no event can slip between them).
      const { snapshot, unsubscribe: unsub } = live.subscribe((ev: CoreEvent) =>
        send(ev as unknown as ServerToClient),
      );
      unsubscribe = unsub;

      // 2) Hydrate the full current state of THIS session, then live events flow.
      send({ type: "_snapshot", state: snapshot });

      // 3) Keepalive so intermediaries don't time the idle connection out.
      keepalive = setInterval(() => safeEnqueue(": keepalive\n\n"), 15000);
    },
    cancel() {
      if (keepalive) clearInterval(keepalive);
      unsubscribe?.();
    },
  });

  return new Response(stream, {
    headers: {
      "Content-Type": "text/event-stream; charset=utf-8",
      "Cache-Control": "no-cache, no-transform",
      Connection: "keep-alive",
      "X-Accel-Buffering": "no",
    },
  });
}
