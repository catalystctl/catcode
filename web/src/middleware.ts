import { NextResponse, type NextRequest } from "next/server";

// Edge-runtime cookie check only (can't hit the DB here). Real session
// validation happens server-side in the API routes + page.tsx. This just
// bounces cookieless visitors to /login early for UX.
const SESSION_COOKIE = "better-auth.session_token";
const PUBLIC = ["/setup", "/login", "/api/auth"];

export function middleware(req: NextRequest) {
  const { pathname } = req.nextUrl;
  if (
    PUBLIC.some((p) => pathname === p || pathname.startsWith(p + "/")) ||
    pathname.startsWith("/_next") ||
    pathname === "/favicon.ico"
  ) {
    return NextResponse.next();
  }
  const hasSession = req.cookies.get(SESSION_COOKIE);
  if (!hasSession) {
    const url = req.nextUrl.clone();
    url.pathname = "/login";
    url.search = "";
    return NextResponse.redirect(url);
  }
  return NextResponse.next();
}

export const config = {
  matcher: ["/((?!_next/static|_next/image|favicon.ico).*)"],
};
