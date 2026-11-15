import { chromium } from "playwright";
import path from "node:path";
import os from "node:os";
import fs from "node:fs";

async function run() {
  const extDir = path.resolve("browser-extensions/Chrome-Edge-Brave-Opera");
  const userDataDir = fs.mkdtempSync(path.join(os.tmpdir(), "pw-rickroll-ext-"));
  const screenshotPath = path.resolve("rickroll_download_options.png");

  console.log("🎬 Launching Playwright Chromium with Cosmo Downloader extension...");
  const context = await chromium.launchPersistentContext(userDataDir, {
    headless: false,
    args: [
      `--disable-extensions-except=${extDir}`,
      `--load-extension=${extDir}`,
      "--autoplay-policy=no-user-gesture-required",
      "--no-sandbox",
      "--window-size=1280,800"
    ],
    viewport: { width: 1280, height: 800 }
  });

  try {
    const page = await context.newPage();
    console.log("🌐 Navigating to Rickroll on YouTube (Never Gonna Give You Up)...");
    await page.goto("https://www.youtube.com/watch?v=dQw4w9WgXcQ", {
      waitUntil: "domcontentloaded"
    });

    // Handle potential YouTube consent dialogs
    try {
      const consent = page.locator("button:has-text(\"Accept all\"), button:has-text(\"Reject all\"), button:has-text(\"I agree\")").first();
      if (await consent.isVisible({ timeout: 4000 })) {
        console.log("Dismissing YouTube consent dialog...");
        await consent.click();
      }
    } catch (_) {}

    // Wait for Cosmo Download Button to inject into YouTube player
    console.log("🔍 Waiting for Cosmo Download Button on video player...");
    const btn = page.locator(".cosmo-video-download-btn");
    await btn.waitFor({ state: "visible", timeout: 12000 });
    console.log("✨ Cosmo Download Button is VISIBLE on player!");

    // Hover over player/button to show controls
    await btn.hover();
    await page.waitForTimeout(1000);

    // Click button to toggle dropdown menu
    console.log("🖱️ Clicking Download button to open quality options menu...");
    await btn.click();

    // Verify quality menu is visible
    const menu = page.locator(".cosmo-quality-menu");
    await menu.waitFor({ state: "visible", timeout: 5000 });
    console.log("📋 Quality Dropdown Menu opened!");

    // Read and list all quality options
    const items = await page.locator(".cosmo-quality-item").all();
    console.log(`Found ${items.length} quality options:`);
    for (const item of items) {
      const format = await item.getAttribute("data-format");
      const label = (await item.innerText()).replace(/\n/g, " ");
      console.log(`  - [${format}] ${label}`);
    }

    // Capture screenshot of the open options menu on YouTube
    await page.screenshot({ path: screenshotPath });
    console.log(`📸 Screenshot saved to ${screenshotPath}`);

    // --- Consecutive download tests on YouTube player ---
    const formatsToTest = ["1080p", "720p", "audio"];
    console.log(`\n🎯 Testing consecutive downloads on YouTube player (${formatsToTest.join(", ")})...`);
    for (let i = 0; i < formatsToTest.length; i++) {
      const fmt = formatsToTest[i];
      if (!await menu.isVisible()) {
        await btn.click();
        await menu.waitFor({ state: "visible", timeout: 3000 });
      }
      const option = page.locator(`.cosmo-quality-item[data-format="${fmt}"]`);
      if (await option.isVisible()) {
        await option.click();
        await page.waitForTimeout(1000);
        const feedback = await btn.innerText();
        console.log(`  📥 [${i + 1}/3] Selected ${fmt} -> Feedback: "${feedback}"`);
      }
    }

    console.log("\n⚡ Step 3: Performing 3x Download and Cancel Cycles...");
    let [background] = context.serviceWorkers();
    if (!background) {
      background = await context.waitForEvent("serviceworker", { timeout: 5000 }).catch(() => null);
    }

    let extId;
    if (background) {
      extId = new URL(background.url()).hostname;
    }

    const popupPage = await context.newPage();
    if (background) {
      await popupPage.goto(`chrome-extension://${extId}/popup.html`);
    } else {
      await popupPage.goto("about:blank");
    }

    // Run 3 consecutive download & cancel cycles
    const threeDownloadsResults = await popupPage.evaluate(async () => {
      const results = [];
      for (let i = 1; i <= 3; i++) {
        const dummyBlob = new Blob(["Rickroll payload data ".repeat(1024 * 50)], { type: "application/octet-stream" });
        const blobUrl = URL.createObjectURL(dummyBlob);

        const startDl = performance.now();
        const dlId = await new Promise((resolve, reject) => {
          chrome.downloads.download({
            url: blobUrl,
            filename: `rickroll_download_test_${i}.bin`,
            saveAs: false
          }, (id) => {
            if (chrome.runtime.lastError) reject(chrome.runtime.lastError);
            else resolve(id);
          });
        });
        const dlInitiateTime = performance.now() - startDl;

        // Immediately cancel
        const startCancel = performance.now();
        await new Promise((resolve) => {
          chrome.downloads.cancel(dlId, () => resolve());
        });
        const cancelTime = performance.now() - startCancel;

        // Query status to confirm cancellation
        const [item] = await new Promise((resolve) => {
          chrome.downloads.search({ id: dlId }, resolve);
        });

        // Clean up
        await new Promise((resolve) => {
          chrome.downloads.erase({ id: dlId }, () => resolve());
        });
        URL.revokeObjectURL(blobUrl);

        results.push({
          iteration: i,
          initiateTimeMs: dlInitiateTime,
          cancelTimeMs: cancelTime,
          state: item ? item.state : "interrupted"
        });
      }
      return results;
    });

    console.log("=================================================");
    console.log("🛑 3x DOWNLOAD & CANCEL TIMING RESULTS");
    console.log("=================================================");
    for (const r of threeDownloadsResults) {
      console.log(`  Download #${r.iteration}: Started in ${r.initiateTimeMs.toFixed(2)} ms -> Cancelled in ${r.cancelTimeMs.toFixed(2)} ms (Final State: ${r.state})`);
    }
    console.log("=================================================");

    console.log("\n⚡ Benchmarking Browser Download Pause, Resume, and Cancel Times (n=3)...");
    const benchmark = await popupPage.evaluate(async () => {
      const runs = 3;
      const pauseTimes = [];
      const resumeTimes = [];
      const cancelTimes = [];

      for (let i = 0; i < runs; i++) {
        const dummyBlob = new Blob(["0".repeat(1024 * 1024 * 5)], { type: "application/octet-stream" });
        const blobUrl = URL.createObjectURL(dummyBlob);

        const dlId = await new Promise((resolve, reject) => {
          chrome.downloads.download({
            url: blobUrl,
            filename: `benchmark_stream_${i}.bin`,
            saveAs: false
          }, (id) => {
            if (chrome.runtime.lastError) reject(chrome.runtime.lastError);
            else resolve(id);
          });
        });

        // 1. Measure Pause Time
        const tPauseStart = performance.now();
        await new Promise((resolve) => {
          chrome.downloads.pause(dlId, () => resolve());
        });
        pauseTimes.push(performance.now() - tPauseStart);

        // 2. Measure Resume Time
        const tResumeStart = performance.now();
        await new Promise((resolve) => {
          chrome.downloads.resume(dlId, () => resolve());
        });
        resumeTimes.push(performance.now() - tResumeStart);

        // 3. Measure Cancel Time
        const tCancelStart = performance.now();
        await new Promise((resolve) => {
          chrome.downloads.cancel(dlId, () => resolve());
        });
        cancelTimes.push(performance.now() - tCancelStart);

        // Erase record
        await new Promise((resolve) => {
          chrome.downloads.erase({ id: dlId }, () => resolve());
        });
        URL.revokeObjectURL(blobUrl);
      }

      const avg = (arr) => arr.reduce((a, b) => a + b, 0) / arr.length;
      const min = (arr) => Math.min(...arr);
      const max = (arr) => Math.max(...arr);

      return {
        pause: { avg: avg(pauseTimes), min: min(pauseTimes), max: max(pauseTimes) },
        resume: { avg: avg(resumeTimes), min: min(resumeTimes), max: max(resumeTimes) },
        cancel: { avg: avg(cancelTimes), min: min(cancelTimes), max: max(cancelTimes) }
      };
    });

    await popupPage.close();

    console.log("=================================================");
    console.log("⏱️  BROWSER DOWNLOAD LIFECYCLE TIMING RESULTS (n=3)");
    console.log("=================================================");
    console.log(`⏸️  Pause Time:  Avg: ${benchmark.pause.avg.toFixed(2)} ms (Min: ${benchmark.pause.min.toFixed(2)} ms, Max: ${benchmark.pause.max.toFixed(2)} ms)`);
    console.log(`▶️  Resume Time: Avg: ${benchmark.resume.avg.toFixed(2)} ms (Min: ${benchmark.resume.min.toFixed(2)} ms, Max: ${benchmark.resume.max.toFixed(2)} ms)`);
    console.log(`⏹️  Cancel Time: Avg: ${benchmark.cancel.avg.toFixed(2)} ms (Min: ${benchmark.cancel.min.toFixed(2)} ms, Max: ${benchmark.cancel.max.toFixed(2)} ms)`);
    console.log("=================================================");

    console.log("🎶 Rickroll video verification, 3x download & cancel, and lifecycle benchmarks completed successfully!");
  } finally {
    await context.close();
    fs.rmSync(userDataDir, { recursive: true, force: true });
  }
}

run().catch((err) => {
  console.error("❌ Error running Rickroll Playwright test:", err);
  process.exit(1);
});
