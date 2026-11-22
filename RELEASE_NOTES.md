# Cosmo Downloader v1.2.7

A fast, reliable multi-segment download manager for **Windows 10 and Windows 11**, paired with redesigned browser extensions for Google Chrome, Microsoft Edge, Brave, Opera, and Mozilla Firefox.

---

## What's New in v1.2.7

### 🎥 Universal IDM-Style Video Sniffer & Download Engine
- **Network Stream Sniffing**: The browser extensions now actively monitor network media streams via `webRequest` without blocking or slowing page loads. Automatically intercepts and extracts:
  - **HLS (.m3u8)** adaptive video playlists and manifests
  - **DASH (.mpd)** XML dynamic adaptive streams
  - **Direct video streams** (`.mp4`, `.webm`, `.m4v`, `.flv`, `.mov`, and `video/*` media types)
- **Facebook & Social Media Video Support**: Added dedicated support for Facebook Reels and videos, Instagram, Twitter/X, TikTok, and local streaming websites. Real browser session cookies, referrers, and user agents are forwarded to the download engine to bypass private or authenticated access restrictions.
- **Universal Floating Download Button**: Displays the sleek Cosmo Download Pill directly on qualified video players across any website when playback starts.
- **Quality & Format Picker with Adaptive Fallback**: Real-time display of available stream qualities, resolutions, or direct video streams with exact or estimated file sizes.
- **HLS/DASH Stream Muxing**: Direct integration with bundled `yt-dlp` and `ffmpeg` engines to download all segments in parallel and stitch them seamlessly into standard `.mp4` format.

---

## Release Assets

| File | Description |
| :--- | :--- |
| `CosmoDownloader-Setup-v1.2.7.exe` | **Windows Installer (Recommended)** — Installs Cosmo Downloader (`v1.2.7`), desktop/start shortcuts, `yt-dlp` / `ffmpeg`, and registers browser native messaging hosts. Seamlessly upgrades existing installations. |
| `CosmoDownloader.exe` | Standalone Windows desktop application binary (`v1.2.7`). |
| `CosmoDownloader-v1.2.7-windows-x64.zip` | Standalone Portable Windows package (`v1.2.7`) containing all executables, helper tools, and browser extensions. |
| `CosmoDownloader-Chrome-Edge-Brave-Opera-Extension-v1.2.7.zip` | **All-in-One** unified browser extension package for Google Chrome, Microsoft Edge, Brave, and Opera (`v1.2.7`). |

---

## Quick Install (Windows)

1. Download and run **`CosmoDownloader-Setup-v1.2.7.exe`** (or use the standalone `CosmoDownloader.exe`).
2. **Chrome / Microsoft Edge / Brave / Opera:**
   - Extract **`CosmoDownloader-Chrome-Edge-Brave-Opera-Extension-v1.2.7.zip`**.
   - Open your browser's extensions management page (`chrome://extensions`, `edge://extensions`, `brave://extensions`, `opera://extensions`).
   - Enable **Developer mode** (top-right toggle).
   - Click **Load unpacked** and select the extracted extension folder.
3. Videos played on YouTube, Facebook, or any streaming website will now show the Cosmo download button and download smoothly with full multi-segment acceleration.
