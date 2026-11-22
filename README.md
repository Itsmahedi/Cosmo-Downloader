# Cosmo Downloader v1.2.7

A fast, reliable multi-segment desktop download manager for **Windows 10 and Windows 11**, paired with browser extensions for Google Chrome, Microsoft Edge, Brave, Opera, and Mozilla Firefox.

---

## Key Features

- **Steel Slate Light & Dark UI**: Clean desktop interface (`CosmoDownloader.exe`) with interactive category filter pills and live item badges (`All`, `Video`, `Music`, `Documents`, `Compressed`, `Programs`, `Others`), live search, sortable columns, segmented transfer progress bars, live speed HUD, direct **Open File** / **Open Folder** actions, and adaptive light/dark mode.
- **Multi-Segment & Single-Stream Resume Engine**: Parallel byte-range downloads (up to 32 connections), automatic metadata validation (`Content-Length` / `ETag` / `Last-Modified`), SHA-256 verification, global and per-task rate limits, HTTP/SOCKS proxy support, BitTorrent / `magnet:` support, collision-safe auto-renaming, and crash/restart recovery.
- **Universal IDM-Style Video Sniffer & Muxer**:
  - Passive `webRequest` network sniffing in browser extensions for **HLS (`.m3u8`)**, **DASH (`.mpd`)**, and **direct video streams** (`.mp4`, `.webm`, `.m4v`, `.flv`, `.mov`) without blocking page loads.
  - Full support for YouTube, Facebook & Reels, Instagram, TikTok, Twitter/X, and general streaming sites via bundled `yt-dlp` and `ffmpeg` engines, including real session cookie, `Referer`, and `User-Agent` forwarding and YouTube `webmanifest` fallback handling.
  - Floating in-page download pill with adaptive quality/format picker and instant fallback options.
- **Desktop Ergonomics & Standalone Portability**:
  - Statically linked runtime dependencies (`+crt-static`) for zero-dependency execution on clean Windows 10/11 machines.
  - Shortcuts: `Ctrl+N` (Add URL), `Ctrl+A` (Select All), `Delete` (Delete confirmation with optional disk file removal), `Space` (Pause/Resume selected), `Esc` (Close dialog).
  - Invisible host + on-demand dashboard architecture with system tray icon, single-instance mutex forwarding, and CLI (`cosmodownloader-cli.exe`).
  - Persistent SQLite history & settings (`%LOCALAPPDATA%\CosmoDownloader\CosmoDownloader\data\cosmodownloader.db`) and logs (`%LOCALAPPDATA%\CosmoDownloader\CosmoDownloader\logs\cosmodownloader.log`).
- **Browser Extension Integration**:
  - Unified Chromium Manifest V3 extension (`v1.2.7`) and Mozilla Firefox Manifest V3 extension (`v1.10.4` via AMO).
  - Real-time loopback `/health` probing with a 3-state connection badge (`Connected`, `App Closed`, `Host Missing`), 1-click desktop app launch via Native Messaging (`cosmodownloader-host.exe`), configurable extension/size/domain filters, and safe browser download fallback if the desktop app is unavailable.

---

## Windows Installation & In-Place Upgrades

1. Download and run **`CosmoDownloader-Setup-v1.2.7.exe`** (built from [`installer/cosmodownloader.iss`](installer/cosmodownloader.iss) via Inno Setup 6), or run the standalone **`CosmoDownloader.exe`** / portable **`CosmoDownloader-v1.2.7-windows-x64.zip`** in [`dist/`](dist/).
2. Upgrading over a running installation:
   - Setup detects running Cosmo Downloader processes, sends `--quit` for a clean queue flush (with a 5s wait and `taskkill` fallback), backs up the SQLite database, installs cleanly, and automatically relaunches Cosmo Downloader if it was running prior to the update.
   - User database, queue state, and settings in `%LOCALAPPDATA%\CosmoDownloader` are preserved automatically.
3. Silent install / update:
   ```cmd
   CosmoDownloader-Setup-v1.2.7.exe /VERYSILENT /NORESTART
   ```

---

## Browser Extension Setup

Extension source folders live in [`browser-extensions/`](browser-extensions/):

- **Chrome / Brave / Microsoft Edge / Opera (`v1.2.7`)**:
  1. Open `chrome://extensions`, `brave://extensions`, `edge://extensions`, or `opera://extensions`.
  2. Enable **Developer mode**, click **Load unpacked**, and select [`browser-extensions/Chrome-Edge-Brave-Opera`](browser-extensions/Chrome-Edge-Brave-Opera) (or extract `CosmoDownloader-Chrome-Edge-Brave-Opera-Extension-v1.2.7.zip`).
  3. The manifest public key pins extension ID `lbifcbhmdjlhmcfipfdodbdljmcbdanf` across unpacked installs so the installed native messaging host matches automatically.
- **Mozilla Firefox (`v1.10.4`, Manifest V3)**:
  1. Install from Mozilla Add-ons (AMO), or for local testing open `about:debugging#/runtime/this-firefox`.
  2. Click **Load Temporary Add-on...** and select [`browser-extensions/Mozilla-Firefox/manifest.json`](browser-extensions/Mozilla-Firefox/manifest.json).

---

## Project Structure

```text
cosmo_downloader/
├── crates/
│   ├── core/               # rustget-core: download engine, SQLite DB, HTTP bridge, yt-dlp/torrent support
│   ├── app/                # rustget-app: CosmoDownloader.exe desktop GUI (egui/eframe + tray)
│   ├── cli/                # cosmodownloader-cli.exe CLI utility
│   └── native-host/        # cosmodownloader-host.exe browser native messaging bridge
├── browser-extensions/
│   ├── Chrome-Edge-Brave-Opera/   # Unified Chromium Manifest V3 extension (v1.2.7)
│   └── Mozilla-Firefox/           # Firefox Manifest V3 extension (v1.10.4)
├── installer/              # Inno Setup script (cosmodownloader.iss) & release packager (build-release.ps1)
├── scripts/                # Deployment, signing, extension sync, XPI packaging & stress scripts
├── tests/                  # Playwright & Node extension test suite + Windows smoke test
├── docs/                   # QA checklist & window architecture contract
├── dist/                   # Latest compiled v1.2.7 release artifacts
├── CHANGELOG.md
├── RELEASE_NOTES.md
└── LICENSE
```

---

## Building & Testing from Source

```powershell
# Run workspace unit and integration tests
cargo test --workspace

# Run Clippy strict lint checks
cargo clippy --workspace --all-targets -- -D warnings

# Run extension CSS lint and Playwright automated tests
npm run lint:css
npm test

# Sync shared extension files from Chromium to Firefox
npm run sync-extensions

# Build release binaries (CosmoDownloader.exe, cosmodownloader-cli.exe, cosmodownloader-host.exe)
cargo build --release --workspace
```

See [docs/TEST_CHECKLIST.md](docs/TEST_CHECKLIST.md), [docs/window-rules.md](docs/window-rules.md), [CHANGELOG.md](CHANGELOG.md), and [RELEASE_NOTES.md](RELEASE_NOTES.md) for QA procedures, window visibility rules, and version history.

---

## License

Cosmo Downloader is open-source software licensed under the [MIT License](LICENSE).
