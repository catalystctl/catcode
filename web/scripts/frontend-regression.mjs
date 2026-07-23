import puppeteer from "puppeteer";
import { existsSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const HERE = dirname(fileURLToPath(import.meta.url));
const WEB_ROOT = join(HERE, "..");
const BASE = process.env.AUDIT_BASE || "http://localhost:3000";
const OUT = join(WEB_ROOT, ".frontend-audit", "runtime");
mkdirSync(OUT, { recursive: true });

function loadEnv(path) {
  if (!existsSync(path)) return;
  for (const raw of readFileSync(path, "utf8").split("\n")) {
    const line = raw.trim();
    if (!line || line.startsWith("#")) continue;
    const i = line.indexOf("=");
    if (i < 1) continue;
    const key = line.slice(0, i).trim();
    let value = line.slice(i + 1).trim();
    if ((value.startsWith('"') && value.endsWith('"')) || (value.startsWith("'") && value.endsWith("'"))) {
      value = value.slice(1, -1);
    }
    if (!(key in process.env)) process.env[key] = value;
  }
}
loadEnv(join(WEB_ROOT, ".env.local"));
loadEnv(join(WEB_ROOT, ".env"));

const EMAIL = process.env.AUDIT_EMAIL || "";
const PASSWORD = process.env.AUDIT_PASSWORD || "";
const wait = (ms) => new Promise((resolve) => setTimeout(resolve, ms));

async function login(page) {
  if (!EMAIL || !PASSWORD) throw new Error("AUDIT_EMAIL/AUDIT_PASSWORD are required");
  await page.goto(`${BASE}/login`, { waitUntil: "networkidle2", timeout: 45_000 });
  await page.type('input[type="email"]', EMAIL);
  await page.type('input[type="password"]', PASSWORD);
  await Promise.all([
    page.click('button[type="submit"]'),
    page.waitForNavigation({ waitUntil: "networkidle2", timeout: 45_000 }).catch(() => null),
  ]);
  await page.waitForSelector('button[aria-label="Explorer"]', { timeout: 15_000 }).catch(() => null);
  if (!await page.$('button[aria-label="Explorer"]')) {
    const reason = await page.evaluate(() => {
      const text = document.body.innerText;
      if (text.includes("Two-factor authentication")) return "the audit account requires a second factor";
      if (text.includes("Invalid email or password")) return "the configured audit credentials were rejected";
      return `login remained at ${location.pathname}: ${text.slice(0, 240).replaceAll("\n", " | ")}`;
    });
    throw new Error(`Frontend browser tests could not authenticate: ${reason}`);
  }
  await page.waitForFunction(() => [...document.querySelectorAll("button")].some((el) =>
    (el.getAttribute("title") || "").startsWith("Switch project · /"),
  ), { timeout: 45_000 });
}

async function workspace(page) {
  return page.evaluate(() => {
    const button = [...document.querySelectorAll("button")].find((el) =>
      (el.getAttribute("title") || "").startsWith("Switch project · "),
    );
    return (button?.getAttribute("title") || "").replace("Switch project · ", "");
  });
}

function fixturePath(ws, name) {
  return `catcode-frontend-audit-${name}`;
}

async function apiFile(page, ws, path, options = {}) {
  return page.evaluate(async ({ ws, path, options }) => {
    const url = options.method === "DELETE" || !options.method || options.method === "GET"
      ? `/api/file?path=${encodeURIComponent(path)}&workspace=${encodeURIComponent(ws)}`
      : "/api/file";
    const response = await fetch(url, {
      method: options.method || "GET",
      headers: options.body === undefined ? undefined : { "Content-Type": "application/json" },
      body: options.method === "DELETE" || options.body === undefined
        ? undefined
        : JSON.stringify({ path, workspace: ws, content: options.body }),
    });
    return { status: response.status, data: await response.json().catch(() => ({})) };
  }, { ws, path, options });
}

async function openFile(page, path) {
  const explorerActive = await page.$eval('button[aria-label="Explorer"]', (el) => el.getAttribute("aria-pressed") === "true");
  if (!explorerActive) await page.click('button[aria-label="Explorer"]');
  await wait(300);
  await page.click('button[aria-label="Refresh"]');
  await page.waitForSelector(`[data-tree-path="${path}"]`, { timeout: 15_000 });
  await page.click(`[data-tree-path="${path}"]`);
  await page.waitForSelector(".monaco-editor", { timeout: 45_000 });
  await wait(500);
}

async function setEditorText(page, text) {
  const input = await page.$(".monaco-editor textarea.inputarea, .monaco-editor .view-lines");
  if (!input) throw new Error("Monaco input missing");
  await input.evaluate((el) => el.focus());
  await input.click();
  await page.keyboard.down("Control");
  await page.keyboard.press("KeyA");
  await page.keyboard.up("Control");
  await page.keyboard.type(text);
}

async function appendEditorText(page, text) {
  const input = await page.$(".monaco-editor textarea.inputarea, .monaco-editor .view-lines");
  if (!input) throw new Error("Monaco input missing");
  await input.evaluate((el) => el.focus());
  await input.click();
  await page.keyboard.down("Control");
  await page.keyboard.press("End");
  await page.keyboard.up("Control");
  await page.keyboard.type(text);
}

async function save(page) {
  await page.keyboard.down("Control");
  await page.keyboard.press("KeyS");
  await page.keyboard.up("Control");
}

async function tabDirty(page, path) {
  return page.evaluate((target) => {
    const tab = [...document.querySelectorAll('[role="tab"]')].find((el) => el.getAttribute("title") === target);
    return { found: !!tab, text: tab?.textContent || "", dirty: (tab?.textContent || "").includes("●") };
  }, path);
}

async function closeTab(page, path) {
  const dirty = await tabDirty(page, path);
  if (dirty.dirty) {
    page.once("dialog", (dialog) => dialog.accept());
  }
  await page.evaluate((target) => {
    const tab = [...document.querySelectorAll('[role="tab"]')].find((el) => el.getAttribute("title") === target);
    tab?.querySelector("button")?.click();
  }, path);
  await wait(250);
}

async function switchProject(page, target) {
  await page.click('button[aria-label="Switch project"]');
  await page.waitForSelector('[role="dialog"][aria-label="Switch project"]');
  const clicked = await page.evaluate((path) => {
    const spans = [...document.querySelectorAll('[role="dialog"][aria-label="Switch project"] span')];
    const pathSpan = spans.find((el) => el.textContent?.trim() === path);
    const match = pathSpan?.closest("button");
    if (!(match instanceof HTMLButtonElement)) return false;
    match.click();
    return true;
  }, target);
  if (!clicked) throw new Error(`project not present: ${target}`);
  await page.waitForFunction((path) => {
    return [...document.querySelectorAll("button")].some((el) =>
      el.getAttribute("title") === `Switch project · ${path}`,
    );
  }, { timeout: 45_000 }, target);
  await wait(500);
}

async function persistedLayoutRecovery(page, ws) {
  await wait(600);
  const key = `catcode:ide-layout:${encodeURIComponent(ws)}`;
  const original = await page.evaluate((k) => localStorage.getItem(k), key);
  const bad = {
    activePanel: "not-a-panel",
    sidebarWidth: 100000,
    sidebarCollapsed: false,
    bottomPanelHeight: -9000,
    bottomPanelVisible: true,
    copilotVisible: true,
    copilotWidth: 100000,
    leftDockWidth: 100000,
    panelLocations: { chat: "right", terminal: "bottom", git: "left", preview: "main", screen: "main" },
    panelVisibility: { chat: true, terminal: true, git: false, preview: false, screen: false },
    activeDockPanels: { left: null, right: "chat", bottom: "terminal", main: null },
    expandedDirs: [],
    terminals: [],
    activeTerminalId: null,
    uiMode: "ide",
  };
  await page.evaluate(({ key, bad }) => localStorage.setItem(key, JSON.stringify(bad)), { key, bad });
  await page.reload({ waitUntil: "networkidle2" });
  await page.waitForSelector('button[aria-label="Explorer"]');
  await wait(700);
  const measured = await page.evaluate(() => {
    const aside = document.querySelector("aside");
    const right = document.querySelector('section[aria-label="right dock"]');
    const main = document.querySelector("main");
    return {
      viewport: innerWidth,
      aside: aside?.getBoundingClientRect().width ?? null,
      right: right?.getBoundingClientRect().width ?? null,
      main: main?.getBoundingClientRect().width ?? null,
      documentWidth: document.documentElement.scrollWidth,
      stored: Object.entries(localStorage)
        .filter(([k]) => k.startsWith("catcode:ide-layout"))
        .map(([k, v]) => [k, v]),
    };
  });
  await page.evaluate(({ key, original }) => {
    if (original === null) localStorage.removeItem(key);
    else localStorage.setItem(key, original);
  }, { key, original });
  await page.reload({ waitUntil: "networkidle2" });
  await page.waitForSelector('button[aria-label="Explorer"]');
  if (
    measured.main === null ||
    measured.main < 240 ||
    measured.documentWidth > measured.viewport
  ) {
    throw new Error(`persisted layout remained unusable: ${JSON.stringify(measured)}`);
  }
  return measured;
}

async function resizeInterruptions(page) {
  const handles = await page.$$('div[role="separator"]');
  const results = [];
  for (let i = 0; i < handles.length; i++) {
    const handle = handles[i];
    const box = await handle.boundingBox();
    if (!box) continue;
    await page.mouse.move(box.x + box.width / 2, box.y + box.height / 2);
    await page.mouse.down();
    await page.mouse.move(box.x + 40, box.y + 40);
    await page.evaluate((index) => {
      const handles = document.querySelectorAll('div[role="separator"]');
      handles[index]?.dispatchEvent(new PointerEvent("pointercancel", { bubbles: true, pointerId: 1 }));
    }, i);
    await page.mouse.up();
    results.push(await page.evaluate(() => ({
      classLeft: document.body.classList.contains("catalyst-resizing"),
      cursor: document.body.style.cursor,
      userSelect: document.body.style.userSelect,
    })));
  }
  if (results.some((result) => result.classLeft || result.cursor || result.userSelect)) {
    throw new Error(`resize interruption cleanup regressed: ${JSON.stringify(results)}`);
  }
  return results;
}

async function dragMatrix(page, workspace) {
  const labels = {
    chat: "AI Chat",
    terminal: "Terminal",
    git: "Source Control",
    preview: "Preview",
    screen: "Screen",
  };
  const activity = {
    chat: "Copilot (Chat)",
    terminal: "Terminal",
    git: "Source Control",
    preview: "Preview",
    screen: "Screen",
  };
  const results = [];
  const requestedPanel = process.env.AUDIT_PANEL;
  const panels = Object.keys(labels).filter((panel) => !requestedPanel || panel === requestedPanel);
  for (const panel of panels) {
    const tabSelector = `[draggable="true"][title^="Drag ${labels[panel]}"]`;
    for (const position of ["left", "right", "bottom", "main"]) {
      if (!await page.$(tabSelector)) {
        await page.click(`button[aria-label="${activity[panel]}"]`);
        await page.waitForSelector(tabSelector, { timeout: 5_000 });
        await wait(100);
      }
      const result = await page.evaluate(async ({ label, position }) => {
        const tab = [...document.querySelectorAll('[draggable="true"]')].find((el) =>
          (el.getAttribute("title") || "").startsWith(`Drag ${label}`),
        );
        if (!tab) return { ok: false, reason: "tab missing" };
        const dataTransfer = new DataTransfer();
        tab.dispatchEvent(new DragEvent("dragstart", {
          bubbles: true,
          cancelable: true,
          dataTransfer,
        }));
        const expected = position === "main"
          ? `dock ${label.toLowerCase()} in editor area`
          : `dock ${label.toLowerCase()} ${position}`;
        const target = [...document.querySelectorAll("div")].find((el) =>
          (el.textContent || "").trim().toLowerCase() === expected,
        );
        if (!target) return {
          ok: false,
          reason: "target missing",
          dragging: document.body.classList.contains("catalyst-panel-dragging"),
          payload: dataTransfer.getData("text/plain"),
        };
        target.dispatchEvent(new DragEvent("dragover", {
          bubbles: true,
          cancelable: true,
          dataTransfer,
        }));
        target.dispatchEvent(new DragEvent("drop", {
          bubbles: true,
          cancelable: true,
          dataTransfer,
        }));
        await new Promise((resolve) => setTimeout(resolve, 0));
        const movedTab = [...document.querySelectorAll('[draggable="true"]')].find((el) =>
          (el.getAttribute("title") || "").startsWith(`Drag ${label}`),
        );
        const renderedDock = movedTab?.closest("section")?.getAttribute("aria-label") ??
          (movedTab?.closest("aside") ? "left dock" : "unknown");
        tab.dispatchEvent(new DragEvent("dragend", {
          bubbles: true,
          dataTransfer,
        }));
        return {
          ok: true,
          payload: dataTransfer.getData("text/plain"),
          target: target.textContent?.trim(),
          renderedDock,
        };
      }, { label: labels[panel], position });
      // Persistence is debounced by 200ms; allow a scheduling margin under
      // dev compilation and terminal rendering load.
      await wait(500);
      await page.waitForFunction(
        ({ panel, position, workspace }) => {
          const key = `catcode:ide-layout:${encodeURIComponent(workspace)}`;
          const parsed = key ? JSON.parse(localStorage.getItem(key) || "{}") : {};
          return parsed.panelLocations?.[panel] === position;
        },
        { timeout: 2500 },
        { panel, position, workspace },
      ).catch(() => null);
      const invariant = await page.evaluate(({ panel, position, workspace }) => {
        const key = `catcode:ide-layout:${encodeURIComponent(workspace)}`;
        const parsed = key ? JSON.parse(localStorage.getItem(key) || "{}") : {};
        return {
          location: parsed.panelLocations?.[panel],
          bodyDragging: document.body.classList.contains("catalyst-panel-dragging"),
          overlayBlocking: [...document.querySelectorAll('div.absolute.inset-0.z-50[aria-hidden="true"]')]
            .some((el) => getComputedStyle(el).pointerEvents !== "none"),
          composers: document.querySelectorAll('textarea[aria-label="Message the agent"]').length,
          expected: position,
        };
      }, { panel, position, workspace });
      results.push({ panel, position, dispatch: result, invariant });
    }
  }
  const failures = results.filter((result) =>
    !result.dispatch.ok ||
    result.invariant.location !== result.invariant.expected ||
    result.invariant.bodyDragging ||
    result.invariant.overlayBlocking ||
    result.invariant.composers > 1
  );
  if (failures.length > 0) {
    throw new Error(`panel drag invariants regressed: ${JSON.stringify(failures)}`);
  }
  return results;
}

async function saveRace(page, ws, path, iteration) {
  await apiFile(page, ws, path, { method: "PUT", body: "audit-initial" });
  await openFile(page, path);
  const before = `saved-request-${iteration}`;
  const after = `-typed-after-request-${iteration}`;
  await setEditorText(page, before);
  await page.waitForFunction((target) => {
    const tab = [...document.querySelectorAll('[role="tab"]')].find((el) => el.getAttribute("title") === target);
    return (tab?.textContent || "").includes("●");
  }, { timeout: 10_000 }, path);

  let release;
  let sawRequest;
  const requestSeen = new Promise((resolve) => { sawRequest = resolve; });
  const requestRelease = new Promise((resolve) => { release = resolve; });
  await page.setRequestInterception(true);
  const handler = async (request) => {
    if (request.url().endsWith("/api/file") && request.method() === "PUT") {
      sawRequest(request.postData());
      await requestRelease;
      await request.continue();
    } else {
      await request.continue();
    }
  };
  page.on("request", handler);
  await save(page);
  const postData = await Promise.race([
    requestSeen,
    wait(10_000).then(() => { throw new Error("save PUT was not observed"); }),
  ]);
  await appendEditorText(page, after);
  release();
  await wait(900);
  page.off("request", handler);
  await page.setRequestInterception(false);

  const disk = await apiFile(page, ws, path);
  const dirty = await tabDirty(page, path);
  const screenshot = join(OUT, `save-race-${iteration}.png`);
  await page.screenshot({ path: screenshot });
  await closeTab(page, path);
  return {
    postData: JSON.parse(postData || "{}").content,
    disk: disk.data.content,
    dirty,
    expectedUnsavedText: before + after,
    screenshot,
  };
}

async function projectSwitchDirty(page, originalWs, alternateWs, path, iteration) {
  await apiFile(page, originalWs, path, { method: "PUT", body: "audit-project-switch-initial" });
  await openFile(page, path);
  const unsaved = `unsaved-project-switch-${iteration}`;
  await setEditorText(page, unsaved);
  await page.waitForFunction((target) => {
    const tab = [...document.querySelectorAll('[role="tab"]')].find((el) => el.getAttribute("title") === target);
    return (tab?.textContent || "").includes("●");
  }, { timeout: 10_000 }, path);
  const before = await tabDirty(page, path);
  let dialogCount = 0;
  const dismissHandler = async (dialog) => {
    dialogCount++;
    await dialog.dismiss();
  };
  page.once("dialog", dismissHandler);
  await page.click('button[aria-label="Switch project"]');
  await page.waitForSelector('[role="dialog"][aria-label="Switch project"]');
  await page.evaluate((target) => {
    const pathSpan = [...document.querySelectorAll('[role="dialog"][aria-label="Switch project"] span')]
      .find((el) => el.textContent?.trim() === target);
    const button = pathSpan?.closest("button");
    if (!(button instanceof HTMLButtonElement)) throw new Error(`project not present: ${target}`);
    button.click();
  }, alternateWs);
  await wait(300);
  const workspaceAfterCancel = await workspace(page);
  const stillDirtyAfterCancel = await tabDirty(page, path);
  await page.keyboard.press("Escape");

  page.once("dialog", async (dialog) => {
    dialogCount++;
    await dialog.accept();
  });
  await switchProject(page, alternateWs);
  await switchProject(page, originalWs);
  await openFile(page, path);
  const disk = await apiFile(page, originalWs, path);
  const after = await tabDirty(page, path);
  const screenshot = join(OUT, `project-switch-dirty-${iteration}.png`);
  await page.screenshot({ path: screenshot });
  await closeTab(page, path);
  if (workspaceAfterCancel !== originalWs || !stillDirtyAfterCancel.dirty) {
    throw new Error("dirty project-switch cancellation did not preserve the current editor");
  }
  if (dialogCount !== 2 || disk.data.content !== "audit-project-switch-initial") {
    throw new Error("dirty project-switch discard changed disk state or skipped confirmation");
  }
  return {
    before,
    after,
    disk: disk.data.content,
    unsaved,
    dialogCount,
    workspaceAfterCancel,
    stillDirtyAfterCancel,
    screenshot,
  };
}

async function keyboardConflicts(page, ws, path, iteration) {
  await apiFile(page, ws, path, { method: "PUT", body: "keyboard-audit" });
  await openFile(page, path);
  const editorInput = await page.$(".monaco-editor textarea.inputarea, .monaco-editor .view-lines");
  await editorInput?.evaluate((el) => el.focus());
  await page.keyboard.down("Control");
  await page.keyboard.press("KeyK");
  await page.keyboard.up("Control");
  await wait(250);
  const editorOpenedPalette = !!(await page.$('[role="dialog"][aria-label="Command palette"]'));
  if (editorOpenedPalette) await page.keyboard.press("Escape");

  await page.click('button[aria-label="Terminal"]');
  await page.waitForSelector('button[aria-label="new terminal"]');
  await wait(1000);
  const terminalSurface = await page.$("canvas, [class*='ghostty']");
  await terminalSurface?.click();
  await page.keyboard.down("Control");
  await page.keyboard.press("KeyK");
  await page.keyboard.up("Control");
  await wait(250);
  const terminalOpenedPalette = !!(await page.$('[role="dialog"][aria-label="Command palette"]'));
  if (terminalOpenedPalette) await page.keyboard.press("Escape");
  const screenshot = join(OUT, `keyboard-conflicts-${iteration}.png`);
  await page.screenshot({ path: screenshot });
  await closeTab(page, path);
  return { editorOpenedPalette, terminalOpenedPalette, screenshot };
}

async function main() {
  const browser = await puppeteer.launch({
    headless: "new",
    args: ["--no-sandbox", "--disable-setuid-sandbox", "--disable-dev-shm-usage"],
  });
  const page = await browser.newPage();
  await page.setViewport({ width: 1366, height: 768 });
  const consoleEvents = [];
  const requestFailures = [];
  const badResponses = [];
  const sockets = [];
  const fixtures = [];
  page.on("console", (message) => {
    if (["error", "warning"].includes(message.type())) consoleEvents.push({ type: message.type(), text: message.text() });
  });
  page.on("pageerror", (error) => consoleEvents.push({ type: "pageerror", text: String(error) }));
  page.on("requestfailed", (request) => requestFailures.push({ url: request.url(), error: request.failure()?.errorText }));
  page.on("response", (response) => {
    if (response.status() >= 400) badResponses.push({ url: response.url(), status: response.status() });
  });
  const cdp = await page.createCDPSession();
  await cdp.send("Network.enable");
  cdp.on("Network.webSocketCreated", (event) => sockets.push({ type: "created", url: event.url, id: event.requestId }));
  cdp.on("Network.webSocketClosed", (event) => sockets.push({ type: "closed", id: event.requestId }));

  try {
    await login(page);
    const originalWs = await workspace(page);
    const alternateWs = originalWs.endsWith("/web")
      ? originalWs.slice(0, -4)
      : `${originalWs}/web`;
    const savePath = fixturePath(originalWs, "save-race.txt");
    const projectPath = fixturePath(originalWs, "project-switch-dirty.txt");
    fixtures.push([originalWs, savePath], [originalWs, projectPath]);

    const result = {
      at: new Date().toISOString(),
      base: BASE,
      originalWs,
      alternateWs,
      baseline: await page.evaluate(() => ({
        url: location.href,
        width: innerWidth,
        height: innerHeight,
        bodyText: document.body.innerText.slice(0, 500),
      })),
      persistedLayoutRecovery: [],
      resizeInterruptions: [],
      dragMatrix: [],
      saveRace: [],
      projectSwitchDirty: [],
      keyboardConflicts: [],
    };

    const passes = Number.parseInt(process.env.AUDIT_PASSES || "2", 10);
    for (let i = 1; i <= passes; i++) {
      if (process.env.AUDIT_ONLY === "project") {
        result.projectSwitchDirty.push(await projectSwitchDirty(page, originalWs, alternateWs, projectPath, i));
        continue;
      }
      if (process.env.AUDIT_ONLY === "keyboard") {
        result.keyboardConflicts.push(await keyboardConflicts(page, originalWs, savePath, i));
        continue;
      }
      if (process.env.AUDIT_ONLY === "layout") {
        result.persistedLayoutRecovery.push(await persistedLayoutRecovery(page, originalWs));
        result.resizeInterruptions.push(await resizeInterruptions(page));
        result.dragMatrix.push(await dragMatrix(page, originalWs));
        continue;
      }
      result.saveRace.push(await saveRace(page, originalWs, savePath, i));
      const saveResult = result.saveRace[result.saveRace.length - 1];
      if (!saveResult.dirty.dirty || saveResult.disk !== saveResult.postData) {
        throw new Error(`save-race regression: newer editor text was marked clean: ${JSON.stringify(saveResult)}`);
      }
      if (process.env.AUDIT_ONLY === "save") continue;
      result.projectSwitchDirty.push(await projectSwitchDirty(page, originalWs, alternateWs, projectPath, i));
      result.persistedLayoutRecovery.push(await persistedLayoutRecovery(page, originalWs));
      result.resizeInterruptions.push(await resizeInterruptions(page));
      result.dragMatrix.push(await dragMatrix(page, originalWs));
    }

    result.consoleEvents = consoleEvents;
    result.requestFailures = requestFailures;
    result.badResponses = badResponses;
    result.sockets = sockets;
    writeFileSync(join(OUT, "audit-runtime.json"), JSON.stringify(result, null, 2));
    console.log(JSON.stringify({
      persistedLayoutRecovery: result.persistedLayoutRecovery,
      resizeInterruptions: result.resizeInterruptions,
      dragMatrixFailures: result.dragMatrix.flat().filter((x) =>
        !x.dispatch.ok ||
        x.invariant.location !== x.invariant.expected ||
        x.invariant.bodyDragging ||
        x.invariant.overlayBlocking ||
        x.invariant.composers > 1,
      ),
      saveRace: result.saveRace,
      projectSwitchDirty: result.projectSwitchDirty,
      keyboardConflicts: result.keyboardConflicts,
      consoleEvents: consoleEvents.length,
      requestFailures: requestFailures.length,
      badResponses: badResponses.length,
      sockets: sockets.length,
    }, null, 2));
  } finally {
    for (const [fixtureWorkspace, fixturePath] of fixtures) {
      await apiFile(page, fixtureWorkspace, fixturePath, { method: "DELETE" }).catch(() => null);
    }
    await browser.close();
  }
}

main().catch((error) => {
  console.error(error);
  process.exit(1);
});
