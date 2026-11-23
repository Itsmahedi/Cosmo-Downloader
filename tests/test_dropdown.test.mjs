import { test, before, after } from "node:test";
import assert from "node:assert/strict";
import http from "node:http";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { chromium } from "playwright";

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const rootDir = path.resolve(__dirname, "..");
const extDir = path.join(rootDir, "browser-extensions", "Chrome-Edge-Brave-Opera");

let server;
let serverPort;
let browser;

function getExecutablePath() {
  if (process.env.PLAYWRIGHT_CHROMIUM_EXECUTABLE_PATH && fs.existsSync(process.env.PLAYWRIGHT_CHROMIUM_EXECUTABLE_PATH)) {
    return process.env.PLAYWRIGHT_CHROMIUM_EXECUTABLE_PATH;
  }
  return undefined;
}

before(async () => {
  // 1. Create a lightweight local HTTP server
  server = http.createServer((req, res) => {
    const url = new URL(req.url, `http://127.0.0.1:${serverPort}`);
    if (url.pathname === "/" || url.pathname === "/index.html") {
      res.writeHead(200, { "Content-Type": "text/html; charset=utf-8" });
      res.end(`<!DOCTYPE html>
<html>
<head>
  <meta charset="utf-8">
  <title>Cosmo Video Test Page</title>
  <link rel="stylesheet" href="/content.css">
  <style>
    body { margin: 0; padding: 40px; background: #0f172a; color: #fff; font-family: sans-serif; min-height: 1200px; }
    .player-box { position: relative; width: 640px; height: 360px; background: #1e293b; margin-bottom: 50px; }
    .player-box video { width: 100%; height: 100%; object-fit: contain; }
    #overflow-player { position: absolute; bottom: 20px; left: 40px; height: 180px; width: 360px; }
  </style>
</head>
<body>
  <h1>Cosmo Video Test</h1>
  <div class="player-box" id="main-player">
    <video id="test-video" width="640" height="360" controls src="/sample.mp4"></video>
  </div>
  <div class="player-box" id="overflow-player">
    <video id="overflow-video" width="640" height="360" controls src="/sample-bottom.mp4"></video>
  </div>

  <script>
    window.sentMessages = [];
    window.chrome = {
      runtime: {
        getURL: (p) => "/" + p,
        sendMessage: (msg) => { window.sentMessages.push(msg); }
      }
    };
  </script>
  <script src="/content.js"></script>
</body>
</html>`);
    } else if (url.pathname === "/overlap") {
      res.writeHead(200, { "Content-Type": "text/html; charset=utf-8" });
      res.end(`<!DOCTYPE html>
<html>
<head>
  <meta charset="utf-8">
  <title>Cosmo Overlap Test</title>
  <link rel="stylesheet" href="/content.css">
  <style>
    body { margin: 0; padding: 40px; background: #0f172a; }
    .player-box { position: relative; width: 640px; height: 360px; }
    .player-box video { width: 100%; height: 100%; }
  </style>
</head>
<body>
  <div class="player-box" id="overlap-player">
    <video width="640" height="360" controls src="/sample.mp4"></video>
  </div>
  <script>
    window.sentMessages = [];
    window.chrome = {
      runtime: {
        getURL: (p) => "/" + p,
        sendMessage: (msg, cb) => {
          window.sentMessages.push(msg);
          if (msg.action === "getVideoFormats" && cb) {
            cb({
              success: true,
              formats: {
                video_id: "test_overlap",
                title: "4K Test Video",
                is_live: false,
                qualities: [
                  { label: "2160p", format: "2160p", height: 2160, format_id: "313+251", size: 1331439861, size_formatted: "1.24 GB", is_approx: true },
                  { label: "1080p", format: "1080p", height: 1080, format_id: "137+140", size: 68500000, size_formatted: "65 MB", is_approx: false },
                  { label: "720p", format: "720p", height: 720, format_id: "136+140", size: 34200000, size_formatted: "33 MB", is_approx: false },
                  { label: "480p", format: "480p", height: 480, format_id: "135+140", size: 17100000, size_formatted: "16 MB", is_approx: false },
                  { label: "360p", format: "360p", height: 360, format_id: "134+140", size: 9500000, size_formatted: "9.1 MB", is_approx: false }
                ],
                audio: { label: "Audio only", format: "audio", height: null, format_id: "140", size: 5200000, size_formatted: "5.0 MB", is_approx: false },
                best: { label: "Best available", format: "best", height: 2160, format_id: "313+251", size: 1331439861, size_formatted: "Auto · 2160p · ~1.24 GB", is_approx: true }
              }
            });
            return;
          }
          if (cb) cb({ success: true });
        }
      }
    };
  </script>
  <script src="/content.js"></script>
</body>
</html>`);
    } else if (url.pathname === "/content.css") {
      res.writeHead(200, { "Content-Type": "text/css" });
      res.end(fs.readFileSync(path.join(extDir, "content.css")));
    } else if (url.pathname === "/content.js") {
      res.writeHead(200, { "Content-Type": "application/javascript" });
      res.end(fs.readFileSync(path.join(extDir, "content.js")));
    } else {
      res.writeHead(204);
      res.end();
    }
  });

  await new Promise((resolve) => {
    server.listen(0, "127.0.0.1", () => {
      serverPort = server.address().port;
      resolve();
    });
  });

  // 2. Launch browser
  browser = await chromium.launch({
    executablePath: getExecutablePath(),
    headless: true,
    args: ["--no-sandbox", "--disable-setuid-sandbox"]
  });
});

after(async () => {
  if (browser) await browser.close();
  if (server) await new Promise((r) => server.close(r));
});

test("Video Download Button appears with correct ARIA attributes and hidden menu", async () => {
  const page = await browser.newPage({ viewport: { width: 1280, height: 900 } });
  await page.goto(`http://127.0.0.1:${serverPort}/`, { waitUntil: "domcontentloaded" });

  const btn = page.locator("#main-player .cosmo-video-download-btn");
  await assert.doesNotReject(btn.waitFor({ state: "visible", timeout: 3000 }));

  assert.equal(await btn.getAttribute("aria-haspopup"), "true");
  assert.equal(await btn.getAttribute("aria-expanded"), "false");

  const btnLogo = page.locator("#main-player .cosmo-video-download-btn svg.cosmo-btn-logo");
  await assert.doesNotReject(btnLogo.waitFor({ state: "visible", timeout: 3000 }));
  assert.equal(await page.locator("#main-player .cosmo-menu-header svg.cosmo-menu-logo").count(), 1);
  assert.equal(await page.locator("#main-player .cosmo-video-container img").count(), 0);

  const menu = page.locator("#main-player .cosmo-quality-menu");
  const isMenuVisible = await menu.evaluate((el) => window.getComputedStyle(el).display !== "none");
  assert.equal(isMenuVisible, false, "Menu must be display:none initially");

  await page.close();
});

test("Clicking button toggles menu open and sets aria-expanded true", async () => {
  const page = await browser.newPage({ viewport: { width: 1280, height: 900 } });
  await page.goto(`http://127.0.0.1:${serverPort}/`, { waitUntil: "domcontentloaded" });

  const btn = page.locator("#main-player .cosmo-video-download-btn");
  const menu = page.locator("#main-player .cosmo-quality-menu");

  await btn.click();

  assert.equal(await btn.getAttribute("aria-expanded"), "true");
  const displayAfterOpen = await menu.evaluate((el) => window.getComputedStyle(el).display);
  assert.equal(displayAfterOpen, "block", "Menu display must be block when open");

  // Clicking again closes it
  await btn.click();
  assert.equal(await btn.getAttribute("aria-expanded"), "false");
  const displayAfterClose = await menu.evaluate((el) => window.getComputedStyle(el).display);
  assert.equal(displayAfterClose, "none", "Menu display must be none when closed");

  await page.close();
});

test("Escape key closes the menu and returns focus to button", async () => {
  const page = await browser.newPage({ viewport: { width: 1280, height: 900 } });
  await page.goto(`http://127.0.0.1:${serverPort}/`, { waitUntil: "domcontentloaded" });

  const btn = page.locator("#main-player .cosmo-video-download-btn");
  const menu = page.locator("#main-player .cosmo-quality-menu");

  await btn.click();
  assert.equal(await menu.evaluate((el) => window.getComputedStyle(el).display), "block");

  await page.keyboard.press("Escape");
  assert.equal(await menu.evaluate((el) => window.getComputedStyle(el).display), "none");
  assert.equal(await btn.getAttribute("aria-expanded"), "false");

  await page.close();
});

test("Outside click closes the open menu", async () => {
  const page = await browser.newPage({ viewport: { width: 1280, height: 900 } });
  await page.goto(`http://127.0.0.1:${serverPort}/`, { waitUntil: "domcontentloaded" });

  const btn = page.locator("#main-player .cosmo-video-download-btn");
  const menu = page.locator("#main-player .cosmo-quality-menu");

  await btn.click();
  assert.equal(await menu.evaluate((el) => window.getComputedStyle(el).display), "block");

  // Click on h1 outside container
  await page.click("h1");
  assert.equal(await menu.evaluate((el) => window.getComputedStyle(el).display), "none");
  assert.equal(await btn.getAttribute("aria-expanded"), "false");

  await page.close();
});

test("Every quality option sends downloadVideo message and auto-closes menu", async () => {
  const page = await browser.newPage({ viewport: { width: 1280, height: 900 } });
  await page.goto(`http://127.0.0.1:${serverPort}/`, { waitUntil: "domcontentloaded" });

  const btn = page.locator("#main-player .cosmo-video-download-btn");
  const menu = page.locator("#main-player .cosmo-quality-menu");

  const expectedQualities = [
    { format: "1080p", ext: ".mp4" },
    { format: "720p", ext: ".mp4" },
    { format: "480p", ext: ".mp4" },
    { format: "360p", ext: ".mp4" },
    { format: "audio", ext: ".mp3" },
    { format: "best", ext: ".mp4" }
  ];

  for (const { format, ext } of expectedQualities) {
    // Open menu
    await btn.click();
    assert.equal(await menu.evaluate((el) => window.getComputedStyle(el).display), "block");

    // Click item
    const item = page.locator(`#main-player .cosmo-quality-item[data-format="${format}"]`);
    await item.click();

    // Menu must auto-close
    assert.equal(await menu.evaluate((el) => window.getComputedStyle(el).display), "none");

    // Button text stays "Download" without switching to a sent feedback state
    const btnText = await page.locator("#main-player .cosmo-btn-text").textContent();
    assert.equal(btnText, "Download", `Button text should remain "Download" for format ${format}, got: ${btnText}`);
    const btnClass = (await btn.getAttribute("class")) || "";
    assert.ok(!btnClass.split(/\s+/).includes("sent"), `Button must not have sent class for format ${format}`);

    // Verify last message sent
    const lastMsg = await page.evaluate(() => window.sentMessages[window.sentMessages.length - 1]);
    assert.ok(lastMsg, "No message was sent to extension runtime");
    assert.equal(lastMsg.action, "downloadVideo");
    assert.equal(lastMsg.quality, format);
    assert.ok(lastMsg.filename.endsWith(ext), `Expected filename to end with ${ext}, got: ${lastMsg.filename}`);
  }

  await page.close();
});

test("Menu flips upward when near viewport bottom to prevent overflow", async () => {
  const page = await browser.newPage({ viewport: { width: 1280, height: 700 } });
  await page.goto(`http://127.0.0.1:${serverPort}/`, { waitUntil: "domcontentloaded" });

  // Scroll so overflow-player is visible near bottom
  await page.evaluate(() => {
    document.getElementById("overflow-player").scrollIntoView({ block: "end" });
  });

  const overflowBtn = page.locator("#overflow-player .cosmo-video-download-btn");
  const overflowContainer = page.locator("#overflow-player .cosmo-video-container");

  await overflowBtn.click();
  const classes = await overflowContainer.getAttribute("class");
  assert.ok(classes.includes("flip-up"), `Expected container to have flip-up class, got: ${classes}`);

  await page.close();
});

test("Best available / Auto text never overlaps or wraps, and clicking 480p sends 480p formatId", async () => {
  const page = await browser.newPage({ viewport: { width: 1280, height: 900 } });
  await page.goto(`http://127.0.0.1:${serverPort}/overlap`, { waitUntil: "domcontentloaded" });

  const btn = page.locator("#overlap-player .cosmo-video-download-btn");
  await btn.waitFor({ state: "visible", timeout: 4000 });
  await btn.click();

  const menu = page.locator("#overlap-player .cosmo-quality-menu");
  await menu.waitFor({ state: "visible", timeout: 3000 });

  const layoutIssues = await page.evaluate(() => {
    const m = document.querySelector("#overlap-player .cosmo-quality-menu");
    const menuRect = m.getBoundingClientRect();
    const items = Array.from(document.querySelectorAll("#overlap-player .cosmo-quality-item"));
    const issues = [];
    for (const item of items) {
      const nameEl = item.querySelector(".quality-name");
      const metaEl = item.querySelector(".meta");
      const nameRect = nameEl.getBoundingClientRect();
      const metaRect = metaEl.getBoundingClientRect();
      if (Math.abs(nameRect.top - metaRect.top) >= 6) {
        issues.push(`Row '${nameEl.textContent}' wrapped vertically: nameTop=${nameRect.top}, metaTop=${metaRect.top}`);
      }
      if (nameRect.right + 8 > metaRect.left) {
        issues.push(`Row '${nameEl.textContent}' overlaps meta '${metaEl.textContent}': nameRight=${nameRect.right}, metaLeft=${metaRect.left}`);
      }
      if (metaRect.right > menuRect.right + 1) {
        issues.push(`Row '${nameEl.textContent}' meta overflows menu right: metaRight=${metaRect.right}, menuRight=${menuRect.right}`);
      }
    }
    return { itemCount: items.length, issues };
  });

  assert.equal(layoutIssues.itemCount, 7);
  assert.deepEqual(layoutIssues.issues, []);

  await page.locator('#overlap-player .cosmo-quality-item[data-format="480p"]').click();
  const lastMsg = await page.evaluate(() => {
    return window.sentMessages.filter((m) => m.action === "downloadVideo").pop();
  });
  assert.ok(lastMsg);
  assert.equal(lastMsg.quality, "480p");
  assert.equal(lastMsg.formatId, "135+140");
  assert.equal(lastMsg.size, 17100000);

  await page.close();
});
