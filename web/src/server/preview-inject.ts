/**
 * HTML helpers for Preview: inject `<base href>` (dev-proxy) and the element-
 * inspect bootstrap so the Preview panel can pick DOM nodes into chat.
 *
 * The inspect script is intentionally self-contained (no imports) so it can be
 * inlined into proxied / previewed HTML responses.
 */

import { PREVIEW_INSPECT_SOURCE } from "@/lib/preview-proxy";

export { PREVIEW_INSPECT_SOURCE };

/** Marker comments so re-injection is idempotent. */
const BASE_MARK = "<!--catcode-preview-base-->";
const SCRIPT_MARK = "<!--catcode-preview-inspect-->";

/**
 * Inline inspect bootstrap. Listens for parent `set-inspect` messages and
 * posts selected element descriptors back to the parent frame.
 */
export function previewInspectScript(): string {
  // Keep this as a plain string — it runs in the iframe, not the Next bundle.
  return `(function(){
  if (window.__catcodePreviewInspectInstalled) return;
  window.__catcodePreviewInspectInstalled = true;
  var SOURCE = ${JSON.stringify(PREVIEW_INSPECT_SOURCE)};
  var enabled = false;
  var overlay = null;
  var lastEl = null;
  var MAX_HTML = 3500;

  function ensureOverlay() {
    if (overlay) return overlay;
    overlay = document.createElement("div");
    overlay.setAttribute("data-catcode-inspect-overlay", "1");
    overlay.style.cssText = "position:fixed;pointer-events:none;z-index:2147483646;border:2px solid #3b82f6;background:rgba(59,130,246,0.12);display:none;box-sizing:border-box;";
    (document.documentElement || document.body).appendChild(overlay);
    return overlay;
  }

  function clearOverlay() {
    if (overlay) overlay.style.display = "none";
    lastEl = null;
  }

  function setEnabled(on) {
    enabled = !!on;
    if (!enabled) clearOverlay();
    try {
      document.documentElement.style.cursor = enabled ? "crosshair" : "";
    } catch (e) {}
  }

  function cssEscapeIdent(value) {
    if (typeof CSS !== "undefined" && CSS.escape) return CSS.escape(value);
    return String(value).replace(/[^a-zA-Z0-9_-]/g, "\\\\$&");
  }

  function selectorFor(el) {
    if (!el || el.nodeType !== 1) return "";
    if (el.id) return "#" + cssEscapeIdent(el.id);
    var parts = [];
    var cur = el;
    var depth = 0;
    while (cur && cur.nodeType === 1 && depth < 8) {
      var tag = (cur.tagName || "").toLowerCase();
      if (!tag || tag === "html" || tag === "body") {
        parts.unshift(tag || "body");
        break;
      }
      var part = tag;
      if (cur.classList && cur.classList.length) {
        var cls = Array.prototype.slice.call(cur.classList, 0, 3)
          .filter(Boolean)
          .map(function(c){ return "." + cssEscapeIdent(c); })
          .join("");
        part += cls;
      }
      var parent = cur.parentElement;
      if (parent) {
        var siblings = parent.children;
        var same = 0;
        var idx = 0;
        for (var i = 0; i < siblings.length; i++) {
          if (siblings[i].tagName === cur.tagName) {
            same++;
            if (siblings[i] === cur) idx = same;
          }
        }
        if (same > 1) part += ":nth-of-type(" + idx + ")";
      }
      parts.unshift(part);
      cur = parent;
      depth++;
    }
    return parts.join(" > ");
  }

  function highlight(el) {
    if (!el || el === document.documentElement || el === document.body) {
      clearOverlay();
      return;
    }
    lastEl = el;
    var box = ensureOverlay();
    var r = el.getBoundingClientRect();
    box.style.display = "block";
    box.style.top = Math.max(0, r.top) + "px";
    box.style.left = Math.max(0, r.left) + "px";
    box.style.width = Math.max(0, r.width) + "px";
    box.style.height = Math.max(0, r.height) + "px";
  }

  function truncateHtml(html) {
    if (!html) return "";
    if (html.length <= MAX_HTML) return html;
    return html.slice(0, MAX_HTML) + "\\n<!-- truncated -->";
  }

  function onMove(ev) {
    if (!enabled) return;
    var t = ev.target;
    if (!t || t === overlay || (t.getAttribute && t.getAttribute("data-catcode-inspect-overlay"))) return;
    highlight(t);
  }

  function onClick(ev) {
    if (!enabled) return;
    ev.preventDefault();
    ev.stopPropagation();
    var el = lastEl || ev.target;
    if (!el || el.nodeType !== 1) return;
    if (el === overlay || (el.getAttribute && el.getAttribute("data-catcode-inspect-overlay"))) return;
    var tag = (el.tagName || "").toLowerCase();
    var id = el.id || "";
    var classes = el.classList ? Array.prototype.slice.call(el.classList) : [];
    var html = "";
    try { html = truncateHtml(el.outerHTML || ""); } catch (e) { html = ""; }
    var rect = null;
    try {
      var r = el.getBoundingClientRect();
      rect = { x: r.x, y: r.y, width: r.width, height: r.height };
    } catch (e2) {}
    try {
      window.parent.postMessage({
        source: SOURCE,
        type: "element",
        payload: {
          tag: tag,
          id: id,
          classes: classes,
          selector: selectorFor(el),
          outerHTML: html,
          rect: rect,
          pageUrl: String(location.href || "")
        }
      }, "*");
    } catch (e3) {}
  }

  function onKey(ev) {
    if (!enabled) return;
    if (ev.key === "Escape") {
      setEnabled(false);
      try {
        window.parent.postMessage({ source: SOURCE, type: "inspect-off" }, "*");
      } catch (e) {}
    }
  }

  window.addEventListener("message", function(ev) {
    var data = ev.data;
    if (!data || data.source !== SOURCE) return;
    if (data.type === "set-inspect") setEnabled(!!data.enabled);
  });

  document.addEventListener("mousemove", onMove, true);
  document.addEventListener("click", onClick, true);
  document.addEventListener("keydown", onKey, true);
})();`;
}

export type InjectPreviewOptions = {
  /** When set, inject/replace `<base href="…">` for proxied asset resolution. */
  baseHref?: string;
  /** Inject the element-inspect bootstrap (default true). */
  inspect?: boolean;
  /**
   * Proxy path prefix (e.g. `/api/dev-proxy/4321`). Injects an early script that
   * rewrites fetch/XHR/history/location root-absolute URLs so SPA routers stay
   * under the proxy (and send cookies with allow-same-origin).
   */
  pathPrefix?: string;
};

const PREFIX_MARK = "<!--catcode-preview-prefix-->";

/**
 * Patch fetch/XHR/history/location/WebSocket so root-absolute `/foo` stays under
 * `/api/dev-proxy/<port>/foo`. Without history/location patches, SPAs that call
 * `location.assign('/login')` or `history.pushState(…, '/dashboard')` escape the
 * proxy and load the Catalyst host app instead.
 */
export function previewPathPrefixScript(prefix: string): string {
  return `(function(){
  if (window.__catcodeProxyPrefix) return;
  var PREFIX = ${JSON.stringify(prefix)};
  window.__catcodeProxyPrefix = PREFIX;

  var locProto = Location.prototype;
  var hrefDesc = Object.getOwnPropertyDescriptor(locProto, "href");
  var pathnameDesc = Object.getOwnPropertyDescriptor(locProto, "pathname");

  function realHref(loc) {
    try {
      return hrefDesc && hrefDesc.get ? hrefDesc.get.call(loc || window.location) : String((loc || window.location).href);
    } catch (e) {
      return String((loc || window.location).href);
    }
  }
  function realPathname(loc) {
    try {
      return pathnameDesc && pathnameDesc.get ? pathnameDesc.get.call(loc || window.location) : String((loc || window.location).pathname);
    } catch (e) {
      return String((loc || window.location).pathname);
    }
  }

  function stripPath(pathname) {
    if (pathname.indexOf(PREFIX) === 0) {
      var rest = pathname.slice(PREFIX.length);
      if (!rest) return "/";
      return rest.charAt(0) === "/" ? rest : "/" + rest;
    }
    return pathname;
  }

  function mine(loc) {
    return loc === window.location;
  }

  function fix(u) {
    if (u == null || typeof u !== "string") return u;
    try {
      var resolved = new URL(u, realHref(window.location));
      // ws/wss origins differ from http(s) even on the same host — still rewrite.
      var sameSite =
        resolved.origin === window.location.origin ||
        ((resolved.protocol === "ws:" || resolved.protocol === "wss:") &&
          resolved.host === window.location.host);
      if (!sameSite) return u;
      if (resolved.pathname.indexOf(PREFIX) === 0) {
        if (/^https?:\\/\\//i.test(u) || /^wss?:\\/\\//i.test(u)) return resolved.href;
        return resolved.pathname + resolved.search + resolved.hash;
      }
      if (resolved.pathname.charAt(0) === "/") {
        var next = PREFIX + resolved.pathname + resolved.search + resolved.hash;
        if (/^https?:\\/\\//i.test(u) || /^wss?:\\/\\//i.test(u)) {
          return resolved.protocol + "//" + resolved.host + next;
        }
        return next;
      }
    } catch (e) {}
    return u;
  }
  function fixInput(input) {
    if (typeof input === "string") return fix(input);
    try {
      if (typeof Request !== "undefined" && input instanceof Request) {
        var nu = fix(input.url);
        if (nu === input.url) return input;
        return new Request(nu, input);
      }
    } catch (e) {}
    return input;
  }

  var ofetch = window.fetch;
  window.fetch = function(input, init) {
    return ofetch.call(this, fixInput(input), init);
  };
  if (window.XMLHttpRequest) {
    var open = XMLHttpRequest.prototype.open;
    XMLHttpRequest.prototype.open = function() {
      var args = Array.prototype.slice.call(arguments);
      if (typeof args[1] === "string") args[1] = fix(args[1]);
      return open.apply(this, args);
    };
  }

  // History API — React Router / Vue Router / etc.
  var histProto = History.prototype;
  var origPush = histProto.pushState;
  var origReplace = histProto.replaceState;
  histProto.pushState = function(state, title, url) {
    if (this === window.history && url != null) url = fix(String(url));
    return origPush.call(this, state, title, url);
  };
  histProto.replaceState = function(state, title, url) {
    if (this === window.history && url != null) url = fix(String(url));
    return origReplace.call(this, state, title, url);
  };

  // Hard navigations — location.assign('/login') must stay under the proxy.
  var origAssign = locProto.assign;
  var origReplaceLoc = locProto.replace;
  locProto.assign = function(url) {
    if (mine(this)) url = fix(String(url));
    return origAssign.call(this, url);
  };
  locProto.replace = function(url) {
    if (mine(this)) url = fix(String(url));
    return origReplaceLoc.call(this, url);
  };

  // Present a root-relative pathname to SPAs (so routes match), while the real
  // URL stays under PREFIX. Only rewrite for this frame's location.
  if (pathnameDesc && pathnameDesc.get) {
    Object.defineProperty(locProto, "pathname", {
      configurable: true,
      enumerable: true,
      get: function() {
        var p = pathnameDesc.get.call(this);
        return mine(this) ? stripPath(p) : p;
      },
      set: pathnameDesc.set
        ? function(v) {
            if (!mine(this)) return pathnameDesc.set.call(this, v);
            var path = String(v || "/");
            if (path.charAt(0) !== "/") path = "/" + path;
            if (path.indexOf(PREFIX) !== 0) path = PREFIX + path;
            var cur = new URL(realHref(this));
            cur.pathname = path;
            if (hrefDesc && hrefDesc.set) hrefDesc.set.call(this, cur.href);
            else this.assign(cur.href);
          }
        : undefined,
    });
  }
  if (hrefDesc && hrefDesc.get) {
    Object.defineProperty(locProto, "href", {
      configurable: true,
      enumerable: true,
      get: function() {
        var real = hrefDesc.get.call(this);
        if (!mine(this)) return real;
        try {
          var u = new URL(real);
          u.pathname = stripPath(u.pathname);
          return u.href;
        } catch (e) {
          return real;
        }
      },
      set: function(v) {
        if (!mine(this)) {
          if (hrefDesc.set) hrefDesc.set.call(this, v);
          else origAssign.call(this, v);
          return;
        }
        var next = fix(String(v));
        if (hrefDesc.set) hrefDesc.set.call(this, next);
        else origAssign.call(this, next);
      },
    });
  }

  // WebSocket / EventSource root-absolute URLs.
  if (typeof window.WebSocket === "function") {
    var OrigWS = window.WebSocket;
    function WrappedWS(url, protocols) {
      var fixed = typeof url === "string" ? fix(url) : url;
      if (protocols === undefined) return new OrigWS(fixed);
      return new OrigWS(fixed, protocols);
    }
    WrappedWS.prototype = OrigWS.prototype;
    WrappedWS.CONNECTING = OrigWS.CONNECTING;
    WrappedWS.OPEN = OrigWS.OPEN;
    WrappedWS.CLOSING = OrigWS.CLOSING;
    WrappedWS.CLOSED = OrigWS.CLOSED;
    window.WebSocket = WrappedWS;
  }
  if (typeof window.EventSource === "function") {
    var OrigES = window.EventSource;
    function WrappedES(url, init) {
      var fixed = typeof url === "string" ? fix(url) : url;
      return init === undefined ? new OrigES(fixed) : new OrigES(fixed, init);
    }
    WrappedES.prototype = OrigES.prototype;
    WrappedES.CONNECTING = OrigES.CONNECTING;
    WrappedES.OPEN = OrigES.OPEN;
    WrappedES.CLOSED = OrigES.CLOSED;
    window.EventSource = WrappedES;
  }

  // Native <a href="/…"> ignores <base href>; rewrite before navigation.
  document.addEventListener(
    "click",
    function(ev) {
      if (ev.defaultPrevented || ev.button !== 0) return;
      if (ev.metaKey || ev.ctrlKey || ev.shiftKey || ev.altKey) return;
      var t = ev.target;
      var a = t && t.closest ? t.closest("a[href]") : null;
      if (!a || a.target && a.target !== "" && a.target !== "_self") return;
      if (a.hasAttribute("download")) return;
      var href = a.getAttribute("href");
      if (!href || href.charAt(0) === "#" || /^[a-z][a-z0-9+.-]*:/i.test(href)) return;
      try {
        var resolved = new URL(href, realHref(window.location));
        if (resolved.origin !== window.location.origin) return;
        if (resolved.pathname.indexOf(PREFIX) === 0) return;
        if (resolved.pathname.charAt(0) !== "/") return;
        ev.preventDefault();
        window.location.assign(resolved.pathname + resolved.search + resolved.hash);
      } catch (e) {}
    },
    true,
  );

  // Scroll-reveal (opacity:0 + IO) often never fires inside Preview iframes.
  // Force observed targets to count as intersecting, and unstick opacity:0
  // transition elements after a short delay.
  var OrigIO = window.IntersectionObserver;
  if (OrigIO) {
    window.IntersectionObserver = function(callback, options) {
      var observer = new OrigIO(function(entries, obs) {
        var forced = [];
        for (var i = 0; i < entries.length; i++) {
          var e = entries[i];
          forced.push({
            time: e.time,
            target: e.target,
            isIntersecting: true,
            intersectionRatio: 1,
            boundingClientRect: e.boundingClientRect,
            intersectionRect: e.intersectionRect,
            rootBounds: e.rootBounds,
          });
        }
        callback(forced, obs);
      }, options);
      var observe = observer.observe.bind(observer);
      observer.observe = function(target) {
        observe(target);
        setTimeout(function() {
          try {
            callback([{
              time: performance.now(),
              target: target,
              isIntersecting: true,
              intersectionRatio: 1,
              boundingClientRect: target.getBoundingClientRect(),
              intersectionRect: target.getBoundingClientRect(),
              rootBounds: null,
            }], observer);
          } catch (err) {}
        }, 0);
      };
      return observer;
    };
    window.IntersectionObserver.prototype = OrigIO.prototype;
  }
  function unstickReveal() {
    var all = document.querySelectorAll("body *");
    for (var i = 0; i < all.length; i++) {
      var el = all[i];
      var cs = window.getComputedStyle(el);
      if (cs.opacity === "0" && cs.transition && cs.transition.indexOf("opacity") !== -1) {
        el.style.setProperty("opacity", "1", "important");
        el.style.setProperty("transform", "none", "important");
      }
    }
  }
  if (document.readyState === "loading") {
    document.addEventListener("DOMContentLoaded", function() {
      setTimeout(unstickReveal, 50);
      setTimeout(unstickReveal, 400);
      setTimeout(unstickReveal, 1200);
    });
  } else {
    setTimeout(unstickReveal, 50);
    setTimeout(unstickReveal, 400);
    setTimeout(unstickReveal, 1200);
  }
})();`;
}

/**
 * Inject preview helpers into an HTML document string. Idempotent via markers.
 */
export function injectPreviewHelpers(html: string, opts: InjectPreviewOptions = {}): string {
  const inspect = opts.inspect !== false;
  let out = html;

  if (opts.baseHref) {
    const baseTag = `${BASE_MARK}<base href="${escapeAttr(opts.baseHref)}">`;
    if (out.includes(BASE_MARK)) {
      out = out.replace(
        /<!--catcode-preview-base-->[\s\S]*?<base\b[^>]*>/i,
        baseTag,
      );
    } else if (/<head\b[^>]*>/i.test(out)) {
      out = out.replace(/<head\b[^>]*>/i, (m) => `${m}\n${baseTag}`);
    } else {
      out = `${baseTag}\n${out}`;
    }
  }

  // Path-prefix patch must run BEFORE Vite/Astro modules so client routers
  // fetch under /api/dev-proxy/<port>/….
  if (opts.pathPrefix) {
    const prefixBlock = `${PREFIX_MARK}<script data-catcode-preview-prefix="1">${previewPathPrefixScript(opts.pathPrefix)}</script>`;
    if (!out.includes(PREFIX_MARK)) {
      if (/<head\b[^>]*>/i.test(out)) {
        out = out.replace(/<head\b[^>]*>/i, (m) => `${m}\n${prefixBlock}`);
      } else if (/<base\b[^>]*>/i.test(out)) {
        out = out.replace(/<base\b[^>]*>/i, (m) => `${m}\n${prefixBlock}`);
      } else {
        out = `${prefixBlock}\n${out}`;
      }
    }
  }

  if (inspect) {
    const scriptBlock = `${SCRIPT_MARK}<script data-catcode-preview-inspect="1">${previewInspectScript()}</script>`;
    if (out.includes(SCRIPT_MARK)) {
      // Already injected — leave existing script (avoids double listeners).
    } else if (/<\/head>/i.test(out)) {
      out = out.replace(/<\/head>/i, `${scriptBlock}\n</head>`);
    } else if (/<body\b[^>]*>/i.test(out)) {
      out = out.replace(/<body\b[^>]*>/i, (m) => `${m}\n${scriptBlock}`);
    } else {
      out = `${out}\n${scriptBlock}`;
    }
  }

  return out;
}

function escapeAttr(value: string): string {
  return value
    .replace(/&/g, "&amp;")
    .replace(/"/g, "&quot;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;");
}
