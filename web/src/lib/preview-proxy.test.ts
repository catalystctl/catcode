import { describe, expect, test } from "bun:test";
import {
  filterCookiesForUpstream,
  isLoopbackHost,
  loopbackToProxyPath,
  loopbackUpstreams,
  parseLoopbackUrl,
  parseProxyPort,
  proxyBaseHref,
  rewriteLinkHeader,
  rewriteLoopbackLocation,
  rewriteLoopbackUrlsInHtml,
  rewriteRootAbsoluteUrlsInHtml,
  rewriteRootAbsoluteUrlsInScript,
  rewriteSetCookieForProxy,
  toProxiedPreviewSrc,
  upstreamUnreachableHtml,
} from "./preview-proxy";
import { injectPreviewHelpers } from "../server/preview-inject";

describe("preview-proxy", () => {
  test("recognizes loopback hosts", () => {
    expect(isLoopbackHost("localhost")).toBe(true);
    expect(isLoopbackHost("127.0.0.1")).toBe(true);
    expect(isLoopbackHost("::1")).toBe(true);
    expect(isLoopbackHost("[::1]")).toBe(true);
    expect(isLoopbackHost("example.com")).toBe(false);
    expect(isLoopbackHost("10.0.0.1")).toBe(false);
  });

  test("parses loopback URLs and rejects non-loopback", () => {
    const t = parseLoopbackUrl("http://localhost:5173/app?x=1#hash");
    expect(t).toEqual({
      port: 5173,
      pathname: "/app",
      search: "?x=1",
      hash: "#hash",
    });
    expect(parseLoopbackUrl("http://example.com:5173/")).toBeNull();
    expect(parseLoopbackUrl("ftp://localhost:21/")).toBeNull();
  });

  test("rewrites to /api/dev-proxy path", () => {
    expect(toProxiedPreviewSrc("http://127.0.0.1:3000/foo")).toBe(
      "/api/dev-proxy/3000/foo",
    );
    // Root URL must omit trailing slash (Next redirects …/5173/ → …/5173).
    expect(toProxiedPreviewSrc("http://localhost:5173/")).toBe(
      "/api/dev-proxy/5173",
    );
    expect(toProxiedPreviewSrc("https://example.com")).toBeNull();
  });

  test("loopbackToProxyPath preserves query", () => {
    const t = parseLoopbackUrl("http://localhost:8080/a/b?q=1")!;
    expect(loopbackToProxyPath(t)).toBe("/api/dev-proxy/8080/a/b?q=1");
  });

  test("parseProxyPort validates range", () => {
    expect(parseProxyPort("5173")).toBe(5173);
    expect(parseProxyPort("0")).toBeNull();
    expect(parseProxyPort("65536")).toBeNull();
    expect(parseProxyPort("abc")).toBeNull();
  });

  test("loopbackUpstreams tries IPv4 then IPv6", () => {
    const list = loopbackUpstreams(4321, "/");
    expect(list.map((c) => c.url)).toEqual([
      "http://127.0.0.1:4321/",
      "http://[::1]:4321/",
    ]);
    expect(list[0].host).toBe("localhost:4321");
  });

  test("proxyBaseHref stays relative (Cloudflare-safe)", () => {
    expect(proxyBaseHref("/api/dev-proxy/5173")).toBe("/api/dev-proxy/5173/");
    expect(proxyBaseHref("/api/dev-proxy/5173/")).toBe("/api/dev-proxy/5173/");
    expect(proxyBaseHref("/api/dev-proxy/5173/app/index.html")).toBe(
      "/api/dev-proxy/5173/app/",
    );
  });

  test("rewriteLoopbackLocation uses relative paths", () => {
    expect(rewriteLoopbackLocation("http://localhost:5173/dash", 5173)).toBe(
      "/api/dev-proxy/5173/dash",
    );
    expect(rewriteLoopbackLocation("/login", 3000)).toBe("/api/dev-proxy/3000/login");
    expect(rewriteLoopbackLocation("https://example.com/x", 3000)).toBe(
      "https://example.com/x",
    );
  });

  test("rewriteSetCookieForProxy scopes Path to the proxy prefix", () => {
    expect(
      rewriteSetCookieForProxy(
        "hd_session=abc; Path=/; HttpOnly; SameSite=Lax",
        8080,
      ),
    ).toBe("hd_session=abc; Path=/api/dev-proxy/8080; HttpOnly; SameSite=Lax");
    expect(
      rewriteSetCookieForProxy("tok=1; Path=/login; Domain=localhost", 3000),
    ).toBe("tok=1; Path=/api/dev-proxy/3000/login");
    expect(rewriteSetCookieForProxy("a=b; HttpOnly", 5173)).toBe(
      "a=b; HttpOnly; Path=/api/dev-proxy/5173",
    );
    expect(
      rewriteSetCookieForProxy("x=y; Path=/api/dev-proxy/5173/already", 5173),
    ).toBe("x=y; Path=/api/dev-proxy/5173/already");
  });

  test("filterCookiesForUpstream strips better-auth cookies", () => {
    expect(
      filterCookiesForUpstream(
        "better-auth.session_token=host; hd_session=app; foo=bar",
      ),
    ).toBe("hd_session=app; foo=bar");
    expect(
      filterCookiesForUpstream(
        "__Secure-better-auth.session_token=x; other=1",
      ),
    ).toBe("other=1");
    expect(filterCookiesForUpstream("better-auth.session_token=only")).toBeNull();
    expect(filterCookiesForUpstream(null)).toBeNull();
  });

  test("rewriteLoopbackUrlsInHtml rewrites vite-style absolute URLs", () => {
    const html =
      `<script src="http://localhost:5173/@vite/client"></script>` +
      `<link href="http://127.0.0.1:5173/src/main.tsx">`;
    const out = rewriteLoopbackUrlsInHtml(html);
    expect(out).toContain('src="/api/dev-proxy/5173/@vite/client"');
    expect(out).toContain('href="/api/dev-proxy/5173/src/main.tsx"');
    expect(out).not.toContain("http://localhost");
  });

  test("rewriteRootAbsoluteUrlsInHtml prefixes vite root paths", () => {
    const html =
      `<script src="/@vite/client"></script>` +
      `import x from "/@react-refresh";` +
      `<link href="/src/main.tsx">`;
    const out = rewriteRootAbsoluteUrlsInHtml(html, 5173);
    expect(out).toContain('src="/api/dev-proxy/5173/@vite/client"');
    expect(out).toContain('from "/api/dev-proxy/5173/@react-refresh"');
    expect(out).toContain('href="/api/dev-proxy/5173/src/main.tsx"');
  });

  test("rewriteRootAbsoluteUrlsInHtml rewrites Next.js escaped flight paths", () => {
    const html = `<script>self.__next_f.push([1,"\\"/_next/static/chunks/app.js\\""])</script><a href="/install">x</a>`;
    const out = rewriteRootAbsoluteUrlsInHtml(html, 3000);
    expect(out).toContain('\\"/api/dev-proxy/3000/_next/static/chunks/app.js\\"');
    expect(out).toContain('href="/api/dev-proxy/3000/install"');
  });

  test("rewriteLinkHeader prefixes preload paths", () => {
    const link =
      "</_next/static/media/a.woff2>; rel=preload; as=\"font\", </_next/static/chunks/b.css>; rel=preload";
    expect(rewriteLinkHeader(link, 3000)).toBe(
      "</api/dev-proxy/3000/_next/static/media/a.woff2>; rel=preload; as=\"font\", </api/dev-proxy/3000/_next/static/chunks/b.css>; rel=preload",
    );
  });

  test("rewriteRootAbsoluteUrlsInScript rewrites vite imports", () => {
    const src = `import "/@vite/client";\nfrom "/node_modules/x.js";\nnew URL("/src/main.ts", import.meta.url)`;
    const out = rewriteRootAbsoluteUrlsInScript(src, 4321);
    expect(out).toContain('"/api/dev-proxy/4321/@vite/client"');
    expect(out).toContain('"/api/dev-proxy/4321/node_modules/x.js"');
    expect(out).toContain('"/api/dev-proxy/4321/src/main.ts"');
  });

  test("upstreamUnreachableHtml mentions the port", () => {
    const html = upstreamUnreachableHtml(5174, "fetch failed");
    expect(html).toContain("5174");
    expect(html).toContain("fetch failed");
    expect(html).toContain("<!doctype html>");
  });
});

describe("injectPreviewHelpers", () => {
  test("injects base href and inspect script into a normal HTML document", () => {
    const html =
      "<!doctype html><html><head><title>t</title></head><body><h1>Hi</h1></body></html>";
    const out = injectPreviewHelpers(html, {
      baseHref: "/api/dev-proxy/5173/",
      inspect: true,
    });
    expect(out).toContain('<!--catcode-preview-base--><base href="/api/dev-proxy/5173/">');
    expect(out).toContain("<!--catcode-preview-inspect-->");
    expect(out).toContain("data-catcode-preview-inspect");
    expect(out).toContain("catcode-preview-inspect");
    expect(out.indexOf("catcode-preview-base")).toBeLessThan(out.indexOf("</head>"));
  });

  test("is idempotent on repeated inject", () => {
    const html = "<html><head></head><body></body></html>";
    const once = injectPreviewHelpers(html, {
      baseHref: "/api/dev-proxy/1/",
      inspect: true,
    });
    const twice = injectPreviewHelpers(once, {
      baseHref: "/api/dev-proxy/1/",
      inspect: true,
    });
    const scriptMarks = twice.split("<!--catcode-preview-inspect-->").length - 1;
    expect(scriptMarks).toBe(1);
    expect(twice.split("<!--catcode-preview-base-->").length - 1).toBe(1);
  });

  test("injects path-prefix fetch patch when pathPrefix is set", () => {
    const html = "<html><head></head><body></body></html>";
    const out = injectPreviewHelpers(html, {
      baseHref: "/api/dev-proxy/4321/",
      pathPrefix: "/api/dev-proxy/4321",
      inspect: false,
    });
    expect(out).toContain("<!--catcode-preview-prefix-->");
    expect(out).toContain("__catcodeProxyPrefix");
    expect(out).toContain("/api/dev-proxy/4321");
    // SPA hard-nav + History API must stay under the proxy prefix.
    expect(out).toContain("histProto.pushState");
    expect(out).toContain("locProto.assign");
    expect(out).toContain("stripPath");
    expect(out).toContain("WrappedWS");
  });
});
