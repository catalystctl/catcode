"use client";

// Cross-session notification surfaces — the OS/tab layer.
//
// State (the feed + live-session map) lives in useAgent/reducer; this module
// only renders the two side-effect channels that exist OUTSIDE the app chrome:
//   • the browser tab badge — document.title prefix ("(2) ⚠ …") + a favicon dot,
//     no permission needed, works even when desktop notifications are denied;
//   • OS desktop notifications via the Web Notifications API — opt-in, gated on
//     Notification.permission === "granted".
//
// Everything is guarded for SSR (Next.js renders on the server) and wrapped so a
// failure here never breaks the app — the in-app feed is the source of truth.

import type { AttentionKind } from "./types";

// Whether the user has opted into desktop notifications AND granted OS
// permission. Set by the Settings toggle / on-mount load; read by useAgent when
// deciding whether to fire an OS notification for a new feed item. Lives at
// module scope so the Settings component and the useAgent effect can share it
// without threading state through the agent return.
let desktopEnabled = false;
export function isDesktopEnabled(): boolean {
  return desktopEnabled;
}
export function setDesktopEnabled(on: boolean): void {
  desktopEnabled = on;
}

// ── Tab badge ───────────────────────────────────────────────────────────────

let baseTitle: string | null = null;
let originalFavicon: string | null = null;
let badgeActive = false;

function findFavicon(): HTMLLinkElement | null {
  return (
    document.querySelector<HTMLLinkElement>('link[rel~="icon"]') ??
    document.querySelector<HTMLLinkElement>('link[rel="shortcut icon"]')
  );
}

function rememberOriginalFavicon(): void {
  if (originalFavicon !== null) return;
  const el = findFavicon();
  originalFavicon = el?.getAttribute("href") ?? "/favicon.ico";
}

/** A 16×16 dark square with a small colored dot — a recognizable "unread"
 *  favicon. Falls back to the original favicon if canvas is unavailable. */
function dotFavicon(color: string): string {
  try {
    const c = document.createElement("canvas");
    c.width = 16;
    c.height = 16;
    const ctx = c.getContext("2d");
    if (!ctx) return originalFavicon ?? "/favicon.ico";
    ctx.fillStyle = "#0b0e14";
    ctx.fillRect(0, 0, 16, 16);
    ctx.fillStyle = color;
    ctx.beginPath();
    ctx.arc(12, 12, 4.5, 0, Math.PI * 2);
    ctx.fill();
    return c.toDataURL("image/png");
  } catch {
    return originalFavicon ?? "/favicon.ico";
  }
}

function applyFavicon(href: string): void {
  let el = findFavicon();
  if (!el) {
    el = document.createElement("link");
    el.rel = "icon";
    document.head.appendChild(el);
  }
  el.setAttribute("href", href);
}

/**
 * Reflect the number of unread cross-session notifications in the browser tab.
 * `hasAttention` (a background session is blocked on the user) is shown with a
 * warning glyph + red dot; a plain finished turn uses a neutral count + blue dot.
 * Call with 0 to restore the resting title/favicon.
 */
export function setTabBadge(unreadCount: number, hasAttention: boolean): void {
  if (typeof document === "undefined") return;
  if (baseTitle === null) {
    baseTitle = document.title || "Catalyst Code";
    rememberOriginalFavicon();
  }
  if (unreadCount <= 0) {
    document.title = baseTitle;
    if (badgeActive) {
      applyFavicon(originalFavicon ?? "/favicon.ico");
      badgeActive = false;
    }
    return;
  }
  document.title = `${hasAttention ? "⚠ " : ""}(${unreadCount}) ${baseTitle}`;
  applyFavicon(dotFavicon(hasAttention ? "#f04438" : "#3b82f6"));
  badgeActive = true;
}

// ── Desktop notifications ────────────────────────────────────────────────────

export type DesktopPermission = "default" | "granted" | "denied";

export function desktopPermission(): DesktopPermission {
  if (typeof window === "undefined" || !("Notification" in window)) return "denied";
  return Notification.permission;
}

export async function requestDesktopPermission(): Promise<DesktopPermission> {
  if (typeof window === "undefined" || !("Notification" in window)) return "denied";
  if (Notification.permission === "granted") return "granted";
  try {
    // Older Safari takes a callback; the promise form is universal elsewhere.
    const result = await Notification.requestPermission();
    return result;
  } catch {
    return Notification.permission;
  }
}

export interface DesktopNotifyOpts {
  title: string;
  body: string;
  /** Dedup key — a new notification with the same tag replaces a prior one, so
   *  repeated approvals for one session coalesce instead of stacking. */
  tag?: string;
  /** Keep the notification on screen until the user interacts (blocking items). */
  requireInteraction?: boolean;
  onClick?: () => void;
}

/** Show an OS desktop notification if the user opted in (permission granted).
 *  Silently no-ops otherwise — the in-app feed + tab badge still fire. */
export function desktopNotify(opts: DesktopNotifyOpts): void {
  if (typeof window === "undefined" || !("Notification" in window)) return;
  if (Notification.permission !== "granted") return;
  try {
    const n = new Notification(opts.title, {
      body: opts.body,
      tag: opts.tag,
      requireInteraction: opts.requireInteraction,
    });
    if (opts.onClick) {
      n.onclick = () => {
        window.focus();
        opts.onClick?.();
        n.close();
      };
    }
    // Auto-close non-sticky notifications after 6s (some platforms hold them).
    if (!opts.requireInteraction) {
      setTimeout(() => {
        try {
          n.close();
        } catch {
          /* already gone */
        }
      }, 6000);
    }
  } catch {
    /* OS-level notifications disabled — ignore */
  }
}

// ── Helpers ─────────────────────────────────────────────────────────────────

/** Whether the tab is currently visible to the user. */
export function tabVisible(): boolean {
  if (typeof document === "undefined") return true;
  return document.visibilityState === "visible";
}

/** Human label for an attention kind, for desktop notification bodies. */
export function attentionLabel(kind: AttentionKind | undefined): string {
  switch (kind) {
    case "approval":
      return "needs tool approval";
    case "ask":
      return "asked a question";
    case "sudo":
      return "needs sudo approval";
    case "intercom":
      return "a subagent needs a decision";
    case "oauth":
      return "needs an OAuth code";
    default:
      return "needs your attention";
  }
}
