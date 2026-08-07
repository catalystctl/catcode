import { getSession } from "@/lib/auth";
import { startSelfUpdate, updateLockActive, webInstallDetected } from "@/lib/self-update";

export const dynamic = "force-dynamic";
export const runtime = "nodejs";

/** POST /api/version/update — start CLI (+ web) self-update in the background. */
export async function POST(req: Request) {
  if (!(await getSession(req.headers))) {
    return Response.json({ ok: false, error: "unauthorized" }, { status: 401 });
  }

  const result = await startSelfUpdate();
  if (!result.ok) {
    return Response.json(result, { status: 500 });
  }
  return Response.json(
    {
      ...result,
      webInstallDetected: webInstallDetected(),
      updating: updateLockActive(),
    },
    { headers: { "Cache-Control": "no-store" } },
  );
}
