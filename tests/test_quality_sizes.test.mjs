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

const mockFormatsDatabase = {
  // Video 1: 720p max
  "jpurmr0Sovs": {
    video_id: "jpurmr0Sovs",
    title: "Short 720p HD Upload Test",
    is_live: false,
    qualities: [
      { label: "720p", format: "720p", height: 720, format_id: "136+140", size: 1320990, size_formatted: "1.3 MB", is_approx: false },
      { label: "480p", format: "480p", height: 480, format_id: "135+140", size: 750000, size_formatted: "750 KB", is_approx: false },
      { label: "360p", format: "360p", height: 360, format_id: "134+140", size: 450000, size_formatted: "450 KB", is_approx: false }
    ],
    audio: { label: "Audio only", format: "audio", height: null, format_id: "140", size: 135000, size_formatted: "135 KB", is_approx: false },
    best: { label: "Auto · 720p · 1.3 MB", format: "best", height: 720, format_id: "136+140", size: 1320990, size_formatted: "1.3 MB", is_approx: false }
  },
  // Video 2: 1080p video with different sizes
  "video2_1080p": {
    video_id: "video2_1080p",
    title: "1080p High Definition Sample",
    is_live: false,
    qualities: [
      { label: "1080p", format: "1080p", height: 1080, format_id: "137+140", size: 68500000, size_formatted: "68 MB", is_approx: false },
      { label: "720p", format: "720p", height: 720, format_id: "136+140", size: 34200000, size_formatted: "34 MB", is_approx: false },
      { label: "480p", format: "480p", height: 480, format_id: "135+140", size: 17100000, size_formatted: "17 MB", is_approx: false }
    ],
    audio: { label: "Audio only", format: "audio", height: null, format_id: "140", size: 5200000, size_formatted: "5.2 MB", is_approx: false },
    best: { label: "Auto · 1080p · 68 MB", format: "best", height: 1080, format_id: "137+140", size: 68500000, size_formatted: "68 MB", is_approx: false }
  },
  // Video 3: Live stream (no sizes)
  "live_stream": {
    video_id: "live_stream",
    title: "24/7 Live Stream Channel",
    is_live: true,
    qualities: [
      { label: "1080p", format: "1080p", height: 1080, format_id: "96", size: null, size_formatted: "—", is_approx: false },
      { label: "720p", format: "720p", height: 720, format_id: "95", size: null, size_formatted: "—", is_approx: false }
    ],
    audio: { label: "Audio only", format: "audio", height: null, format_id: "audio", size: null, size_formatted: "—", is_approx: false },
    best: { label: "Auto · 1080p", format: "best", height: 1080, format_id: "96", size: null, size_formatted: "—", is_approx: false }
  }
};

before(async () => {
  server = http.createServer((req, res) => {
    const url = new URL(req.url, `http://127.0.0.1:${serverPort}`);
    if (url.pathname === "/" || url.pathname === "/index.html" || url.pathname === "/watch") {
      res.writeHead(200, { "Content-Type": "text/html; charset=utf-8" });
      res.end(`<!DOCTYPE html>
<html>
<head>
  <meta charset="utf-8">
  <title>Cosmo Quality Size Tests</title>
  <link rel="stylesheet" href="/content.css">
  <style>
    body { margin: 0; padding: 20px; background: #0f172a; color: #fff; font-family: sans-serif; }
    .player-container { position: relative; width: 640px; height: 360px; background: #1e293b; margin-bottom: 20px; }
    video { width: 100%; height: 100%; }
    .sidebar { margin-top: 20px; }
    .sidebar a { color: #60a5fa; cursor: pointer; display: block; margin: 4px 0; }
  </style>
</head>
<body>
  <div class="player-container" id="player">
    <video id="vid" controls src="/sample.mp4"></video>
  </div>
  <div class="sidebar">
    <a id="link-video1">Switch to 720p Video (jpurmr0Sovs)</a>
    <a id="link-video2">Switch to 1080p Video (video2_1080p)</a>
    <a id="link-live">Switch to Live Stream</a>
    <a id="link-neterr">Switch to Network Error Video</a>
  </div>

  <script>
    window.mockDb = ${JSON.stringify(mockFormatsDatabase)};
    window.sentMessages = [];
    window.chrome = {
      runtime: {
        getURL: (p) => "/" + p,
        sendMessage: (msg, callback) => {
          window.sentMessages.push(msg);
          if (msg.action === "getVideoFormats") {
            const url = msg.url || "";
            setTimeout(() => {
              if (url.includes("jpurmr0Sovs")) {
                callback({ success: true, formats: window.mockDb["jpurmr0Sovs"] });
              } else if (url.includes("video2_1080p")) {
                callback({ success: true, formats: window.mockDb["video2_1080p"] });
              } else if (url.includes("live_stream")) {
                callback({ success: true, formats: window.mockDb["live_stream"] });
              } else if (url.includes("neterr")) {
                callback({ success: false, error: "Network timeout fetching video formats" });
              } else {
                callback({ success: true, formats: window.mockDb["video2_1080p"] });
              }
            }, 50);
            return true;
          }
          if (callback) callback({ success: true });
        }
      }
    };

    // Navigation simulations
    document.getElementById("link-video1").addEventListener("click", () => {
      history.pushState({}, "", "/watch?v=jpurmr0Sovs");
      window.dispatchEvent(new CustomEvent("yt-navigate-finish"));
    });
    document.getElementById("link-video2").addEventListener("click", () => {
      history.pushState({}, "", "/watch?v=video2_1080p");
      window.dispatchEvent(new CustomEvent("yt-navigate-finish"));
    });
    document.getElementById("link-live").addEventListener("click", () => {
      history.pushState({}, "", "/watch?v=live_stream");
      window.dispatchEvent(new CustomEvent("yt-navigate-finish"));
    });
    document.getElementById("link-neterr").addEventListener("click", () => {
      history.pushState({}, "", "/watch?v=neterr");
      window.dispatchEvent(new CustomEvent("yt-navigate-finish"));
    });
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

test("Test 1: Two different videos show different sizes", async () => {
  const page = await browser.newPage({ viewport: { width: 1280, height: 900 } });
  
  // 1. Load Video 1 (jpurmr0Sovs)
  await page.goto(`http://127.0.0.1:${serverPort}/watch?v=jpurmr0Sovs`, { waitUntil: "domcontentloaded" });
  const btn = page.locator("#player .cosmo-video-download-btn");
  await btn.waitFor({ state: "visible", timeout: 4000 });
  await btn.click();

  // Wait for formats to be populated
  await page.waitForTimeout(150);
  const v1Items = await page.locator("#player .cosmo-quality-item").allTextContents();
  const v1Text = v1Items.join(" | ");

  // 2. Switch to Video 2 (video2_1080p)
  await page.click("#link-video2");
  await page.waitForTimeout(150);

  // Open menu for Video 2
  await btn.click();
  const v2Items = await page.locator("#player .cosmo-quality-item").allTextContents();
  const v2Text = v2Items.join(" | ");

  assert.notEqual(v1Text, v2Text, "Two different videos must display different sizes");
  assert.ok(v1Text.includes("1.3 MB"), "Video 1 must show 1.3 MB");
  assert.ok(v2Text.includes("68 MB"), "Video 2 must show 68 MB");

  await page.close();
});

test("Test 2: A video with a maximum of 720p has no 1080p row", async () => {
  const page = await browser.newPage({ viewport: { width: 1280, height: 900 } });
  await page.goto(`http://127.0.0.1:${serverPort}/watch?v=jpurmr0Sovs`, { waitUntil: "domcontentloaded" });

  const btn = page.locator("#player .cosmo-video-download-btn");
  await btn.waitFor({ state: "visible", timeout: 4000 });
  await btn.click();

  await page.waitForTimeout(150);
  const rows = await page.locator("#player .cosmo-quality-item").allTextContents();
  const has1080p = rows.some((r) => r.startsWith("1080p") || r.trim() === "1080p");
  assert.equal(has1080p, false, "A video with maximum 720p must NOT have a 1080p row");

  const has720p = rows.some((r) => r.includes("720p"));
  assert.equal(has720p, true, "Must include 720p row");

  await page.close();
});

test("Test 3: Open one video, then click another in sidebar without reloading: menu shows new video sizes", async () => {
  const page = await browser.newPage({ viewport: { width: 1280, height: 900 } });
  await page.goto(`http://127.0.0.1:${serverPort}/watch?v=jpurmr0Sovs`, { waitUntil: "domcontentloaded" });

  const btn = page.locator("#player .cosmo-video-download-btn");
  await btn.waitFor({ state: "visible", timeout: 4000 });

  // Open menu on video 1
  await btn.click();
  await page.waitForTimeout(150);
  let rows = await page.locator("#player .cosmo-quality-item").allTextContents();
  assert.ok(rows.join(" ").includes("1.3 MB"));

  // Click sidebar link without reloading page
  await page.click("#link-video2");
  await page.waitForTimeout(150);

  // Re-open menu
  await btn.click();
  rows = await page.locator("#player .cosmo-quality-item").allTextContents();
  const text = rows.join(" ");

  assert.ok(!text.includes("1.3 MB"), "Old video's sizes must be cleared immediately");
  assert.ok(text.includes("68 MB"), "New video's sizes must be displayed");

  await page.close();
});

test("Test 5: Live stream: no sizes and no wrong numbers", async () => {
  const page = await browser.newPage({ viewport: { width: 1280, height: 900 } });
  await page.goto(`http://127.0.0.1:${serverPort}/watch?v=live_stream`, { waitUntil: "domcontentloaded" });

  const btn = page.locator("#player .cosmo-video-download-btn");
  await btn.waitFor({ state: "visible", timeout: 4000 });
  await btn.click();

  await page.waitForTimeout(150);
  const metaSpans = await page.locator("#player .cosmo-quality-item .meta").allTextContents();
  for (const meta of metaSpans) {
    const clean = meta.trim();
    assert.ok(clean === "" || clean === "—", `Live stream must not display numeric file sizes: found '${clean}'`);
  }

  await page.close();
});

test("Test 6: Network error: the menu still opens and shows 'Sizes unavailable'", async () => {
  const page = await browser.newPage({ viewport: { width: 1280, height: 900 } });
  await page.goto(`http://127.0.0.1:${serverPort}/watch?v=neterr`, { waitUntil: "domcontentloaded" });

  const btn = page.locator("#player .cosmo-video-download-btn");
  await btn.waitFor({ state: "visible", timeout: 4000 });
  await btn.click();

  await page.waitForTimeout(150);
  const menu = page.locator("#player .cosmo-quality-menu");
  const isMenuVisible = await menu.evaluate((el) => window.getComputedStyle(el).display !== "none");
  assert.equal(isMenuVisible, true, "Menu must still open on network error");

  const unavail = page.locator("#player .cosmo-sizes-unavailable");
  await unavail.waitFor({ state: "visible", timeout: 2000 });
  const text = await unavail.textContent();
  assert.ok(text.includes("Sizes unavailable"), "Must display 'Sizes unavailable' line");

  await page.close();
});

test("Test 7: Search the code: none of the old placeholder values remain", () => {
  const extFiles = [
    path.join(extDir, "content.js"),
    path.join(extDir, "background.js"),
    path.join(extDir, "content.css"),
    path.join(rootDir, "Cosmo downloader NEWWW.html")
  ];

  const forbidden = ["148 MB", "82 MB", "41 MB", "24 MB", "9 MB"];
  for (const f of extFiles) {
    if (!fs.existsSync(f)) continue;
    const content = fs.readFileSync(f, "utf-8");
    for (const phrase of forbidden) {
      assert.ok(!content.includes(phrase), `File ${path.basename(f)} still contains forbidden placeholder '${phrase}'`);
    }
  }
});
