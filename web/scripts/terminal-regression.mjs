import puppeteer from "puppeteer";
import { existsSync, readFileSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const HERE = dirname(fileURLToPath(import.meta.url));
const WEB_ROOT = join(HERE, "..");
const ARTIFACTS = join(WEB_ROOT, ".frontend-audit", "runtime");
const BASE = process.env.AUDIT_BASE || "http://localhost:3000";
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

async function login(page) {
  await page.goto(`${BASE}/login`, { waitUntil: "networkidle2" });
  await page.type('input[type="email"]', process.env.AUDIT_EMAIL || "");
  await page.type('input[type="password"]', process.env.AUDIT_PASSWORD || "");
  await Promise.all([
    page.click('button[type="submit"]'),
    page.waitForNavigation({ waitUntil: "networkidle2" }).catch(() => null),
  ]);
  await page.waitForSelector('button[aria-label="Terminal"]', { timeout: 45_000 });
  await page.waitForFunction(() => [...document.querySelectorAll("button")].some((el) =>
    (el.getAttribute("title") || "").startsWith("Switch project · /"),
  ), { timeout: 45_000 });
}

async function currentWorkspace(page) {
  return page.evaluate(() => {
    const button = [...document.querySelectorAll("button")].find((el) => (el.getAttribute("title") || "").startsWith("Switch project · /"));
    return (button?.getAttribute("title") || "").replace("Switch project · ", "");
  });
}

async function resetLayout(page, ws) {
  await page.evaluate((workspace) => {
    const state = {
      activePanel: "explorer", sidebarWidth: 256, sidebarCollapsed: false,
      bottomPanelHeight: 220, bottomPanelVisible: false, copilotVisible: true, copilotWidth: 440,
      panelLocations: { chat: "right", terminal: "bottom", git: "left", preview: "main", screen: "main" },
      panelVisibility: { chat: true, terminal: false, git: false, preview: false, screen: false },
      activeDockPanels: { left: null, right: "chat", bottom: null, main: null },
      leftDockWidth: 360, expandedDirs: [], terminals: [], activeTerminalId: null, uiMode: "ide",
    };
    localStorage.setItem(`catcode:ide-layout:${encodeURIComponent(workspace)}`, JSON.stringify(state));
  }, ws);
  await page.reload({ waitUntil: "networkidle2" });
  await page.waitForSelector('button[aria-label="Terminal"]');
  await wait(500);
}

async function activeSession(page, ws) {
  return page.evaluate((workspace) => {
    const raw = localStorage.getItem(`catcode:ide-layout:${encodeURIComponent(workspace)}`);
    const state = raw ? JSON.parse(raw) : {};
    return { id: state.activeTerminalId, terminals: state.terminals || [] };
  }, ws);
}

async function terminateFromSecondSocket(page, id, workspace) {
  return page.evaluate(async ({ sessionId, workspace }) => {
    const scheme = location.protocol === "https:" ? "wss" : "ws";
    const ws = new WebSocket(`${scheme}://${location.host}/api/terminal`);
    const events = [];
    await new Promise((resolve, reject) => {
      let timer;
      ws.onopen = () => {
        ws.send(JSON.stringify({ type: "terminate", sessionId, workspace }));
        timer = setTimeout(resolve, 700);
      };
      ws.onmessage = (event) => events.push(event.data);
      ws.onclose = () => { clearTimeout(timer); resolve(); };
      ws.onerror = reject;
    });
    const stateAfterAckWindow = ws.readyState;
    if (ws.readyState === WebSocket.OPEN) ws.close();
    return { events, stateAfterAckWindow };
  }, { sessionId: id, workspace });
}

async function uiDeadTerminal(page, ws, iteration, socketEvents) {
  const beforeSockets = socketEvents.filter((event) => event.type === "created").length;
  await page.click('button[aria-label="Terminal"]');
  await page.waitForSelector('button[aria-label="new terminal"]');
  await wait(1200);
  let session = await activeSession(page, ws);
  if (!session.id) {
    await page.click('button[aria-label="new terminal"]');
    await wait(1200);
    session = await activeSession(page, ws);
  }
  const id = session.id;
  if (!id) throw new Error("terminal session id missing");
  const terminateEvents = await terminateFromSecondSocket(page, id, ws);
  await wait(2500);
  const afterSockets = socketEvents.filter((event) => event.type === "created").length;
  const ui = await page.evaluate((sessionId) => {
    const tabs = [...document.querySelectorAll('[role="tab"]')];
    const tab = tabs.find((el) => el.textContent?.includes("Terminal"));
    return {
      tabText: tab?.textContent || "",
      terminalRendererPresent: !!document.querySelector("canvas, .ghostty-terminal"),
      deadOrReconnectText: /exited|failed|detached|reconnect|connecting|no longer available|unavailable/i.test(document.body.innerText),
      stored: Object.entries(localStorage).filter(([key]) => key.startsWith("catcode:ide-layout:")),
      sessionId,
    };
  }, id);
  const screenshot = join(ARTIFACTS, `terminal-dead-${iteration}.png`);
  await page.screenshot({ path: screenshot });
  const close = await page.$('button[aria-label^="close Terminal"]');
  if (close) await close.click();
  await wait(300);
  return { id, terminateEvents, beforeSockets, afterSockets, ui, screenshot };
}

async function duplicateIdCollision(page, workspaceA, workspaceB, iteration) {
  return page.evaluate(async ({ workspaceA, workspaceB, iteration }) => {
    const scheme = location.protocol === "https:" ? "wss" : "ws";
    const url = `${scheme}://${location.host}/api/terminal`;
    const id = `audit_collision_${Date.now()}_${iteration}`;
    const connect = (workspace) => new Promise((resolve, reject) => {
      const ws = new WebSocket(url);
      const events = [];
      const timer = setTimeout(() => reject(new Error("open timeout")), 8000);
      ws.onmessage = (event) => {
        events.push(event.data);
        try {
          if (JSON.parse(event.data).type === "ready") {
            clearTimeout(timer);
            resolve({ ws, events });
          }
        } catch {}
      };
      ws.onopen = () => ws.send(JSON.stringify({ type: "open", sessionId: id, workspace, cols: 80, rows: 24 }));
      ws.onerror = reject;
    });
    const a = await connect(workspaceA);
    const b = await connect(workspaceB);
    const terminator = new WebSocket(url);
    await new Promise((resolve, reject) => {
      let timer;
      terminator.onopen = () => {
        terminator.send(JSON.stringify({ type: "terminate", sessionId: id, workspace: workspaceA }));
        timer = setTimeout(resolve, 700);
      };
      terminator.onclose = () => { clearTimeout(timer); resolve(); };
      terminator.onerror = reject;
    });
    const terminatorStateAfterAckWindow = terminator.readyState;
    if (terminator.readyState === WebSocket.OPEN) terminator.close();
    await new Promise((resolve) => setTimeout(resolve, 500));
    const result = {
      id,
      aWorkspace: workspaceA,
      bWorkspace: workspaceB,
      aEvents: a.events,
      bEvents: b.events,
      aState: a.ws.readyState,
      bState: b.ws.readyState,
      terminatorStateAfterAckWindow,
    };
    if (b.ws.readyState === WebSocket.OPEN) {
      b.ws.send(JSON.stringify({ type: "terminate", sessionId: id }));
      await new Promise((resolve) => setTimeout(resolve, 200));
      b.ws.close();
    }
    if (a.ws.readyState === WebSocket.OPEN) a.ws.close();
    return result;
  }, { workspaceA, workspaceB, iteration });
}

const browser = await puppeteer.launch({ headless: "new", args: ["--no-sandbox", "--disable-dev-shm-usage"] });
const page = await browser.newPage();
await page.setViewport({ width: 1366, height: 768 });
const socketEvents = [];
const consoleEvents = [];
const cdp = await page.createCDPSession();
await cdp.send("Network.enable");
cdp.on("Network.webSocketCreated", (event) => socketEvents.push({ type: "created", id: event.requestId, url: event.url }));
cdp.on("Network.webSocketClosed", (event) => socketEvents.push({ type: "closed", id: event.requestId }));
page.on("console", (event) => {
  if (event.type() === "error" || event.type() === "warning") consoleEvents.push({ type: event.type(), text: event.text() });
});

try {
  await login(page);
  const workspaceA = await currentWorkspace(page);
  const workspaceB = workspaceA.endsWith("/web") ? workspaceA.slice(0, -4) : `${workspaceA}/web`;
  await resetLayout(page, workspaceA);
  const result = { at: new Date().toISOString(), workspaceA, workspaceB, uiDeadTerminal: [], duplicateIdCollision: [] };
  for (let i = 1; i <= 2; i++) {
    const dead = await uiDeadTerminal(page, workspaceA, i, socketEvents);
    if (!dead.ui.deadOrReconnectText || dead.terminateEvents.stateAfterAckWindow === 1) {
      throw new Error(
        `terminal disconnect/status or termination acknowledgement regressed: ${JSON.stringify(dead)}`,
      );
    }
    result.uiDeadTerminal.push(dead);
    const collision = await duplicateIdCollision(page, workspaceA, workspaceB, i);
    if (collision.aState === 1 || collision.bState !== 1) {
      throw new Error("workspace-scoped terminal termination regressed");
    }
    result.duplicateIdCollision.push(collision);
  }
  result.socketEvents = socketEvents;
  result.consoleEvents = consoleEvents;
  writeFileSync(join(ARTIFACTS, "terminal-audit.json"), JSON.stringify(result, null, 2));
  console.log(JSON.stringify(result, null, 2));
} finally {
  await browser.close();
}
