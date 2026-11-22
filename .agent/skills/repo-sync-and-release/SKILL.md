---
name: repo-sync-and-release
description: >-
  Keep the Itsmahedi/Cosmo-Downloader GitHub repository clean, synced, and up to date after every
  change, and ensure only the latest verified release artifacts are uploaded to GitHub Releases.
  Use whenever finishing feature work, cleaning up files, bumping versions, updating docs, pushing
  commits, or publishing/updating GitHub Releases.
---

# Repo Sync & GitHub Release Workflow

Always keep the codebase, documentation, version numbers, and GitHub Releases (`Itsmahedi/Cosmo-Downloader`) clean and synchronized with the latest verified files.

## 1. Keep Codebase, Versions & Docs Up to Date

After every feature, bugfix, cleanup, or version bump:

1. **Synchronize versions across all targets**:
   - Workspace version (`vX.Y.Z`):
     - `Cargo.toml` (`[workspace.package] version`)
     - `crates/app/app.rc`, `crates/cli/app.rc`, `crates/native-host/app.rc` (`FILEVERSION`, `PRODUCTVERSION`)
     - `crates/app/build.rs`, `crates/cli/build.rs`, `crates/native-host/build.rs` (fallback version)
     - `installer/cosmodownloader.iss` (`MyAppVersion`, `MyAppShortVer`, `VersionInfoVersion`)
     - `installer/build-release.ps1` (portable folder & ZIP names)
     - `package.json` (`version`)
     - `browser-extensions/Chrome-Edge-Brave-Opera/manifest.json` (`version`, `version_name`, `action.default_title`)
   - Firefox extension version (`v1.10.X`):
     - `browser-extensions/Mozilla-Firefox/manifest.json` (`version`, `version_name`, `action.default_title`)
     - Sync shared extension files with `npm run sync-extensions`.
2. **Keep documentation & license current**:
   - Update `README.md`, `CHANGELOG.md`, `RELEASE_NOTES.md`, `docs/TEST_CHECKLIST.md`, `LICENSE`, and `.agent/MINDMAP.md` whenever behavior, paths, scripts, or versions change.
3. **Keep only the latest artifacts on disk**:
   - Prune old version builds, duplicate zips, and scratch files from `dist/` and `installer/dist/` so only the latest verified `vX.Y.Z` (and Firefox `v1.10.X`) files remain.

## 2. Verify, Commit in Logical Chunks & Push

Before committing:

```bash
cargo test --workspace
npm run lint:css
node -e 'JSON.parse(require("fs").readFileSync("browser-extensions/Chrome-Edge-Brave-Opera/manifest.json","utf8")); JSON.parse(require("fs").readFileSync("browser-extensions/Mozilla-Firefox/manifest.json","utf8")); console.log("Manifests OK")'
```

Commit related changes in clean, focused conventional-commit chunks (`feat(...)`, `fix(...)`, `refactor(...)`, `chore(...)`, `docs(...)`), verify a clean tree (`git status --short`), and push immediately so `origin/main` is always up to date:

```bash
git push origin main
```

## 3. Upload Only Latest Verified Files to GitHub Releases

Always pass `--repo Itsmahedi/Cosmo-Downloader` explicitly on every `gh` command.

### Release Asset Rules
- Upload **only** the latest `vX.Y.Z` Windows binaries/installer and the **All-in-One** Chromium extension archive from `dist/`:
  - `dist/CosmoDownloader-Setup-vX.Y.Z.exe`
  - `dist/CosmoDownloader.exe`
  - `dist/CosmoDownloader-vX.Y.Z-windows-x64.zip`
  - `dist/CosmoDownloader-Chrome-Edge-Brave-Opera-Extension-vX.Y.Z.zip`
- **NEVER** create or upload separate per-browser Chromium zips (`Brave-only`, `Edge-only`, etc.).
- **NEVER** upload Firefox extension `.zip` or `.xpi` files to GitHub Releases (Firefox is distributed via AMO; save only the latest verified Firefox ZIP in `C:\Users\steam\OneDrive\Documents\cosmo_downloader\browser-extensions` and delete older Firefox ZIPs).

### Create or Update a Release

**New release tag:**
```bash
git tag -a vX.Y.Z -m "Release vX.Y.Z"
git push origin vX.Y.Z

gh release create vX.Y.Z \
  --repo Itsmahedi/Cosmo-Downloader \
  --title "Cosmo Downloader vX.Y.Z" \
  --notes-file RELEASE_NOTES.md \
  dist/CosmoDownloader-Setup-vX.Y.Z.exe \
  dist/CosmoDownloader.exe \
  dist/CosmoDownloader-vX.Y.Z-windows-x64.zip \
  dist/CosmoDownloader-Chrome-Edge-Brave-Opera-Extension-vX.Y.Z.zip
```

**Replacing assets on an existing tag (keep only latest files):**
```bash
# Overwrite updated release assets and refresh notes
gh release edit vX.Y.Z --repo Itsmahedi/Cosmo-Downloader --notes-file RELEASE_NOTES.md
gh release upload vX.Y.Z --clobber --repo Itsmahedi/Cosmo-Downloader \
  dist/CosmoDownloader-Setup-vX.Y.Z.exe \
  dist/CosmoDownloader.exe \
  dist/CosmoDownloader-vX.Y.Z-windows-x64.zip \
  dist/CosmoDownloader-Chrome-Edge-Brave-Opera-Extension-vX.Y.Z.zip

# Delete any stale/obsolete release assets if present
gh release view vX.Y.Z --repo Itsmahedi/Cosmo-Downloader --json assets -q '.assets[].name'
# gh release delete-asset vX.Y.Z <old-asset-name> --yes --repo Itsmahedi/Cosmo-Downloader
```
