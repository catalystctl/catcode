import { getSession } from "@/lib/auth";
import { resolveVersionInfo } from "@/lib/version";

export const dynamic = "force-dynamic";
export const runtime = "nodejs";

/** GET /api/version — running git commit + up-to-date / out-of-date / uncommitted. */
export async function GET(req: Request) {
  if (!(await getSession(req.headers))) {
    return Response.json({ ok: false, error: "unauthorized" }, { status: 401 });
  }
  try {
    const info = await resolveVersionInfo(process.cwd());
    return Response.json({ ok: true, ...info }, {
      headers: { "Cache-Control": "no-store" },
    });
  } catch (err) {
    const message = err instanceof Error ? err.message : String(err);
    return Response.json({ ok: false, error: message }, { status: 500 });
  }
}
