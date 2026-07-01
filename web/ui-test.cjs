// Real interactive UI test — drives a headless Chromium through the actual UI:
// opens dropdowns, clicks options, types, and verifies the app responds. This is
// NOT an API test; it exercises the real DOM the user clicks.
const puppeteer = require("puppeteer");

const BASE = "http://localhost:3000";
const log = (...a) => console.log("•", ...a);
const fail = (msg) => { console.error("✗ FAIL:", msg); process.exitCode = 1; };

(async () => {
  const browser = await puppeteer.launch({
    headless: "new",
    args: ["--no-sandbox", "--disable-setuid-sandbox", "--disable-dev-shm-usage"],
  });
  const page = await browser.newPage();
  await page.setViewport({ width: 1280, height: 900 });
  const errors = [];
  page.on("console", (m) => { if (m.type() === "error") errors.push(m.text()); });
  page.on("pageerror", (e) => errors.push(String(e)));

  log("loading page…");
  await page.goto(BASE, { waitUntil: "networkidle2", timeout: 45000 });

  // Wait for hydration + models (the model button shows a model name once loaded).
  await page.waitForFunction(
    () => !!document.querySelector('button[aria-expanded]'),
    { timeout: 30000 },
  );
  // Give the SSE snapshot a moment to hydrate + auto-select a model.
  await new Promise((r) => setTimeout(r, 2500));

  log("console errors so far:", errors.length ? errors : "none");
  if (errors.length) errors.slice(0, 5).forEach((e) => console.log("   err:", e));

  // ---- Capture the initial model label (the button text) ----
  const modelBtns = await page.$$('button[aria-expanded]');
  // The model dropdown is the first aria-expanded button (menu is 4th, think is 2nd).
  // Identify by the ModelIcon-ish: pick buttons whose next sibling span has model text.
  const allBtnInfo = await page.$$eval('button[aria-expanded]', (els) =>
    els.map((e) => ({ expanded: e.getAttribute("aria-expanded"), text: e.innerText.replace(/\s+/g, " ").trim() })),
  );
  log("dropdown buttons:", JSON.stringify(allBtnInfo));

  // 1) MODEL DROPDOWN
  log("\n=== 1. MODEL DROPDOWN ===");
  // The model button: find the one whose text isn't the thinking level or approval mode.
  // Model button text contains "model"/"Umans"/"GLM" or a model id; thinking is "off/low/medium/high"; approval is "never/destructive/always".
  const modelBtnHandle = await page.evaluateHandle(() => {
    const btns = [...document.querySelectorAll('button[aria-expanded]')];
    return btns.find((b) => /umans|glm|coder|flash|kimi|qwen|model/i.test(b.innerText)) || btns[0];
  });
  const beforeModel = await page.evaluate((el) => el.innerText.replace(/\s+/g, " ").trim(), modelBtnHandle);
  log("model button before:", beforeModel);

  await modelBtnHandle.click();
  await new Promise((r) => setTimeout(r, 400));
  const menuOpen = await page.evaluate(() => !!document.querySelector('.animate-fade-in'));
  log("menu opened:", menuOpen);
  if (!menuOpen) fail("model dropdown did not open");

  // List the options
  const options = await page.$$eval('.animate-fade-in button', (els) =>
    els.map((e) => e.innerText.replace(/\s+/g, " ").trim()),
  );
  log("options visible:", JSON.stringify(options));
  if (!options.length) fail("no options shown in model dropdown");

  // Click the LAST option (different from the auto-selected first/preferred)
  const pickedIdx = options.length - 1;
  const optionHandles = await page.$$('.animate-fade-in button');
  await optionHandles[pickedIdx].click();
  await new Promise((r) => setTimeout(r, 400));
  const afterModel = await page.evaluate((el) => el.innerText.replace(/\s+/g, " ").trim(), modelBtnHandle);
  log("model button after click:", afterModel);
  if (beforeModel === afterModel) {
    // maybe same model was auto-selected as last; try clicking a DIFFERENT one
    log("label unchanged — retrying with option 0");
    await modelBtnHandle.click();
    await new Promise((r) => setTimeout(r, 300));
    const h = await page.$$('.animate-fade-in button');
    if (h[0]) { await h[0].click(); await new Promise((r) => setTimeout(r, 300)); }
    const after2 = await page.evaluate((el) => el.innerText.replace(/\s+/g, " ").trim(), modelBtnHandle);
    if (after2 === beforeModel) fail("model selection did not change the label");
    else log("✓ model selected (label now:", after2 + ")");
  } else {
    log("✓ model selected (label now:", afterModel + ")");
  }

  // 2) THINKING DROPDOWN
  log("\n=== 2. THINKING DROPDOWN ===");
  const thinkBtnHandle = await page.evaluateHandle(() => {
    const btns = [...document.querySelectorAll('button[aria-expanded]')];
    return btns.find((b) => /^(off|low|medium|high|minimal)$/i.test(b.innerText.trim())) || btns[1];
  });
  const beforeThink = await page.evaluate((el) => el.innerText.replace(/\s+/g, " ").trim(), thinkBtnHandle);
  log("thinking button before:", beforeThink);
  await thinkBtnHandle.click();
  await new Promise((r) => setTimeout(r, 300));
  const thinkOpts = await page.$$eval('.animate-fade-in button', (els) => els.map((e) => e.innerText.replace(/\s+/g, " ").trim()));
  log("thinking options:", JSON.stringify(thinkOpts));
  // pick one different from current
  const cur = beforeThink.trim().toLowerCase();
  const target = thinkOpts.find((o) => o.trim().toLowerCase() !== cur) || thinkOpts[0];
  log("clicking:", target);
  const th = await page.$$('.animate-fade-in button');
  const targetIdx = thinkOpts.findIndex((o) => o.trim().toLowerCase() === target.trim().toLowerCase());
  if (th[targetIdx]) { await th[targetIdx].click(); await new Promise((r) => setTimeout(r, 300)); }
  const afterThink = await page.evaluate((el) => el.innerText.replace(/\s+/g, " ").trim(), thinkBtnHandle);
  log("thinking after:", afterThink);
  if (afterThink.trim().toLowerCase() !== target.trim().toLowerCase()) fail("thinking selection did not apply");
  else log("✓ thinking selected");

  // 3) APPROVAL DROPDOWN
  log("\n=== 3. APPROVAL DROPDOWN ===");
  const apprBtnHandle = await page.evaluateHandle(() => {
    const btns = [...document.querySelectorAll('button[aria-expanded]')];
    return btns.find((b) => /never|destructive|always/i.test(b.innerText)) || btns[btns.length - 1];
  });
  await apprBtnHandle.click();
  await new Promise((r) => setTimeout(r, 300));
  const apprOpts = await page.$$eval('.animate-fade-in button', (els) => els.map((e) => e.innerText.replace(/\s+/g, " ").trim()));
  log("approval options:", JSON.stringify(apprOpts));
  // click "never" then back to "destructive"
  const neverH = await page.$$('.animate-fade-in button');
  const neverIdx = apprOpts.findIndex((o) => /never/i.test(o));
  if (neverH[neverIdx]) { await neverH[neverIdx].click(); await new Promise((r) => setTimeout(r, 300)); }
  log("✓ approval 'never' clicked");

  // 4) DROPDOWN CLOSES ON OUTSIDE CLICK
  log("\n=== 4. OUTSIDE-CLICK CLOSES DROPDOWN ===");
  await apprBtnHandle.click();
  await new Promise((r) => setTimeout(r, 300));
  let open1 = await page.evaluate(() => !!document.querySelector('.animate-fade-in'));
  log("dropdown open after toggle:", open1);
  // click the messages area (outside)
  await page.mouse.click(640, 500);
  await new Promise((r) => setTimeout(r, 300));
  let open2 = await page.evaluate(() => !!document.querySelector('.animate-fade-in'));
  log("dropdown open after outside-click:", open2);
  if (open2) fail("dropdown did not close on outside click");
  else log("✓ dropdown closed on outside click");

  // 5) COMPOSER: type + send
  log("\n=== 5. COMPOSER (type + send) ===");
  const ta = await page.$('textarea[aria-label="Message the agent"]');
  if (!ta) fail("composer textarea not found");
  await ta.type("Reply with exactly one word: PONG");
  const val = await page.evaluate((el) => el.value, ta);
  log("textarea value:", JSON.stringify(val));
  // press Enter to send
  await page.keyboard.press("Enter");
  await new Promise((r) => setTimeout(r, 300));
  const cleared = await page.evaluate((el) => el.value, ta);
  log("textarea after Enter:", JSON.stringify(cleared));
  if (cleared !== "") fail("composer did not clear after send");
  else log("✓ composer cleared after send");
  // wait for a streaming assistant message to appear
  try {
    await page.waitForFunction(
      () => [...document.querySelectorAll('*')].some((e) => /PONG/i.test(e.textContent || "")),
      { timeout: 30000 },
    );
    log("✓ agent response containing 'PONG' appeared");
  } catch {
    fail("did not see agent response 'PONG' within 30s");
  }

  // 6) SIDEBAR on mobile viewport
  log("\n=== 6. SIDEBAR (mobile) ===");
  await page.setViewport({ width: 390, height: 844 });
  await new Promise((r) => setTimeout(r, 500));
  const menuBtn = await page.$('button[aria-label="Open sessions"]');
  if (!menuBtn) fail("mobile menu button not found");
  await menuBtn.click();
  await new Promise((r) => setTimeout(r, 500));
  const sidebarVisible = await page.evaluate(() => {
    const a = document.querySelector('aside');
    if (!a) return false;
    const r = a.getBoundingClientRect();
    return r.x >= 0 && r.x < 100; // slid into view
  });
  log("sidebar visible on mobile:", sidebarVisible);
  if (!sidebarVisible) fail("sidebar did not open on mobile");
  else log("✓ sidebar opens on mobile");

  log("\n=== DONE ===");
  if (errors.length) {
    console.log("console errors during run:", errors.length);
    errors.slice(0, 8).forEach((e) => console.log("   ", e));
  }
  if (!process.exitCode) console.log("\n✅ ALL UI INTERACTION TESTS PASSED");
  else console.log("\n❌ SOME TESTS FAILED (see above)");
  await browser.close();
})().catch((e) => { console.error("FATAL:", e); process.exit(1); });
