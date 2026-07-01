// GET /api/stream — a Server-Sent Events stream of raw core events.
//
// On connect: ensure the core is started, atomically capture a snapshot of the
// current AgentState and subscribe to live events, then emit the snapshot
// followed by every live core event as `data: <json>\n\n`. A 15s keepalive
// comment prevents proxies from closing the idle connection.

import { getBridge } from "@/server/core-bridge";
import { authorized } from "@/lib/auth";
import type { CoreEvent, ServerToClient } from "@/lib/types";

export const dynamic = "force-dynamic";
export const runtime = "nodejs";

export async function GET(req: Request) {
  if (!authorized(req)) return new Response("unauthorized", { status: 401 });
  const bridge = getBridge();
  try {
    await bridge.ensure();
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
      const { snapshot, unsubscribe: unsub } = bridge.subscribe((ev: CoreEvent) =>
        send(ev as unknown as ServerToClient),
      );
      unsubscribe = unsub;

      // 2) Hydrate the full current state, then live events flow via the sink.
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
