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
