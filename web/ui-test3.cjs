// Focused approval-gate test: set destructive, ask the agent to run a bash
// command (bash is always destructive), then actually CLICK "Approve once" when
// the banner appears — exercising the human-in-the-loop buttons end-to-end.
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

  log("loading…");
  await page.goto(BASE, { waitUntil: "networkidle2", timeout: 45000 });
  await page.waitForSelector('button[aria-expanded]', { timeout: 30000 });
  await wait(2500);

  // 1. Ensure approval is "destructive" (set via dropdown).
  log("setting approval → destructive");
  const apprBtn = await page.evaluateHandle(() =>
    [...document.querySelectorAll('button[aria-expanded]')].find((b) => /never|destructive|always/i.test(b.innerText)),
  );
  await apprBtn.click(); await wait(300);
  const opts = await page.$$eval('.animate-fade-in button', (els) => els.map((e) => e.innerText.trim().toLowerCase()));
  const di = opts.findIndex((o) => o === "destructive");
  const dh = await page.$$('.animate-fade-in button');
  if (dh[di]) { await dh[di].click(); await wait(400); }
  // close any stray dropdown
  await page.mouse.click(640, 400); await wait(200);

  // 2. Ask the agent to run a bash command (triggers the approval gate).
  const ta = await page.$('textarea[aria-label="Message the agent"]');
  await ta.type("Run this exact bash command and tell me its output: echo APPROVAL_WORKS");
  await page.keyboard.press("Enter");
  log("sent bash request; waiting for approval banner…");

  // 3. Wait for the approval banner, then click "Approve once".
  let clicked = false;
  try {
    await page.waitForFunction(() => /approval required/i.test(document.body.innerText), { timeout: 60000 });
    log("✓ approval banner appeared");
    // Which tool is asking?
    const toolName = await page.evaluate(() => {
      const m = document.body.innerText.match(/(bash|write_file|edit|patch|bulk)/i);
      return m ? m[1] : "?";
    });
    log("  tool requesting approval:", toolName);

    const approveBtn = await page.evaluateHandle(() => {
      const btns = [...document.querySelectorAll("button")];
      return btns.find((b) => /approve once/i.test(b.innerText));
    });
    const ab = approveBtn.asElement();
    if (ab) {
      await ab.click();
      clicked = true;
      log("✓ clicked 'Approve once'");
    } else {
      fail("'Approve once' button not found in banner");
    }
  } catch {
    fail("no approval banner appeared within 60s (model may not have called bash)");
  }

  if (clicked) {
    // 4. Verify the banner disappears + the tool result / output appears.
    await wait(1500);
    const bannerGone = await page.evaluate(() => !/approval required/i.test(document.body.innerText));
    log("banner disappeared after approve:", bannerGone);
    if (!bannerGone) fail("approval banner did not disappear after clicking Approve");

    // Wait for the command output to stream back.
    try {
      await page.waitForFunction(() => /APPROVAL_WORKS/i.test(document.body.innerText), { timeout: 30000 });
      log("✓ bash output 'APPROVAL_WORKS' appeared after approval");
    } catch {
      fail("did not see bash output 'APPROVAL_WORKS' after approval");
    }
  }

  log("\n=== DONE ===");
  log("pageerrors:", errors.length);
  errors.slice(0, 6).forEach((e) => console.log("   ", e));
  if (!process.exitCode && !errors.length) console.log("\n✅ APPROVAL GATE TEST PASSED");
  else console.log("\n❌ APPROVAL GATE TEST FAILED (see above)");
  await browser.close();
})().catch((e) => { console.error("FATAL:", e); process.exit(1); });
