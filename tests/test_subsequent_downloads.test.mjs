import { test, before, after } from "node:test";
import assert from "node:assert/strict";
import http from "node:http";
import path from "node:path";
import os from "node:os";
import fs from "node:fs";
import { chromium } from "playwright";

const extDir = path.resolve("browser-extensions/Chrome-Edge-Brave-Opera");
let server;
let receivedBridgeDownloads = [];
let browserContext;
let popupPage;
let userDataDir;

before(async () => {
  // 1. Mock CosmoDownloader HTTP Bridge on port 42424
  server = http.createServer((req, res) => {
    let body = "";
    req.on("data", (chunk) => { body += chunk; });
    req.on("end", () => {
      if (req.url === "/health") {
        res.writeHead(200, { "Content-Type": "application/json" });
        res.end(JSON.stringify({ status: "ok" }));
        return;
      }
      if (req.url === "/download") {
        try {
          receivedBridgeDownloads.push(JSON.parse(body));
        } catch (_) {
          receivedBridgeDownloads.push({ raw: body });
        }
        res.writeHead(200, {
          "Content-Type": "application/json",
          "Access-Control-Allow-Origin": "*",
          "Access-Control-Allow-Headers": "*"
        });
        res.end(JSON.stringify({ success: true, message: "Queued" }));
        return;
      }
      res.writeHead(404);
      res.end();
    });
  });

  await new Promise((resolve) => server.listen(42424, "127.0.0.1", resolve));

  // 2. Launch browser with extension
  userDataDir = fs.mkdtempSync(path.join(os.tmpdir(), "pw-ext-test-"));
  browserContext = await chromium.launchPersistentContext(userDataDir, {
    headless: false,
    args: [
      "--headless=new",
      `--disable-extensions-except=${extDir}`,
      `--load-extension=${extDir}`,
      "--no-sandbox"
    ]
  });

  // Get extension ID
  let [background] = browserContext.serviceWorkers();
  if (!background) {
    background = await browserContext.waitForEvent("serviceworker");
  }
  const extId = new URL(background.url()).hostname;

  popupPage = await browserContext.newPage();
  await popupPage.goto(`chrome-extension://${extId}/popup.html`);
});

after(async () => {
  if (browserContext) await browserContext.close();
  if (server) await new Promise((resolve) => server.close(resolve));
  if (userDataDir && fs.existsSync(userDataDir)) {
    fs.rmSync(userDataDir, { recursive: true, force: true });
  }
});

test("First download request successfully reaches desktop bridge", async () => {
  receivedBridgeDownloads = [];
  const res = await popupPage.evaluate(async () => {
    return new Promise((resolve) => {
      chrome.runtime.sendMessage({
        action: "downloadVideo",
        url: "http://example.com/video1.mp4",
        pageUrl: "http://example.com/watch?v=1",
        filename: "video1.mp4",
        quality: "1080p"
      }, resolve);
    });
  });

  assert.equal(res.success, true);
  assert.equal(receivedBridgeDownloads.length, 1);
  assert.equal(receivedBridgeDownloads[0].url, "http://example.com/video1.mp4");
});

test("Subsequent download request on same page with different quality is NOT stuck or suppressed", async () => {
  const prevCount = receivedBridgeDownloads.length;
  const res = await popupPage.evaluate(async () => {
    return new Promise((resolve) => {
      chrome.runtime.sendMessage({
        action: "downloadVideo",
        url: "http://example.com/video1.mp4",
        pageUrl: "http://example.com/watch?v=1",
        filename: "video1_audio.mp3",
        quality: "audio"
      }, resolve);
    });
  });

  assert.equal(res.success, true, "Subsequent download request must succeed");
  assert.equal(res.deduplicated, undefined, "Subsequent quality request must not be deduplicated/dropped");
  assert.equal(receivedBridgeDownloads.length, prevCount + 1, "Bridge server must receive subsequent download");
});

test("Multiple different files downloaded from the same referrer page are all intercepted", async () => {
  const startCount = receivedBridgeDownloads.length;
  // Trigger file A via addUrl
  const resA = await popupPage.evaluate(async () => {
    return new Promise((resolve) => {
      chrome.runtime.sendMessage({
        action: "addUrl",
        url: "http://example.com/archive_part1.zip"
      }, resolve);
    });
  });
  assert.equal(resA.success, true);

  // Trigger file B via addUrl immediately after
  const resB = await popupPage.evaluate(async () => {
    return new Promise((resolve) => {
      chrome.runtime.sendMessage({
        action: "addUrl",
        url: "http://example.com/archive_part2.zip"
      }, resolve);
    });
  });
  assert.equal(resB.success, true);
  assert.equal(receivedBridgeDownloads.length, startCount + 2);
});
