import { NextResponse, type NextRequest } from "next/server";

// Edge-runtime cookie check only (can't hit the DB here). Real session
// validation happens server-side in the API routes + page.tsx. This just
// bounces cookieless visitors to /login early for UX.
// Better Auth uses `better-auth.session_token`, or `__Secure-better-auth.session_token`
// when baseURL is https / useSecureCookies / production.
const PUBLIC = ["/setup", "/login", "/api/auth"];

function hasSessionCookie(req: NextRequest): boolean {
  return !!(
    req.cookies.get("better-auth.session_token") ||
    req.cookies.get("__Secure-better-auth.session_token")
  );
}

export function middleware(req: NextRequest) {
  const { pathname } = req.nextUrl;
  if (
    PUBLIC.some((p) => pathname === p || pathname.startsWith(p + "/")) ||
    pathname.startsWith("/_next") ||
    pathname === "/favicon.ico"
  ) {
    return NextResponse.next();
  }
  if (!hasSessionCookie(req)) {
    // Standalone Next builds construct req.nextUrl from their bind hostname
    // even when a reverse proxy supplies Host/X-Forwarded-Host. Prefer the
    // configured canonical origin so external users never get sent to the
    // loopback address.
    const url = process.env.CATCODE_WEB_ORIGIN
      ? new URL(req.nextUrl.pathname, process.env.CATCODE_WEB_ORIGIN)
      : req.nextUrl.clone();
    url.pathname = "/login";
    url.search = "";
    return NextResponse.redirect(url);
  }
  return NextResponse.next();
}

export const config = {
  matcher: ["/((?!_next/static|_next/image|favicon.ico).*)"],
};
