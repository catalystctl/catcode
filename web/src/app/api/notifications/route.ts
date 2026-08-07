// GET/POST /api/notifications — the web-only desktop-notification preference.
//
// The OS permission itself is a browser concept (Notification.permission) and
// is requested client-side; this route only persists the user's *opt-in toggle*
// (`desktop_notifications` in settings.json, shared with the TUI which ignores
// the key). The client re-evaluates enablement as `preference && permission`.

import { getSession } from "@/lib/auth";
import { loadSettings, saveDesktopNotifications } from "@/server/settings-file";

export const dynamic = "force-dynamic";
export const runtime = "nodejs";

export async function GET(req: Request) {
  if (!(await getSession(req.headers)))
    return new Response("unauthorized", { status: 401 });
  const s = loadSettings();
  return Response.json({ desktopNotifications: s.desktopNotifications ?? false });
}

export async function POST(req: Request) {
  if (!(await getSession(req.headers)))
    return new Response("unauthorized", { status: 401 });
  const body = (await req.json().catch(() => ({}))) as {
    desktopNotifications?: boolean;
  };
  const enabled = !!body.desktopNotifications;
  saveDesktopNotifications(enabled);
  return Response.json({ desktopNotifications: enabled });
}
