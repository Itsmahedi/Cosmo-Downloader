# Cosmo Downloader Window Architecture & Visibility Rules

This document establishes the permanent architectural contract for window creation, visibility, and focus within Cosmo Downloader.

---

## 1. Core Architecture Design: Invisible Host + On-Demand Dashboard

To completely prevent the main window from ever popping up without explicit user intent, Cosmo Downloader decouples the background service host from the GUI viewports (Architecture Option B).

### The Invisible Root Host
- **Viewport ID**: `ViewportId::ROOT` (`"Cosmo Downloader Host"`)
- **Properties**:
  - Size: 1x1 logical pixels positioned off-screen.
  - Transparent, undecorated (`with_transparent(true)`, `with_decorations(false)`).
  - Win32 ExStyle: `WS_EX_TOOLWINDOW` with alpha = 0.
  - Taskbar: No taskbar icon or button.
- **Responsibilities**:
  - Hosts the Tokio async runtime, SQLite database connections, and queue worker.
  - Manages the local IPC bridge server (`127.0.0.1:42424`).
  - Owns the system tray icon and native context menu.
  - Acts as the parent eframe coordinator for on-demand child viewports.
  - Never renders user UI and is never presented to the user.

### The On-Demand Main Dashboard
- **Viewport ID**: `main_dashboard_viewport_id()` (`MAIN_VIEWPORT_ID = "cosmo_main_dashboard"`)
- **Properties**:
  - Created and rendered via `ctx.show_viewport_immediate(...)` **ONLY IF** `is_main_window_open == true`.
  - When the user closes the main dashboard (via the title bar `[X]` button or menu):
    - `is_main_window_open.store(false, Ordering::SeqCst)`
    - The child viewport is destroyed by eframe / Windows OS.
    - The background host continues running silently (serving background downloads and tray).
  - Because the dashboard viewport literally does not exist when closed, background events, native messaging queries, or downloads cannot accidentally bring it to the front.

### Download Dialog Viewports
- **Viewport ID**: `egui::ViewportId::from_hash_of(format!("download_window_{}", id))`
- **Properties**:
  - Detached, independent 560 x 350 logical px viewports (`"Download File Info"`).
  - Self-contained state machine: `Form -> Connecting -> Downloading <-> Paused -> Processing -> Complete` (or `Failed`/`Canceled`).
  - Can exist completely on their own without the main dashboard ever opening.

---

## 2. The Single Gate Rule: `show_main_window`

**Rule**: No code may directly call `ctx.send_viewport_cmd(Visible(true))`, `Focus`, `Minimized(false)`, or native Win32 `ShowWindow` / `SetForegroundWindow` on the main dashboard window.
All requests to display or focus the main window **MUST** pass through the gate function:

```rust
pub fn show_main_window(&mut self, reason: MainWindowReason, ctx: &egui::Context) -> bool
```

### Allowed Reasons (`MainWindowReason`)

| Reason | Origin / Context | Permitted | Notes |
| :--- | :--- | :--- | :--- |
| `UserLaunchedManually` | Desktop shortcut or start menu launch (`LaunchOrigin::User`) with no `--minimized` | ✅ Allowed | Opens main dashboard on launch. |
| `TrayLeftClick` | User clicks or double-clicks system tray icon | ✅ Allowed | Restores main dashboard. |
| `TrayMenuOpen` | User selects "Open Cosmo Downloader" from tray menu | ✅ Allowed | Restores main dashboard. |
| `SecondInstanceLaunchedByUser` | User double-clicks app exe while it is already running | ✅ Allowed | Mutex handover forwards focus to existing instance. |
| `UserClickedOpenMainWindow` | User clicks button inside a dialog to view main queue | ✅ Allowed | Explicit user request. |
| `SettingOpenMainOnDownloadStart`| Download begins in download dialog | ⚠️ Conditional | **ONLY IF** user setting `setting_open_main_on_download_start == true`. Otherwise strictly rejected. |

### Forbidden Reasons (Strictly Rejected)

The following events **MUST NEVER** open, restore, or focus the main window:
- Browser extension download button clicks (`LaunchOrigin::Extension`).
- Native messaging host requests / IPC queries.
- Protocol links (`cosmodownloader://`, `rustget://`, `magnet:`, `.torrent` associations).
- Clipboard URL detection events.
- Download dialog open, close, cancel, or completion events.
- Windows autostart (`--minimized`, `--daemon`, `--autostart`).
- Restoring saved window state from prior sessions.

> In debug builds, passing any forbidden reason triggers an immediate `panic!`. In release builds, it logs a warning with full stack trace and returns `false`.

---

## 3. Launch Origin Categorization

Launch origin is determined strictly at startup via `determine_launch_origin(&args)`:

1. `LaunchOrigin::Extension`: Arguments contain `--from-extension`, `--popup-file`, or `--popup`.
   - **Mode**: Dialog-only mode. Main window is closed.
2. `LaunchOrigin::Autostart`: Arguments contain `--minimized`, `--daemon`, or `--autostart`.
   - **Mode**: Background tray mode. All windows hidden.
3. `LaunchOrigin::Protocol`: Arguments contain `--add-url`, `cosmodownloader://`, `rustget://`, `magnet:`, or `.torrent`.
   - **Mode**: Dialog-only mode. Main window is closed.
4. `LaunchOrigin::Clipboard`: Arguments contain `--clipboard`.
   - **Mode**: Dialog-only mode. Main window is closed.
5. `LaunchOrigin::User`: Manual launch without special flags.
   - **Mode**: Main dashboard opens.

---

## 4. Pull Request Verification Checklist

Every pull request that modifies window lifecycle, viewport commands, tray interaction, or IPC must verify:

- [ ] **No Raw Viewport Commands**: No new calls to `ViewportCommand::Visible(true)`, `Focus`, or `Minimized(false)` outside `show_main_window`.
- [ ] **Gate Compliance**: All main window visibility transitions route through `show_main_window(MainWindowReason, ctx)`.
- [ ] **Automated Tests Pass**: `cargo test --test window_gate_test` passes all 12 test cases covering the 9 scenarios.
- [ ] **Smoke Test Verified**: `powershell.exe -ExecutionPolicy Bypass -File tests/windows_smoke.ps1` runs without errors on Windows builds.
- [ ] **Pre-commit Hook Active**: `.git/hooks/pre-commit` verifies window gate tests before committing.
