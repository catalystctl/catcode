// Auth gate for the web API routes. Opt-in: when UMANS_WEB_TOKEN is set, requests
// must carry it as a Bearer header or an umans_token cookie (set by middleware).
// When unset, all requests are allowed (localhost dev convenience).

export function authorized(req: Request): boolean {
  const token = process.env.UMANS_WEB_TOKEN;
  if (!token) return true; // unset => localhost dev, allow all
  const auth = req.headers.get("authorization") ?? "";
  if (auth === `Bearer ${token}`) return true;
  const cookie = req.headers.get("cookie") ?? "";
  return cookie.split(";").some((c) => c.trim() === `umans_token=${token}`);
}
