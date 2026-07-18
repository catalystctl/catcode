// Catch-all reverse proxy for loopback dev servers so a remote browser can
// preview http://localhost:<port>/… through the Catalyst web origin.
//
//   GET|HEAD|POST|… /api/dev-proxy/<port>/[...path]
//   → http://127.0.0.1:<port>/[...path]  (fallback http://[::1]:<port>/…)
//
// Auth-gated. SSRF-safe: upstream host is always loopback. HTML responses get
// a relative <base href> + inspect/history bootstrap so SPA assets and client
// routers stay on the public host under /api/dev-proxy/<port>.
//
// Auth headers: forward Authorization (proxied app JWTs) and non-Catalyst
// cookies; strip better-auth cookies. Rewrite upstream Set-Cookie Path onto
// the proxy prefix so sessions do not attach to the Catalyst host root.
//
// Critical behind reverse proxies: do NOT forward X-Forwarded-* / CF-* headers
// to the upstream — Vite/webpack would emit absolute asset URLs on the public
// hostname (missing /api/dev-proxy) or with the wrong port, which Cloudflare
// surfaces as a Host error.
//
// IPv4 vs IPv6: many Node/Vite/Astro servers bind `localhost` as [::1] only.
// Always try 127.0.0.1 then [::1].

import { getSession } from "@/lib/auth";
import {
  filterCookiesForUpstream,
  loopbackUpstreams,
  parseProxyPort,
  proxyBaseHref,
  rewriteLinkHeader,
  rewriteLoopbackLocation,
  rewriteLoopbackUrlsInHtml,
  rewriteRootAbsoluteUrlsInCss,
  rewriteRootAbsoluteUrlsInHtml,
  rewriteRootAbsoluteUrlsInScript,
  rewriteSetCookieForProxy,
  upstreamUnreachableHtml,
} from "@/lib/preview-proxy";
import { injectPreviewHelpers } from "@/server/preview-inject";

export const dynamic = "force-dynamic";
export const runtime = "nodejs";

const HOP_BY_HOP = new Set([
  "connection",
  "keep-alive",
  "proxy-authenticate",
  "proxy-authorization",
  "te",
  "trailers",
  "transfer-encoding",
  "upgrade",
  "host",
  "content-length",
]);

/** Headers that must never reach the upstream (Catalyst auth + reverse-proxy hints). */
function shouldDropRequestHeader(name: string): boolean {
  const lower = name.toLowerCase();
  if (HOP_BY_HOP.has(lower)) return true;
  // Cookie is filtered separately (forward app cookies, strip better-auth).
  if (lower === "cookie") return true;
  // Authorization (Bearer) is forwarded — proxied SPAs use it for their own JWTs.
  // Catalyst session auth is cookie-based, so this does not leak host credentials.
  if (lower === "origin" || lower === "referer") return true;
  if (lower === "forwarded") return true;
  if (lower.startsWith("x-forwarded-")) return true;
  if (lower.startsWith("cf-")) return true;
  if (lower === "true-client-ip" || lower === "cdn-loop") return true;
  return false;
}

type RouteCtx = { params: Promise<{ port: string; path?: string[] }> };

async function handle(req: Request, ctx: RouteCtx): Promise<Response> {
  if (!(await getSession(req.headers))) {
    return Response.json({ error: "unauthorized" }, { status: 401 });
  }

  const { port: portRaw, path: pathParts } = await ctx.params;
  const port = parseProxyPort(portRaw);
  if (port == null) {
    return Response.json({ error: "invalid port" }, { status: 400 });
  }

  const incoming = new URL(req.url);
  // pathParts from Next are decoded; re-encode each segment for the upstream URL.
  const safePath =
    pathParts && pathParts.length > 0
      ? `/${pathParts.map((p) => encodeURIComponent(p)).join("/")}`
      : "/";

  const pathWithQuery = `${safePath}${incoming.search}`;
  const candidates = loopbackUpstreams(port, pathWithQuery);

  const baseHeaders = new Headers();
  req.headers.forEach((value, key) => {
    if (shouldDropRequestHeader(key)) return;
    baseHeaders.set(key, value);
  });
  // Forward proxied-app cookies (e.g. hd_session) but never Catalyst better-auth.
  const upstreamCookie = filterCookiesForUpstream(req.headers.get("cookie"));
  if (upstreamCookie) baseHeaders.set("cookie", upstreamCookie);
  // Prefer identity encoding so we can rewrite HTML; upstream may still compress.
  baseHeaders.set("accept-encoding", "identity");

  const method = req.method.toUpperCase();
  const hasBody = method !== "GET" && method !== "HEAD";
  // Buffer the body so we can retry IPv6 after an IPv4 connection failure.
  const bodyBuf = hasBody ? await req.arrayBuffer() : undefined;

  let upstream: Response | null = null;
  const errors: string[] = [];
  for (const candidate of candidates) {
    try {
      const headers = new Headers(baseHeaders);
      headers.set("host", candidate.host);
      const init: RequestInit = {
        method,
        headers,
        redirect: "manual",
      };
      if (bodyBuf) init.body = bodyBuf;
      upstream = await fetch(candidate.url, init);
      break;
    } catch (e: unknown) {
      const msg = e instanceof Error ? e.message : "unreachable";
      errors.push(`${candidate.label}: ${msg}`);
    }
  }

  if (!upstream) {
    const detail = errors.join("; ") || "upstream unreachable";
    console.warn(`[dev-proxy] cannot reach port ${port}${safePath}: ${detail}`);
    // Do NOT return HTTP 502 — Cloudflare replaces origin 502s with its own
    // "Host Error" page. Serve a 200 HTML document the Preview iframe can show.
    return new Response(upstreamUnreachableHtml(port, detail), {
      status: 200,
      headers: {
        "Content-Type": "text/html; charset=utf-8",
        "Cache-Control": "no-store",
        "X-Catcode-Dev-Proxy": "upstream-unreachable",
      },
    });
  }

  const outHeaders = new Headers();
  upstream.headers.forEach((value, key) => {
    const lower = key.toLowerCase();
    if (HOP_BY_HOP.has(lower)) return;
    if (lower === "content-security-policy") return; // allow our inject + parent postMessage
    if (lower === "x-frame-options") return; // must embed in Preview iframe
    if (lower === "content-encoding") return; // we asked for identity / may rewrite body
    if (lower === "set-cookie") return; // handled via getSetCookie() below
    outHeaders.set(key, value);
  });
  outHeaders.set("Cache-Control", "no-store");
  outHeaders.set("X-Content-Type-Options", "nosniff");

  // Scope upstream session cookies to the proxy prefix (Path=/ would otherwise
  // attach to the Catalyst host and leak across apps).
  const setCookies =
    typeof upstream.headers.getSetCookie === "function"
      ? upstream.headers.getSetCookie()
      : [];
  if (setCookies.length > 0) {
    for (const raw of setCookies) {
      outHeaders.append("Set-Cookie", rewriteSetCookieForProxy(raw, port));
    }
  } else {
    const single = upstream.headers.get("set-cookie");
    if (single) {
      outHeaders.append("Set-Cookie", rewriteSetCookieForProxy(single, port));
    }
  }

  const location = upstream.headers.get("location");
  if (location) {
    // Relative Location keeps the browser on the Cloudflare/public host.
    outHeaders.set("Location", rewriteLoopbackLocation(location, port));
  }
  const link = upstream.headers.get("link");
  if (link) {
    outHeaders.set("Link", rewriteLinkHeader(link, port));
  }

  const contentType = (upstream.headers.get("content-type") || "").toLowerCase();
  const isHtml = contentType.includes("text/html");
  const isJs =
    contentType.includes("javascript") ||
    contentType.includes("ecmascript") ||
    contentType.includes("typescript") ||
    /\.(?:m?js|ts|tsx|jsx|mjs|cjs)(?:\?|$)/i.test(safePath);
  const isCss = contentType.includes("text/css") || /\.css(?:\?|$)/i.test(safePath);

  if (req.method === "HEAD" || !upstream.body) {
    return new Response(upstream.body, {
      status: upstream.status,
      statusText: upstream.statusText,
      headers: outHeaders,
    });
  }

  // Rewrite JS/CSS module graphs so root-absolute imports stay under the proxy.
  if (!isHtml && (isJs || isCss)) {
    const text = await upstream.text();
    const rewritten = isCss
      ? rewriteRootAbsoluteUrlsInCss(text, port)
      : rewriteRootAbsoluteUrlsInScript(text, port);
    outHeaders.delete("content-length");
    return new Response(rewritten, {
      status: upstream.status,
      statusText: upstream.statusText,
      headers: outHeaders,
    });
  }

  if (!isHtml) {
    return new Response(upstream.body, {
      status: upstream.status,
      statusText: upstream.statusText,
      headers: outHeaders,
    });
  }

  const html = await upstream.text();
  const basePath =
    pathParts && pathParts.length > 0
      ? `/api/dev-proxy/${port}/${pathParts.map(encodeURIComponent).join("/")}`
      : `/api/dev-proxy/${port}`;
  // Relative base — never use req.url origin (often http://127.0.0.1:49283 behind CF).
  const baseHref = proxyBaseHref(basePath);
  const pathPrefix = `/api/dev-proxy/${port}`;

  const withLoopbackRewrites = rewriteLoopbackUrlsInHtml(html);
  const withRootRewrites = rewriteRootAbsoluteUrlsInHtml(withLoopbackRewrites, port);
  const rewritten = injectPreviewHelpers(withRootRewrites, {
    baseHref,
    pathPrefix,
    inspect: true,
  });
  outHeaders.delete("content-length");
  outHeaders.set(
    "Content-Type",
    contentType.includes("charset") ? contentType : "text/html; charset=utf-8",
  );

  return new Response(rewritten, {
    status: upstream.status,
    statusText: upstream.statusText,
    headers: outHeaders,
  });
}

export const GET = handle;
export const HEAD = handle;
export const POST = handle;
export const PUT = handle;
export const PATCH = handle;
export const DELETE = handle;
export const OPTIONS = handle;
