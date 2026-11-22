# Cosmo Downloader v1.2.6

A fast, reliable multi-segment download manager for **Windows 10 and Windows 11**, paired with redesigned browser extensions for Google Chrome, Microsoft Edge, Brave, Opera, and Mozilla Firefox.

---

## What's New in v1.2.6

### 🎬 Video Player & Mini-Preview Fixes
- **Strict Video Targeting**: Fixed download button injection on YouTube and streaming sites so it appears strictly on active main players (`#movie_player` and active Shorts). Buttons are completely prevented from showing on video thumbnails, mini players, or recommendation feeds.
- **Subsequent Downloads Debounce**: Solved rapid consecutive download race conditions, allowing instant subsequent downloads without getting blocked.
- **Lenient Size Fallback**: Improved format parsing and size resolution for streaming videos.

### 🔌 Real-Time Extension Probing & 1-Click Launch
- **App Lifecycle Health Probing**: Extension dynamically polls internal loopback `/health` to immediately detect when Cosmo Downloader starts or stops.
- **3-State Connection Status**: Extension popup clearly displays whether Cosmo Downloader is Connected (green), Closed (orange), or Host Missing (red).
- **1-Click Launch**: If the application is closed, clicking the status banner instantly launches Cosmo Downloader via the native messaging host.

### 🎨 Modernized Desktop GUI & Category HUD
- **Interactive Category Filter Pills**: Instant filtering by file type (All, Video, Audio, Documents, Compressed, Programs, Others) with live item count badges.
- **Direct File & Folder Actions**: One-click "Open File" and "Open Folder" action buttons on completed downloads.
- **Segmented Visual Progress Bars**: Real-time multi-threaded segment progress visualization.
- **Live Speed HUD**: Aggregate download speed indicator in toolbar and tray.
- **Vector Icons**: Replaced raster assets with sharp, scalable vector icons rendered directly with `egui::Painter`.

### 🪟 Foreground Activation & Windows Platform Fixes
- **Reliable Topmost Raising**: Ensured the download info dialog raises reliably above browsers (`BringWindowToTop`, `SetForegroundWindow`, `AttachThreadInput`).
- **Unicode Sanitization**: Enhanced disk path resolution and sanitization for Unicode filenames.

---

## Release Assets

| File | Description |
| :--- | :--- |
| `CosmoDownloader-Setup-v1.2.6.exe` | **Windows Installer (Recommended)** — Installs Cosmo Downloader (`v1.2.6`), desktop/start shortcuts, `yt-dlp` / `ffmpeg`, and registers browser native messaging hosts. Seamlessly upgrades existing installations. |
| `CosmoDownloader.exe` | Standalone Windows desktop application binary (`v1.2.6`). |
| `CosmoDownloader-Chrome-Edge-Brave-Opera-Extension-v1.2.6.zip` | **All-in-One** unified browser extension package for Google Chrome, Microsoft Edge, Brave, and Opera (`v1.2.6`). |

---

## Quick Install (Windows)

1. Download and run **`CosmoDownloader-Setup-v1.2.6.exe`**.
2. **Chrome / Microsoft Edge / Brave / Opera:**
   - Extract **`CosmoDownloader-Chrome-Edge-Brave-Opera-Extension-v1.2.6.zip`**.
   - Open your browser's extensions management page (`chrome://extensions`, `edge://extensions`, `brave://extensions`, `opera://extensions`).
   - Enable **Developer mode** (top-right toggle).
   - Click **Load unpacked** and select the extracted extension folder.
3. Downloads clicked inside your browser or from video players will automatically open the redesigned Cosmo Downloader popup dialog.
