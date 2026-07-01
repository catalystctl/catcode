// POST /api/command — forward a core command to the umans-core stdin.
//
// The body is a raw core command object ({ type: "send", ... }). The response
// arrives asynchronously over the SSE stream (e.g. list_sessions → sessions),
// so this endpoint just acknowledges acceptance. The bridge records user messages
// for send/steer into its snapshot state.

import { getBridge } from "@/server/core-bridge";
import { authorized } from "@/lib/auth";
import type { CoreCommand } from "@/lib/types";

export const dynamic = "force-dynamic";
export const runtime = "nodejs";

export async function POST(req: Request) {
  if (!authorized(req)) return Response.json({ ok: false, error: "unauthorized" }, { status: 401 });
  const bridge = getBridge();
  try {
    await bridge.ensure();
  } catch (err: any) {
    return Response.json(
      { ok: false, error: err?.message ?? "failed to start umans-core" },
      { status: 502 },
    );
  }

  let cmd: CoreCommand;
  try {
    cmd = (await req.json()) as CoreCommand;
  } catch {
    return Response.json({ ok: false, error: "invalid JSON body" }, { status: 400 });
  }
  if (!cmd || typeof cmd.type !== "string") {
    return Response.json({ ok: false, error: "missing command type" }, { status: 400 });
  }

  try {
    bridge.send(cmd);
    return Response.json({ ok: true });
  } catch (err: any) {
    return Response.json({ ok: false, error: err?.message ?? "send failed" }, { status: 500 });
  }
}
