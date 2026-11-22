---
name: copy-build-to-desktop
description: >-
  Automatically locates the newest compiled Cosmo Downloader application binary (CosmoDownloader,
  CosmoDownloader.exe, or Linux AppImage) from the build outputs in WSL or Windows and copies it
  directly to the Windows Desktop for rapid testing and development cycles. Use when the user asks to
  copy the latest build to the desktop, deploy the app to desktop, or check/export the newest build.
---

# Copy Build to Desktop Skill

This skill automatically finds the most recently compiled application binary (Linux ELF, Windows `.exe`, or `.AppImage`) and deploys it straight to the user's Windows Desktop (`C:\Users\steam\OneDrive\Desktop`).

## Workflow

### 1. Build the Application (if needed)
If the project hasn't been compiled recently or the user made fresh code changes:

**In WSL (Linux build / AppImage):**
```bash
# Debug build
cargo build -p rustget-app

# Or Release build
cargo build --release -p rustget-app

# Or AppImage
./scripts/build-appimage.sh
```

**In Windows (Windows .exe build):**
```powershell
cargo build --release -p rustget-app
```

### 2. Copy the Newest Artifact to Desktop

**From WSL / Linux:**
```bash
./scripts/copy_build.sh
```

**From Windows PowerShell:**
```powershell
.\scripts\copy_build.ps1
```

### 3. Verify
Confirm the file arrived on the Windows Desktop:
- Path: `C:\Users\steam\OneDrive\Desktop\CosmoDownloader.exe` (or `CosmoDownloader` / `*.AppImage`)
- Report the copied binary name, file size, and timestamp to the user.
