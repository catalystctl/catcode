// Puppeteer smoke test for the /hub terminal workspace.
//
// Flow: login → /hub → clear persisted state → add the repo workspace by
// BROWSING (path bar → Open) → tab + auto-launched catcode pane appear →
// preset 2×2 spawns four panes → close one pane → git sidebar renders →
// project tab switch + close.
//
// Run:  node scripts/hub-regression.mjs   (against AUDIT_BASE, default :3000)

import puppeteer from "puppeteer";
import { execSync } from "node:child_process";
import { existsSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const HERE = dirname(fileURLToPath(import.meta.url));
const WEB_ROOT = join(HERE, "..");
const ARTIFACTS = join(WEB_ROOT, ".frontend-audit", "runtime");
mkdirSync(ARTIFACTS, { recursive: true });
const BASE = process.env.AUDIT_BASE || "http://localhost:3000";
const WORKSPACE = process.env.HUB_WORKSPACE || process.cwd();

function loadEnv(path) {
  if (!existsSync(path)) return;
  for (const raw of readFileSync(path, "utf8").split("\n")) {
    const line = raw.trim();
    if (!line || line.startsWith("#")) continue;
    const at = line.indexOf("=");
    if (at < 1) continue;
    const key = line.slice(0, at).trim();
    let value = line.slice(at + 1).trim();
    if ((value.startsWith('"') && value.endsWith('"')) || (value.startsWith("'") && value.endsWith("'"))) value = value.slice(1, -1);
    if (!(key in process.env)) process.env[key] = value;
  }
}
loadEnv(join(WEB_ROOT, ".env.local"));
const wait = (ms) => new Promise((resolve) => setTimeout(resolve, ms));

/** Count TUI processes (`catcode` exactly — not catcode-core) via pgrep. */
function catcodeProcessCount() {
  let out = "";
  try {
    out = execSync("pgrep -af catcode || true", { encoding: "utf8" });
  } catch {
    return 0;
  }
  return out
    .split("\n")
    .filter(Boolean)
    .filter((line) => {
      const argv = line.trim().split(/\s+/).slice(1);
      return argv.some((a) => a === "catcode" || a.endsWith("/catcode"));
    }).length;
}

async function login(page) {
  await page.goto(`${BASE}/login`, { waitUntil: "networkidle2" });
  const path = new URL(page.url()).pathname;

  if (path === "/setup") {
    // Fresh instance: create the single account with the audit credentials.
    await page.type('input[type="email"]', process.env.AUDIT_EMAIL || "");
    const passwords = await page.$$('input[type="password"]');
    if (passwords.length < 2) throw new Error("setup form missing password fields");
    await passwords[0].type(process.env.AUDIT_PASSWORD || "");
    await passwords[1].type(process.env.AUDIT_PASSWORD || "");
    await page.click('button[type="submit"]');
    // signUp is SPA-driven (router.replace("/")) and the session cookie is
    // HttpOnly — wait for the URL to leave /setup instead of polling cookies.
    await page.waitForFunction(
      () => !window.location.pathname.startsWith("/setup"),
      { timeout: 20_000 },
    );
    // The form navigates to "/" (the IDE); the hub smoke runs from /hub.
    return;
  }

  if (await page.$('input[type="email"]')) {
    await page.type('input[type="email"]', process.env.AUDIT_EMAIL || "");
    await page.type('input[type="password"]', process.env.AUDIT_PASSWORD || "");
    await Promise.all([
      page.click('button[type="submit"]'),
      page.waitForNavigation({ waitUntil: "networkidle2" }).catch(() => null),
    ]);
  }
}

const browser = await puppeteer.launch({ headless: "new", args: ["--no-sandbox", "--disable-dev-shm-usage"] });
const page = await browser.newPage();
await page.setViewport({ width: 1440, height: 900 });
page.on("dialog", (dialog) => dialog.accept()); // close-pane confirm
const consoleErrors = [];
page.on("console", (event) => {
  if (event.type() === "error") consoleErrors.push(event.text());
});

const report = { at: new Date().toISOString(), base: BASE, workspace: WORKSPACE };

try {
  await login(page);
  report.baselineCatcodeProcesses = catcodeProcessCount();

  // ── fresh hub state ───────────────────────────────────────────────────────
  await page.goto(`${BASE}/hub`, { waitUntil: "networkidle2" });
  await page.evaluate(() => localStorage.removeItem("catcode:hub:v1"));
  await page.reload({ waitUntil: "networkidle2" });
  await page.waitForFunction(
    () =>
      !!document.querySelector('button[aria-label="Add or switch project"]') ||
      document.body.innerText.includes("Add a project"),
    { timeout: 20_000 },
  );
  await page.waitForFunction(
    () => document.body.innerText.includes("No projects open yet"),
    { timeout: 15_000 },
  );

  console.log("step: open switcher");
  // ── add the workspace by browsing ─────────────────────────────────────────
  await page.click('button[aria-label="Add or switch project"]');
  await page.waitForFunction(
    () => [...document.querySelectorAll("button")].some((b) =>
      (b.textContent || "").includes("Add project by browsing"),
    ),
    { timeout: 10_000 },
  );
  await page.evaluate(() => {
    const btn = [...document.querySelectorAll("button")].find((b) =>
      (b.textContent || "").includes("Add project by browsing"),
    );
    if (btn) btn.click();
  });
  await page.waitForSelector('input[aria-label="Directory path"]', { timeout: 10_000 });

  console.log("step: browse path bar");
  // Path bar → jump straight to the workspace dir. Entering browse mode fires
  // an async initial loadBrowse() that fills the bar with the HOME dir; a
  // late resolve would clobber whatever we set. Wait until that initial load
  // has rendered (bar non-empty) before overwriting the value.
  await page.waitForFunction(
    () => {
      const el = document.querySelector('input[aria-label="Directory path"]');
      return !!el && el.value.length > 0;
    },
    { timeout: 10_000 },
  );
  await wait(300);

  // The input is a React controlled component and page.type only APPENDS (and
  // can race re-renders), so set the full value via the native setter in ONE
  // bubbling input event — deterministic.
  await page.evaluate((sel, value) => {
    const el = document.querySelector(sel);
    if (!el) return;
    const set = Object.getOwnPropertyDescriptor(window.HTMLInputElement.prototype, "value").set;
    set.call(el, value);
    el.dispatchEvent(new Event("input", { bubbles: true }));
  }, 'input[aria-label="Directory path"]', WORKSPACE);
  await page.waitForFunction(
    (sel, value) => document.querySelector(sel)?.value === value,
    { timeout: 5_000 },
    'input[aria-label="Directory path"]',
    WORKSPACE,
  );
  await page.click('button[type="submit"]');
  const openLabel = `Open ${WORKSPACE.split("/").filter(Boolean).pop()}`;
  await page.waitForFunction(
    (label) => [...document.querySelectorAll("button")].some((b) => ((b.getAttribute("aria-label") || "") + (b.textContent || "")).includes(label)),
    { timeout: 15_000 },
    openLabel,
  );
  await page.evaluate((label) => {
    const btn = [...document.querySelectorAll("button")].find((b) => ((b.getAttribute("aria-label") || "") + (b.textContent || "")).includes(label));
    if (btn) btn.click();
  }, openLabel);

  console.log("step: waiting for tab + terminal");
  // ── tab + auto-launched catcode ───────────────────────────────────────────
  await page.waitForSelector('[role="tab"][aria-selected="true"]', { timeout: 15_000 });
  await page.waitForSelector("canvas, .ghostty-terminal", { timeout: 30_000 });
  await wait(4000); // let catcode boot inside the PTY
  const panes1 = await page.$$eval('button[aria-label="Close pane"]', (els) => els.length);
  if (panes1 !== 1) throw new Error(`expected 1 pane after add, got ${panes1}`);
  const afterSingle = catcodeProcessCount();
  if (afterSingle < report.baselineCatcodeProcesses + 1) {
    throw new Error(`catcode did not auto-launch: baseline ${report.baselineCatcodeProcesses}, now ${afterSingle}`);
  }

  console.log("step: preset 2x2");
  // ── preset 2×2 ────────────────────────────────────────────────────────────
  await page.click('button[aria-label="Layout 2×2"]');
  await page.waitForFunction(() => document.querySelectorAll('button[aria-label="Close pane"]').length === 4, { timeout: 15_000 });
  await wait(3500);
  const panes4 = await page.$$eval('button[aria-label="Close pane"]', (els) => els.length);
  if (panes4 !== 4) throw new Error(`expected 4 panes after 2x2 preset, got ${panes4}`);
  const afterGrid = catcodeProcessCount();
  if (afterGrid < afterSingle + 3) {
    throw new Error(`2x2 preset did not spawn catcode in new panes: single=${afterSingle}, grid=${afterGrid}`);
  }

  console.log("step: git sidebar");
  // ── git sidebar ───────────────────────────────────────────────────────────
  // NOTE: GitPanel's header uses CSS text-transform:uppercase, so innerText
  // reports "SOURCE CONTROL" — match case-insensitively.
  await page.waitForFunction(() => document.body.innerText.toUpperCase().includes("SOURCE CONTROL"), { timeout: 15_000 });
  const gitAside = await page.$('aside[aria-label="Git"]');
  if (!gitAside) throw new Error("git sidebar aside missing");
  const gitText = await page.evaluate(() => document.querySelector('aside[aria-label="Git"]')?.innerText || "");
  if (!/source control/i.test(gitText)) throw new Error("git sidebar empty");
  report.gitSidebarSnippet = gitText.slice(0, 240);

  // toggle off/on
  await page.click('button[aria-label="Hide Git panel"]');
  await wait(200);
  if (await page.$('aside[aria-label="Git"]')) throw new Error("git sidebar did not hide");
  await page.click('button[aria-label="Show Git panel"]');
  await page.waitForSelector('aside[aria-label="Git"]', { timeout: 5000 });

  console.log("step: close pane");
  // ── close one pane ────────────────────────────────────────────────────────
  const closes = await page.$$('button[aria-label="Close pane"]');
  await closes[0].click();
  await page.waitForFunction(() => document.querySelectorAll('button[aria-label="Close pane"]').length === 3, { timeout: 10_000 });

  console.log("step: refresh persistence");
  // ── persistence: refresh reattaches ───────────────────────────────────────
  await page.reload({ waitUntil: "networkidle2" });
  await page.waitForSelector('[role="tab"][aria-selected="true"]', { timeout: 15_000 });
  await page.waitForFunction(() => document.querySelectorAll('button[aria-label="Close pane"]').length === 3, { timeout: 15_000 });

  // ── close the tab (terminates PTYs, keeps layout) ────────────────────────
  await page.click('[role="tab"][aria-selected="true"] button[aria-label^="Close"]');
  await page.waitForFunction(() => document.body.innerText.includes("No projects open yet"), { timeout: 10_000 });

  await page.screenshot({ path: join(ARTIFACTS, "hub-final.png") });
  report.status = "pass";
} catch (err) {
  report.status = "fail";
  report.error = String(err?.message ?? err);
  await page.screenshot({ path: join(ARTIFACTS, "hub-fail.png") }).catch(() => null);
} finally {
  report.consoleErrors = consoleErrors.slice(0, 20);
  await browser.close();
}

writeFileSync(join(ARTIFACTS, "hub-audit.json"), JSON.stringify(report, null, 2));
console.log(JSON.stringify(report, null, 2));
if (report.status !== "pass") process.exit(1);
