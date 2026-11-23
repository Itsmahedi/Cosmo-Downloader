# Cosmo Downloader v1.2.8

A fast, reliable multi-segment download manager for **Windows 10 and Windows 11**, paired with redesigned browser extensions for Google Chrome, Microsoft Edge, Brave, Opera, and Mozilla Firefox.

---

## What's New in v1.2.8

### 🎯 End-to-End Video Quality & Format Selection
- **Exact Quality & Format Forwarding**: Selecting a specific resolution (e.g., `1080p`, `720p`), `Best Video`, or `Audio` from the browser extension's floating download menu now carries the chosen `quality` and `format_id` straight into the desktop **Download File Info** dialog and `yt-dlp` / `ffmpeg` muxing pipeline (`combine_format_selector` & `build_ytdlp_format_args`).
- **Smart Audio-Only Routing**: Audio-only requests automatically resolve to `.mp3` filenames and route to the **Music** category folder while preserving per-download `Cookie`, `Referer`, and `User-Agent` headers.
- **Queue Recovery Support**: Persisted `audio:<fid>` and `<fid>|<H>p` format selectors are recognized cleanly when resuming interrupted streaming downloads.

### 🎨 Browser Extension UI & Stability Polish
- **Inline SVG Logo & Dropdown Layout**: Replaced image logo tags in the in-page video pill and menu header with crisp inline SVG rendering (`createCosmoLogoSvg`), widened the quality picker (`220px`–`340px`) for single-line non-wrapping rows, deduplicated size / `Auto` labels, and eliminated the lingering `Sent` pill state.
- **Clean Extension Reload Handling**: Added context-invalidation detection (`stopScannerIfInvalidated`) so content scripts gracefully stop polling timers and disconnect `MutationObserver` after extension updates.

---

## Release Assets

| File | Description |
| :--- | :--- |
| `CosmoDownloader-Setup-v1.2.8.exe` | **Windows Installer (Recommended)** — Installs Cosmo Downloader (`v1.2.8`), desktop/start shortcuts, `yt-dlp` / `ffmpeg`, and registers browser native messaging hosts. Seamlessly upgrades existing installations. |
| `CosmoDownloader.exe` | Standalone Windows desktop application binary (`v1.2.8`). |
| `CosmoDownloader-v1.2.8-windows-x64.zip` | Standalone Portable Windows package (`v1.2.8`) containing all executables, helper tools, and browser extensions. |
| `CosmoDownloader-Chrome-Edge-Brave-Opera-Extension-v1.2.8.zip` | **All-in-One** unified browser extension package for Google Chrome, Microsoft Edge, Brave, and Opera (`v1.2.8`). |

---

## Quick Install (Windows)

1. Download and run **`CosmoDownloader-Setup-v1.2.8.exe`** (or use the standalone `CosmoDownloader.exe`).
2. **Chrome / Microsoft Edge / Brave / Opera:**
   - Extract **`CosmoDownloader-Chrome-Edge-Brave-Opera-Extension-v1.2.8.zip`**.
   - Open your browser's extensions management page (`chrome://extensions`, `edge://extensions`, `brave://extensions`, `opera://extensions`).
   - Enable **Developer mode** (top-right toggle).
   - Click **Load unpacked** and select the extracted extension folder.
3. Videos played on YouTube, Facebook, or any streaming website will now show the Cosmo download button and download smoothly with full multi-segment acceleration.
