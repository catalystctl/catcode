// Next middleware — stamps an httpOnly umans_token cookie on page navigations
// when UMANS_WEB_TOKEN is configured, so the browser auto-sends it to /api/*
// routes (including SSE EventSource, which cannot set Authorization headers).

import { NextResponse } from "next/server";
import type { NextRequest } from "next/server";

export function middleware(req: NextRequest) {
  const token = process.env.UMANS_WEB_TOKEN;
  if (!token) return NextResponse.next();
  // Only stamp the cookie on page navigations (not /api/*), idempotently.
  if (req.nextUrl.pathname.startsWith("/api")) return NextResponse.next();
  const res = NextResponse.next();
  res.cookies.set("umans_token", token, { httpOnly: true, sameSite: "lax", path: "/" });
  return res;
}

export const config = {
  matcher: ["/((?!_next/static|_next/image|favicon.ico).*)"],
};
