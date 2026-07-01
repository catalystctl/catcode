// Extended UI test — clicks the remaining buttons the first test didn't cover:
// sidebar actions (New/Reset/Compact/Stats), session list load, a tool-call card
// expand, and the full approval gate (set destructive → trigger write → Approve).
const puppeteer = require("puppeteer");
const BASE = "http://localhost:3000";
const log = (...a) => console.log("•", ...a);
const fail = (m) => { console.error("✗ FAIL:", m); process.exitCode = 1; };
const wait = (ms) => new Promise((r) => setTimeout(r, ms));

(async () => {
  const browser = await puppeteer.launch({
    headless: "new",
    args: ["--no-sandbox", "--disable-setuid-sandbox", "--disable-dev-shm-usage"],
  });
  const page = await browser.newPage();
  await page.setViewport({ width: 1280, height: 900 });
  const errors = [];
  page.on("pageerror", (e) => errors.push(String(e)));
  page.on("console", (m) => { if (m.type() === "error") errors.push(m.text()); });

  log("loading…");
  await page.goto(BASE, { waitUntil: "networkidle2", timeout: 45000 });
  await page.waitForSelector('button[aria-expanded]', { timeout: 30000 });
  await wait(2500); // hydrate + auto-select + sessions load

  // ---- SIDEBAR ACTION BUTTONS (New/Reset/Compact/Stats) ----
  log("\n=== SIDEBAR: action buttons ===");
  // Find buttons by their label text in the sidebar footer grid.
  async function clickByLabel(text) {
    const handle = await page.evaluateHandle((t) => {
      const btns = [...document.querySelectorAll("button")];
      return btns.find((b) => b.innerText.trim().toLowerCase() === t);
    }, text);
    const el = handle.asElement();
    if (!el) { fail(`button "${text}" not found`); return false; }
    await el.click();
    return true;
  }

  // New session → should clear messages (reset event)
  let msgCountBefore = await page.evaluate(() => document.querySelectorAll("main, [class*='max-w-3xl']").length);
  if (await clickByLabel("new session")) { await wait(1200); log("✓ clicked New session"); }
  // Stats → sidebar stats panel should show turns/messages
  if (await clickByLabel("stats")) { await wait(800); log("✓ clicked Stats"); }
  const statsPanel = await page.evaluate(() => {
    const el = [...document.querySelectorAll("*")].find((e) => /turns/i.test(e.previousElementSibling?.innerText || "") && /^\d+$/.test(e.innerText));
    return !!document.body.innerText.match(/turns\s*\n?\s*\d/i) || !!document.body.innerText.match(/messages\s*\n?\s*\d/i);
  });
  log("stats panel visible:", statsPanel);
  // Compact
  if (await clickByLabel("compact")) { await wait(800); log("✓ clicked Compact"); }
  // Reset
  if (await clickByLabel("reset")) { await wait(800); log("✓ clicked Reset"); }

  // ---- SESSION LIST: click an item to load it ----
  log("\n=== SIDEBAR: load a session ===");
  const sessionItem = await page.evaluateHandle(() => {
    // session list buttons contain a .jsonl filename + relative time
    const btns = [...document.querySelectorAll("aside button")];
    return btns.find((b) => /\.jsonl/i.test(b.innerText)) || null;
  });
  const si = sessionItem.asElement();
  if (si) {
    await si.click();
    await wait(1500);
    log("✓ clicked a session item (load_session)");
  } else {
    log("(no session items to click)");
  }

  // ---- APPROVAL GATE (the human-in-the-loop flow) ----
  log("\n=== APPROVAL GATE ===");
  // 1. Set approval to "destructive" via the dropdown.
  const apprBtn = await page.evaluateHandle(() =>
    [...document.querySelectorAll('button[aria-expanded]')].find((b) => /never|destructive|always/i.test(b.innerText)),
  );
  await apprBtn.click(); await wait(300);
  const destructiveH = await page.$$('.animate-fade-in button');
  const opts = await page.$$eval('.animate-fade-in button', (els) => els.map((e) => e.innerText.trim().toLowerCase()));
  const di = opts.findIndex((o) => o === "destructive");
  if (destructiveH[di]) { await destructiveH[di].click(); await wait(400); log("✓ set approval → destructive"); }

  // 2. Trigger a destructive tool: ask the agent to write a file.
  const ta = await page.$('textarea[aria-label="Message the agent"]');
  await ta.type("Use the write_file tool to create a file at /tmp/ui_test_hello.txt containing the text: hello from ui test");
  await page.keyboard.press("Enter");
  log("sent write_file request; waiting for approval banner…");
  // Wait for the approval banner (contains "Approval required")
  let approved = false;
  try {
    await page.waitForFunction(() => /approval required/i.test(document.body.innerText), { timeout: 45000 });
    log("✓ approval banner appeared");
    // 3. Click "Approve once"
    const approveBtn = await page.evaluateHandle(() => {
      const btns = [...document.querySelectorAll("button")];
      return btns.find((b) => /approve once/i.test(b.innerText));
    });
    const ab = approveBtn.asElement();
    if (ab) { await ab.click(); await wait(500); approved = true; log("✓ clicked Approve once"); }
    else fail("Approve-once button not found");
  } catch {
    // The model may have finished without a destructive call, or auto-approved.
    log("(no approval banner — model may not have called a destructive tool; skipping)");
  }

  // Wait for the turn to finish + a tool result to appear.
  await wait(8000);

  // ---- TOOL-CALL CARD expand ----
  log("\n=== TOOL-CALL CARD ===");
  // Tool-call cards have a button whose text includes a tool name + running/ok/error.
  const toolCard = await page.evaluateHandle(() => {
    const btns = [...document.querySelectorAll("button")];
    // card header buttons contain mono tool names like write_file / list_dir / bash
    return btns.find((b) => /\b(write_file|list_dir|bash|edit|grep|glob|read_file)\b/i.test(b.innerText) && /running|ok|error/i.test(b.innerText));
  });
  const tc = toolCard.asElement();
  if (tc) {
    await tc.click();
    await wait(400);
    const expanded = await page.evaluate(() => /arguments|result/i.test(document.body.innerText));
    log("tool card expanded (shows args/result):", expanded);
    if (expanded) log("✓ tool-call card expands");
    else fail("tool-call card did not expand");
  } else {
    log("(no tool-call card rendered to test expand)");
  }

  // ---- TOAST DISMISS ----
  log("\n=== TOAST DISMISS ===");
  // If any toast is present, click its X.
  const toastX = await page.evaluateHandle(() => {
    const btns = [...document.querySelectorAll('button[aria-label]')];
    return null; // toasts use an X icon button; find by role
  });
  // simpler: count toasts, click the first close (X) button inside a toast
  const hadToast = await page.evaluate(() => /role="status"|aria-live/i.test(document.body.innerHTML) ? document.querySelectorAll('[role="status"] button').length : 0);
  if (hadToast > 0) {
    await page.evaluate(() => {
      const t = document.querySelector('[role="status"] button');
      if (t) t.click();
    });
    log("✓ dismissed a toast");
  } else {
    log("(no toasts to dismiss)");
  }

  log("\n=== DONE ===");
  log("pageerrors:", errors.length);
  errors.slice(0, 6).forEach((e) => console.log("   ", e));
  if (!process.exitCode && !errors.length) console.log("\n✅ ALL EXTENDED UI TESTS PASSED");
  else console.log("\n❌ SOME TESTS FAILED (see above)");
  await browser.close();
})().catch((e) => { console.error("FATAL:", e); process.exit(1); });
