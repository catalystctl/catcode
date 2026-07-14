#!/usr/bin/env node
/**
 * Mobile responsiveness audit for the Catalyst Code web UI.
 *
 * Credentials (optional, for authenticated IDE shell):
 *   AUDIT_EMAIL / AUDIT_PASSWORD from web/.env.local (gitignored)
 *   or the environment.
 *
 * Visits key routes at phone/tablet/desktop viewports and reports
 * horizontal overflow + mobile bottom-nav presence on the IDE shell.
 */
import puppeteer from "puppeteer";
import { mkdirSync, writeFileSync, readFileSync, existsSync } from "node:fs";
import { join, dirname } from "node:path";
import { fileURLToPath } from "node:url";

const __dirname = dirname(fileURLToPath(import.meta.url));
const WEB_ROOT = join(__dirname, "..");

/** Load KEY=VALUE pairs from a gitignored env file into process.env (no override). */
function loadEnvFile(filePath) {
  if (!existsSync(filePath)) return;
  const text = readFileSync(filePath, "utf8");
  for (const raw of text.split("\n")) {
    const line = raw.trim();
    if (!line || line.startsWith("#")) continue;
    const eq = line.indexOf("=");
    if (eq < 0) continue;
    const key = line.slice(0, eq).trim();
    let val = line.slice(eq + 1).trim();
    if (
      (val.startsWith('"') && val.endsWith('"')) ||
      (val.startsWith("'") && val.endsWith("'"))
    ) {
      val = val.slice(1, -1);
    }
    if (!(key in process.env)) process.env[key] = val;
  }
}

loadEnvFile(join(WEB_ROOT, ".env.local"));
loadEnvFile(join(WEB_ROOT, ".env"));

const BASE = process.env.AUDIT_BASE || "http://127.0.0.1:3000";
const EMAIL = process.env.AUDIT_EMAIL || "";
const PASSWORD = process.env.AUDIT_PASSWORD || "";
const OUT = join(WEB_ROOT, ".mobile-audit");

const VIEWPORTS = [
  { name: "iphone-se", width: 375, height: 667, isMobile: true },
  { name: "iphone-14", width: 390, height: 844, isMobile: true },
  { name: "pixel-7", width: 412, height: 915, isMobile: true },
  { name: "tablet", width: 768, height: 1024, isMobile: true },
  { name: "desktop", width: 1280, height: 800, isMobile: false },
];

async function login(page) {
  if (!EMAIL || !PASSWORD) {
    console.log("No AUDIT_EMAIL/AUDIT_PASSWORD — skipping authenticated routes.");
    return false;
  }
  // Prefer localhost — better-auth trusts it; 127.0.0.1 is also listed but
  // cookies/baseURL can disagree depending on how the server was started.
  const loginBase = BASE.replace("127.0.0.1", "localhost");
  await page.goto(`${loginBase}/login`, { waitUntil: "networkidle2", timeout: 30000 });
  await page.waitForSelector('input[type="email"]', { timeout: 10000 });
  await page.evaluate((email, password) => {
    const setNative = (sel, val) => {
      const el = document.querySelector(sel);
      if (!el) return;
      const setter = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")?.set;
      setter?.call(el, val);
      el.dispatchEvent(new Event("input", { bubbles: true }));
      el.dispatchEvent(new Event("change", { bubbles: true }));
    };
    setNative('input[type="email"]', email);
    setNative('input[type="password"]', password);
  }, EMAIL, PASSWORD);
  await Promise.all([
    page.click('button[type="submit"]'),
    page.waitForNavigation({ waitUntil: "networkidle2", timeout: 30000 }).catch(() => null),
  ]);
  await new Promise((r) => setTimeout(r, 1200));
  const url = page.url();
  const ok = !url.includes("/login");
  console.log(ok ? `Logged in as ${EMAIL}` : `Login failed (still at ${url})`);
  return ok;
}

async function auditPage(page, route, vp) {
  const url = `${BASE}${route}`;
  const res = await page.goto(url, { waitUntil: "networkidle2", timeout: 30000 }).catch((e) => ({
    ok: () => false,
    status: () => 0,
    error: String(e),
  }));
  const status = typeof res?.status === "function" ? res.status() : 0;
  await new Promise((r) => setTimeout(r, 600));

  const metrics = await page.evaluate(() => {
    const doc = document.documentElement;
    const body = document.body;
    const scrollW = Math.max(doc.scrollWidth, body?.scrollWidth || 0);
    const clientW = doc.clientWidth;
    const overflowX = scrollW > clientW + 1;

    const offenders = [];
    for (const el of Array.from(document.querySelectorAll("body *"))) {
      const style = window.getComputedStyle(el);
      if (style.display === "none" || style.visibility === "hidden") continue;
      if (style.pointerEvents === "none" && (style.transform.includes("matrix") || el.getAttribute("aria-hidden") === "true")) {
        continue; // off-canvas drawers
      }
      const rect = el.getBoundingClientRect();
      if (rect.width < 1 || rect.height < 1) continue;
      if (style.overflowX === "auto" || style.overflowX === "scroll") continue;
      // Off-canvas / translated drawers parked off the left or right edge.
      if (rect.right <= 0 || rect.left >= clientW) continue;
      if (rect.right > clientW + 2 || rect.left < -2) {
        offenders.push({
          tag: el.tagName.toLowerCase(),
          cls: (typeof el.className === "string" ? el.className : "").slice(0, 100),
          left: Math.round(rect.left),
          right: Math.round(rect.right),
          width: Math.round(rect.width),
        });
        if (offenders.length >= 12) break;
      }
    }

    const bottomNav = !!document.querySelector('nav[aria-label="Primary"]');
    const chatPanel = !!document.querySelector(".chat-panel");
    const composer = !!document.querySelector('textarea[aria-label="Message the agent"]');

    return {
      title: document.title,
      path: location.pathname,
      scrollW,
      clientW,
      overflowX,
      offenders,
      bottomNav,
      chatPanel,
      composer,
      bodyText: (body?.innerText || "").slice(0, 160),
    };
  });

  const shot = join(OUT, `${vp.name}${route.replace(/\//g, "_") || "_root"}.png`);
  await page.screenshot({ path: shot, fullPage: false });

  return { route, status, shot, ...metrics };
}

async function main() {
  mkdirSync(OUT, { recursive: true });
  const browser = await puppeteer.launch({
    headless: true,
    args: ["--no-sandbox", "--disable-setuid-sandbox"],
  });
  const findings = [];

  try {
    // Shared login context so cookies persist across viewports.
    const boot = await browser.newPage();
    const authed = await login(boot);
    const cookies = authed ? await boot.cookies() : [];
    await boot.close();

    const routes = authed ? ["/", "/login"] : ["/login", "/setup"];

    for (const vp of VIEWPORTS) {
      const page = await browser.newPage();
      if (cookies.length) await page.setCookie(...cookies);
      await page.setViewport({
        width: vp.width,
        height: vp.height,
        isMobile: vp.isMobile,
        hasTouch: vp.isMobile,
        deviceScaleFactor: 1,
      });

      for (const route of routes) {
        try {
          const result = await auditPage(page, route, vp);
          findings.push({ viewport: vp.name, ...result });

          let navNote = "";
          if (route === "/" && vp.isMobile) {
            navNote = result.bottomNav ? "nav=yes" : "nav=MISSING";
          } else if (route === "/" && !vp.isMobile) {
            navNote = result.bottomNav ? "nav=unexpected" : "nav=desktop-ok";
          }

          const flag = result.overflowX ? "OVERFLOW" : "ok";
          console.log(
            `[${vp.name}] ${route} → ${result.status} ${flag} scroll=${result.scrollW}/${result.clientW} bottomNav=${result.bottomNav} ${navNote}`,
          );
          if (result.offenders.length) {
            console.log(
              `  offenders: ${result.offenders
                .slice(0, 5)
                .map((o) => `${o.tag}.${o.cls.split(" ")[0]}@${o.left}-${o.right}`)
                .join(", ")}`,
            );
          }
        } catch (err) {
          console.log(`[${vp.name}] ${route} → ERROR ${err}`);
          findings.push({ viewport: vp.name, route, error: String(err) });
        }
      }
      await page.close();
    }
  } finally {
    await browser.close();
  }

  const overflowCount = findings.filter((f) => f.overflowX).length;
  const missingNav = findings.filter(
    (f) =>
      f.route === "/" &&
      ["iphone-se", "iphone-14", "pixel-7", "tablet"].includes(f.viewport) &&
      !f.bottomNav,
  ).length;

  // Strip secrets from any serialized output.
  const summary = {
    at: new Date().toISOString(),
    base: BASE,
    authed: !!(EMAIL && PASSWORD),
    overflowCount,
    missingNav,
    findings: findings.map(({ bodyText, ...rest }) => rest),
  };
  writeFileSync(join(OUT, "report.json"), JSON.stringify(summary, null, 2));
  console.log(`\nWrote ${join(OUT, "report.json")}`);
  console.log(`Overflow: ${overflowCount} · Missing mobile nav: ${missingNav}`);
  process.exit(overflowCount > 0 || missingNav > 0 ? 2 : 0);
}

main().catch((err) => {
  console.error(err);
  process.exit(1);
});
