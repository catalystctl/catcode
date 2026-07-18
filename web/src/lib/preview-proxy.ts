/** Helpers for rewriting loopback preview URLs through `/api/dev-proxy`. */

/** postMessage `source` for Preview element-inspect (iframe ↔ parent). */
export const PREVIEW_INSPECT_SOURCE = "catcode-preview-inspect";

const LOOPBACK_HOSTS = new Set(["localhost", "127.0.0.1", "[::1]", "::1"]);

export type LoopbackTarget = {
  port: number;
  pathname: string;
  search: string;
  hash: string;
};

/** True when `host` is a loopback name we are willing to proxy. */
export function isLoopbackHost(host: string): boolean {
  const h = host.trim().toLowerCase();
  return LOOPBACK_HOSTS.has(h);
}

/**
 * Parse an absolute URL into a loopback proxy target, or null if it is not a
 * loopback http(s) URL with an explicit port (or default 80/443).
 */
export function parseLoopbackUrl(raw: string): LoopbackTarget | null {
  let u: URL;
  try {
    u = new URL(raw);
  } catch {
    return null;
  }
  if (u.protocol !== "http:" && u.protocol !== "https:") return null;
  if (!isLoopbackHost(u.hostname)) return null;
  const port = u.port
    ? Number(u.port)
    : u.protocol === "https:"
      ? 443
      : 80;
  if (!Number.isInteger(port) || port < 1 || port > 65535) return null;
  return {
    port,
    pathname: u.pathname || "/",
    search: u.search,
    hash: u.hash,
  };
}

/** Build the same-origin proxy path for a loopback URL (no hash). */
export function loopbackToProxyPath(target: LoopbackTarget): string {
  const path = target.pathname.startsWith("/") ? target.pathname : `/${target.pathname}`;
  // Next trailingSlash:false redirects `/api/dev-proxy/5173/` → `/api/dev-proxy/5173`.
  // Use no trailing slash for the document root so the iframe avoids a 308.
  const suffix = path === "/" ? "" : path;
  return `/api/dev-proxy/${target.port}${suffix}${target.search}`;
}

/**
 * If `url` is a loopback http(s) URL, return the same-origin `/api/dev-proxy/…`
 * path the iframe / new-tab should load. Otherwise return null (use URL as-is).
 */
export function toProxiedPreviewSrc(url: string): string | null {
  const target = parseLoopbackUrl(url.trim());
  if (!target) return null;
  return loopbackToProxyPath(target) + target.hash;
}

/** Validate a port string from the route param. */
export function parseProxyPort(raw: string): number | null {
  if (!/^\d{1,5}$/.test(raw)) return null;
  const port = Number(raw);
  if (!Number.isInteger(port) || port < 1 || port > 65535) return null;
  return port;
}

/** Loopback upstream candidates (IPv4 then IPv6 — some servers bind only ::1). */
export type LoopbackUpstream = {
  /** Fetch URL, e.g. http://127.0.0.1:4321/ or http://[::1]:4321/ */
  url: string;
  /** Host header value for the upstream request. */
  host: string;
  /** Short label for logs / error pages. */
  label: string;
};

/**
 * Build IPv4 + IPv6 loopback upstream URLs. Vite/Astro often listen on
 * `localhost` which on modern Linux is IPv6-only (`[::1]`), so dialing
 * `127.0.0.1` alone fails with ECONNREFUSED.
 */
export function loopbackUpstreams(port: number, pathWithQuery: string): LoopbackUpstream[] {
  const path = pathWithQuery.startsWith("/") ? pathWithQuery : `/${pathWithQuery}`;
  return [
    {
      url: `http://127.0.0.1:${port}${path}`,
      host: `localhost:${port}`,
      label: `127.0.0.1:${port}`,
    },
    {
      url: `http://[::1]:${port}${path}`,
      host: `localhost:${port}`,
      label: `[::1]:${port}`,
    },
  ];
}

/**
 * Relative `<base href>` for a proxied document path so asset URLs stay on the
 * public Cloudflare/tunnel host (never the Node bind address).
 */
export function proxyBaseHref(proxyPath: string): string {
  let path = proxyPath.startsWith("/") ? proxyPath : `/${proxyPath}`;
  if (!path.endsWith("/")) {
    const last = path.split("/").pop() || "";
    if (last.includes(".")) {
      path = path.slice(0, path.lastIndexOf("/") + 1);
    } else {
      path = `${path}/`;
    }
  }
  return path;
}

/**
 * Rewrite absolute loopback URLs embedded in HTML (Vite/webpack often emit
 * `http://localhost:5173/...`) into same-origin `/api/dev-proxy/...` paths.
 */
export function rewriteLoopbackUrlsInHtml(html: string): string {
  return html.replace(
    /https?:\/\/(?:localhost|127\.0\.0\.1|\[::1\])(?::(\d+))?(\/[^"'`\s<]*)?/gi,
    (full, portStr: string | undefined, pathPart: string | undefined) => {
      const isHttps = full.toLowerCase().startsWith("https:");
      const port = portStr ? Number(portStr) : isHttps ? 443 : 80;
      if (!Number.isInteger(port) || port < 1 || port > 65535) return full;
      const path = pathPart && pathPart.length > 0 ? pathPart : "/";
      return `/api/dev-proxy/${port}${path}`;
    },
  );
}

/**
 * Rewrite root-absolute paths (`/@vite/client`, `/src/main.tsx`) onto the proxy
 * prefix. `<base href>` does not affect paths that start with `/`.
 *
 * Also rewrites escaped JSON strings Next.js embeds in flight/script payloads:
 * `\"/_next/static/..."`.
 */
export function rewriteRootAbsoluteUrlsInHtml(html: string, port: number): string {
  const prefix = `/api/dev-proxy/${port}`;
  // HTML attributes: src="/...", href='/...'
  let out = html.replace(
    /\b(src|href|action|poster|data-src|data-href)=(["'])\/(?!\/|api\/dev-proxy\/)/gi,
    `$1=$2${prefix}/`,
  );
  // Quoted + JSON-escaped quoted root paths inside inline scripts / RSC payloads.
  // Matches "/foo", '/foo', \"/foo, \'/foo — but not //cdn or already-proxied.
  out = out.replace(
    /((?:\\["']|["']))\/(?!\/|api\/dev-proxy\/\d+)/g,
    `$1${prefix}/`,
  );
  // CSS url(/...)
  out = out.replace(
    /url\(\s*(['"]?)\/(?!\/|api\/dev-proxy\/)/gi,
    `url($1${prefix}/`,
  );
  return out;
}

/**
 * Rewrite root-absolute imports inside JS/TS module responses (Vite dep graph).
 * Without this, `from "/node_modules/..."` resolves to the Catalyst host root.
 */
export function rewriteRootAbsoluteUrlsInScript(text: string, port: number): string {
  const prefix = `/api/dev-proxy/${port}`;
  let out = rewriteLoopbackUrlsInHtml(text);
  // Broad quoted root paths (covers import/from/new URL/JSON).
  out = out.replace(
    /((?:\\["']|["']))\/(?!\/|api\/dev-proxy\/\d+)/g,
    `$1${prefix}/`,
  );
  return out;
}

/** Rewrite root-absolute urls inside CSS responses. */
export function rewriteRootAbsoluteUrlsInCss(text: string, port: number): string {
  const prefix = `/api/dev-proxy/${port}`;
  let out = rewriteLoopbackUrlsInHtml(text);
  out = out.replace(
    /url\(\s*(['"]?)\/(?!\/|api\/dev-proxy\/)/gi,
    `url($1${prefix}/`,
  );
  out = out.replace(
    /@import\s+(['"])\/(?!\/|api\/dev-proxy\/)/gi,
    `@import $1${prefix}/`,
  );
  return out;
}

/**
 * Rewrite `Link:` response headers (font/CSS preloads) onto the proxy prefix.
 * Without this the browser preloads `/_next/static/...` from the Catalyst host.
 */
export function rewriteLinkHeader(link: string, port: number): string {
  const prefix = `/api/dev-proxy/${port}`;
  return link.replace(
    /<(\/(?!\/|api\/dev-proxy\/\d+)[^>]+)>/g,
    `<${prefix}$1>`,
  );
}
export function rewriteLoopbackLocation(location: string, fallbackPort: number): string {
  try {
    const abs = new URL(location, `http://127.0.0.1:${fallbackPort}/`);
    if (!isLoopbackHost(abs.hostname)) {
      // Root-relative Location from upstream → keep under the proxy prefix.
      if (location.startsWith("/") && !location.startsWith("//")) {
        if (location.startsWith(`/api/dev-proxy/`)) return location;
        return `/api/dev-proxy/${fallbackPort}${location}`;
      }
      return abs.toString();
    }
    const portNum = abs.port ? Number(abs.port) : fallbackPort;
    const path = abs.pathname || "/";
    return `/api/dev-proxy/${portNum}${path}${abs.search}${abs.hash}`;
  } catch {
    return location;
  }
}

/**
 * Scope upstream `Set-Cookie` to the proxy prefix so session cookies do not
 * attach to the Catalyst host root (and collide with better-auth).
 *
 * `Path=/login` → `Path=/api/dev-proxy/<port>/login`
 * `Path=/` or missing Path → `Path=/api/dev-proxy/<port>`
 * Absolute Domain=localhost / 127.0.0.1 is stripped (cookie stays host-only).
 */
export function rewriteSetCookieForProxy(setCookie: string, port: number): string {
  const prefix = `/api/dev-proxy/${port}`;
  const parts = setCookie.split(";").map((p) => p.trim());
  if (parts.length === 0) return setCookie;

  let pathSeen = false;
  const out: string[] = [parts[0]];
  for (let i = 1; i < parts.length; i++) {
    const part = parts[i];
    const eq = part.indexOf("=");
    const name = (eq === -1 ? part : part.slice(0, eq)).trim().toLowerCase();
    const value = eq === -1 ? "" : part.slice(eq + 1).trim();
    if (name === "path") {
      pathSeen = true;
      const raw = value || "/";
      if (raw.startsWith(prefix)) {
        out.push(`Path=${raw}`);
      } else if (raw === "/" || raw === "") {
        out.push(`Path=${prefix}`);
      } else {
        const suffix = raw.startsWith("/") ? raw : `/${raw}`;
        out.push(`Path=${prefix}${suffix}`);
      }
      continue;
    }
    if (name === "domain") {
      const host = value.replace(/^\./, "").toLowerCase();
      if (isLoopbackHost(host) || host === "") continue; // host-only on Catalyst origin
      // Non-loopback Domain would be wrong on the public host — drop it.
      continue;
    }
    out.push(part);
  }
  if (!pathSeen) out.push(`Path=${prefix}`);
  return out.join("; ");
}

/** Cookie names that belong to Catalyst auth — never forward to upstream apps. */
const CATALYST_COOKIE_RE = /^(?:__secure-)?better-auth\./i;

/**
 * Forward proxied-app cookies upstream while stripping Catalyst session cookies.
 * Returns null when nothing remains to send.
 */
export function filterCookiesForUpstream(cookieHeader: string | null): string | null {
  if (!cookieHeader) return null;
  const kept = cookieHeader
    .split(";")
    .map((c) => c.trim())
    .filter(Boolean)
    .filter((c) => {
      const name = c.split("=", 1)[0]?.trim() ?? "";
      return name.length > 0 && !CATALYST_COOKIE_RE.test(name);
    });
  return kept.length > 0 ? kept.join("; ") : null;
}

/** Friendly HTML shown in the Preview iframe when nothing is listening. */
export function upstreamUnreachableHtml(port: number, detail: string): string {
  const safeDetail = detail
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;");
  return `<!doctype html>
<html lang="en">
<head>
  <meta charset="utf-8"/>
  <title>Preview: port ${port} unreachable</title>
  <style>
    body{font-family:ui-sans-serif,system-ui,sans-serif;background:#0b0d10;color:#c8cdd5;
      display:flex;align-items:center;justify-content:center;min-height:100vh;margin:0;padding:24px}
    .card{max-width:440px;border:1px solid #2a3038;border-radius:12px;padding:20px 22px;background:#12151a}
    h1{font-size:15px;margin:0 0 8px;color:#f3f4f6;font-weight:600}
    p{font-size:13px;line-height:1.5;margin:0 0 10px;color:#9aa3af}
    code{font-family:ui-monospace,SFMono-Regular,Menlo,monospace;font-size:12px;
      background:#1a1f26;padding:1px 6px;border-radius:4px;color:#e5e7eb}
    .hint{font-size:12px;color:#6b7280;margin:0}
  </style>
</head>
<body>
  <div class="card">
    <h1>Could not reach port ${port}</h1>
    <p>Tried <code>127.0.0.1:${port}</code> and <code>[::1]:${port}</code> on the Catalyst host — neither accepted the connection.</p>
    <p class="hint">Confirm the dev server is running and listening on localhost (check its bind address / <code>--host</code>).</p>
    <p class="hint">${safeDetail}</p>
  </div>
</body>
</html>`;
}
