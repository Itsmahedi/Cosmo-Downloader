use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Instant;

use eframe::egui::{self, Context};
use rustget_app::{
    app::{determine_launch_origin, DownloadDialogRequest, LaunchOrigin, MainWindowReason},
    dialogs::download_window::{DownloadWindowStage, DownloadWindowState},
    APP_TITLE, DOWNLOAD_FILE_INFO_TITLE, MAIN_VIEWPORT_ID,
};
use rustget_core::youtube::normalize_url_key;

/// Harness that faithfully mirrors the exact windowing and visibility gate logic
/// implemented in `CosmoDownloaderApp`.
pub struct WindowGateHarness {
    pub origin: LaunchOrigin,
    pub is_main_window_open: Arc<AtomicBool>,
    pub setting_open_main_on_download_start: bool,
    pub setting_minimize_to_tray: bool,
    pub download_windows: Vec<DownloadWindowState>,
    pub recent_dialog_keys: HashMap<String, Instant>,
    pub running_downloads: Vec<String>,
    pub last_restored_at: Instant,
    pub logs: Vec<String>,
}

impl WindowGateHarness {
    pub fn new(args: &[&str]) -> Self {
        let string_args: Vec<String> = args.iter().map(|s| s.to_string()).collect();
        let origin = determine_launch_origin(&string_args);
        let start_minimized = origin == LaunchOrigin::Autostart;
        let should_open_main = origin == LaunchOrigin::User && !start_minimized;

        Self {
            origin,
            is_main_window_open: Arc::new(AtomicBool::new(should_open_main)),
            setting_open_main_on_download_start: false,
            setting_minimize_to_tray: true,
            download_windows: Vec::new(),
            recent_dialog_keys: HashMap::new(),
            running_downloads: Vec::new(),
            last_restored_at: Instant::now(),
            logs: Vec::new(),
        }
    }

    /// Single Gate Function (matches `CosmoDownloaderApp::show_main_window` exactly)
    pub fn show_main_window(&mut self, reason: MainWindowReason, ctx: &Context) -> bool {
        self.logs.push(format!("show_main_window: {:?}", reason));

        match reason {
            MainWindowReason::UserLaunchedManually
            | MainWindowReason::TrayLeftClick
            | MainWindowReason::TrayMenuOpen
            | MainWindowReason::SecondInstanceLaunchedByUser
            | MainWindowReason::UserClickedOpenMainWindow => {
                // Allowed unconditional reasons
            }
            MainWindowReason::SettingOpenMainOnDownloadStart => {
                if !self.setting_open_main_on_download_start {
                    self.logs.push("show_main_window REJECTED: setting is false".to_string());
                    return false;
                }
            }
            MainWindowReason::Forbidden(r) => {
                self.logs.push(format!("show_main_window REJECTED: forbidden reason {r}"));
                #[cfg(debug_assertions)]
                {
                    panic!("Forbidden attempt to show main window: reason={r}");
                }
                #[cfg(not(debug_assertions))]
                {
                    return false;
                }
            }
        }

        self.is_main_window_open.store(true, Ordering::SeqCst);
        self.last_restored_at = Instant::now();
        for w in &mut self.download_windows {
            if w.stage != DownloadWindowStage::Info {
                w.is_open = false;
            }
        }

        let main_vp_id = egui::ViewportId::from_hash_of(MAIN_VIEWPORT_ID);
        ctx.send_viewport_cmd_to(main_vp_id, egui::ViewportCommand::Minimized(false));
        ctx.send_viewport_cmd_to(main_vp_id, egui::ViewportCommand::Visible(true));
        ctx.send_viewport_cmd_to(main_vp_id, egui::ViewportCommand::Focus);

        true
    }

    /// Simulates extension / protocol / UI opening a download dialog
    pub fn show_download_dialog(&mut self, req: DownloadDialogRequest) -> bool {
        let url = req.url.trim();
        let key = if !url.is_empty() {
            normalize_url_key(url)
        } else {
            String::new()
        };

        let filename = req.filename.unwrap_or_else(|| "download".to_string());
        let mut wnd = DownloadWindowState::new(
            req.url.clone(),
            filename,
            "General".to_string(),
            "C:\\Downloads".to_string(),
        );
        wnd.is_open = true;

        if !key.is_empty() {
            self.recent_dialog_keys.insert(key, Instant::now());
        }
        self.download_windows.push(wnd);
        true
    }

    /// User starts the download in a dialog
    pub fn start_download_in_dialog(&mut self, index: usize, ctx: &Context) {
        if let Some(wnd) = self.download_windows.get_mut(index) {
            wnd.stage = DownloadWindowStage::Downloading;
            self.running_downloads.push(wnd.url.clone());
            // Check setting gate
            if self.setting_open_main_on_download_start {
                wnd.is_open = false;
                self.show_main_window(MainWindowReason::SettingOpenMainOnDownloadStart, ctx);
            }
        }
    }

    /// User cancels or closes a dialog
    pub fn close_dialog(&mut self, index: usize) {
        if let Some(wnd) = self.download_windows.get_mut(index) {
            wnd.is_open = false;
        }
    }

    /// Query exact list of visible window titles presented to the user
    pub fn visible_windows(&self) -> Vec<String> {
        let mut windows = Vec::new();

        // The root window is permanently a 1x1 transparent WS_EX_TOOLWINDOW host with no taskbar presence.
        // It is NOT a user-visible window.

        // Main dashboard is only visible if is_main_window_open is true
        if self.is_main_window_open.load(Ordering::SeqCst) {
            windows.push(APP_TITLE.to_string());
        }

        // Each open download dialog is a visible top-level window
        for wnd in &self.download_windows {
            if wnd.is_open {
                windows.push(DOWNLOAD_FILE_INFO_TITLE.to_string());
            }
        }

        windows
    }
}

// ============================================================================
// STEP 0 & STEP 4: 9 Scenarios Test Suite
// ============================================================================

/// Scenario 1: Cold start, browser Download button (extension)
/// Expected: Only "Download File Info" window is visible. Main window MUST NOT be in the list.
#[test]
fn test_scenario_1_cold_start_browser_download_button() {
    let ctx = Context::default();
    let mut harness = WindowGateHarness::new(&[
        "CosmoDownloader.exe",
        "--from-extension",
        "--popup-file",
        "C:\\Temp\\dl.json",
    ]);

    assert_eq!(harness.origin, LaunchOrigin::Extension);
    assert!(!harness.is_main_window_open.load(Ordering::SeqCst));

    // Extension shows download dialog
    harness.show_download_dialog(DownloadDialogRequest {
        url: "https://example.com/video.mp4".to_string(),
        ..Default::default()
    });

    let visible = harness.visible_windows();
    assert_eq!(visible, vec!["Download File Info".to_string()]);
    assert!(!visible.contains(&"Cosmo Downloader".to_string()));

    // User clicks Start Download (default setting: open_main = false)
    harness.start_download_in_dialog(0, &ctx);
    let visible_after_start = harness.visible_windows();
    assert_eq!(visible_after_start, vec!["Download File Info".to_string()]);
    assert!(!visible_after_start.contains(&"Cosmo Downloader".to_string()));
}

/// Scenario 1 (variant): When SettingOpenMainOnDownloadStart is explicitly TRUE
#[test]
fn test_scenario_1_setting_open_main_true_opens_main_window() {
    let ctx = Context::default();
    let mut harness = WindowGateHarness::new(&["CosmoDownloader.exe", "--from-extension"]);
    harness.setting_open_main_on_download_start = true;

    harness.show_download_dialog(DownloadDialogRequest {
        url: "https://example.com/file.zip".to_string(),
        ..Default::default()
    });

    assert!(!harness.is_main_window_open.load(Ordering::SeqCst));
    harness.start_download_in_dialog(0, &ctx);

    // Because setting is TRUE, main window opens via SettingOpenMainOnDownloadStart and popup closes
    assert!(harness.is_main_window_open.load(Ordering::SeqCst));
    let visible = harness.visible_windows();
    assert!(visible.contains(&"Cosmo Downloader".to_string()));
    assert!(!visible.contains(&"Download File Info".to_string()));
}

/// Scenario 2: App already running with main window hidden in tray, browser Download button
/// Expected: Only "Download File Info" window appears. Main window stays hidden.
#[test]
fn test_scenario_2_app_running_hidden_in_tray_browser_download() {
    let mut harness = WindowGateHarness::new(&["CosmoDownloader.exe", "--minimized"]);
    assert!(!harness.is_main_window_open.load(Ordering::SeqCst));
    assert!(harness.visible_windows().is_empty());

    // Browser triggers download
    harness.show_download_dialog(DownloadDialogRequest {
        url: "https://www.youtube.com/watch?v=dQw4w9WgXcQ".to_string(),
        ..Default::default()
    });

    let visible = harness.visible_windows();
    assert_eq!(visible, vec!["Download File Info".to_string()]);
    assert!(!visible.contains(&"Cosmo Downloader".to_string()));
}

/// Scenario 3: App running with main window visible, browser Download button
/// Expected: Both "Cosmo Downloader" (was already open) and "Download File Info" are visible.
#[test]
fn test_scenario_3_app_running_with_main_visible_browser_download() {
    // Cold start with User origin opens main window
    let mut harness = WindowGateHarness::new(&["CosmoDownloader.exe"]);
    assert_eq!(harness.origin, LaunchOrigin::User);
    assert!(harness.is_main_window_open.load(Ordering::SeqCst));

    // Browser triggers download
    harness.show_download_dialog(DownloadDialogRequest {
        url: "https://example.com/archive.tar.gz".to_string(),
        ..Default::default()
    });

    let visible = harness.visible_windows();
    assert!(visible.contains(&"Cosmo Downloader".to_string()));
    assert!(visible.contains(&"Download File Info".to_string()));
    assert_eq!(visible.len(), 2);
}

/// Scenario 4: Cold start via protocol link / command-line --add-url
/// Expected: Only "Download File Info" window is visible. Main window MUST NOT be visible.
#[test]
fn test_scenario_4_cold_start_protocol_or_add_url() {
    for arg in [
        "--add-url",
        "cosmodownloader://https://example.com/test.iso",
        "rustget://download?url=https://example.com/test.iso",
        "magnet:?xt=urn:btih:abc123",
        "C:\\Downloads\\archive.torrent",
    ] {
        let harness = WindowGateHarness::new(&["CosmoDownloader.exe", arg]);
        assert_eq!(
            harness.origin,
            LaunchOrigin::Protocol,
            "Failed for arg: {}",
            arg
        );
        assert!(
            !harness.is_main_window_open.load(Ordering::SeqCst),
            "Main window must not open for protocol: {}",
            arg
        );
    }
}

/// Scenario 5: Startup with Windows (--minimized)
/// Expected: Zero visible windows (host is invisible). Main window MUST NOT be in the list.
#[test]
fn test_scenario_5_startup_with_windows_minimized() {
    let harness = WindowGateHarness::new(&["CosmoDownloader.exe", "--minimized"]);
    assert_eq!(harness.origin, LaunchOrigin::Autostart);
    assert!(!harness.is_main_window_open.load(Ordering::SeqCst));
    assert!(harness.visible_windows().is_empty());
}

/// Scenario 6: User double-clicks the app while it is already running
/// Expected: Single-instance handover sends show event -> Main window becomes visible.
#[test]
fn test_scenario_6_user_double_clicks_app_while_running() {
    let ctx = Context::default();
    // App was running in tray (minimized)
    let mut harness = WindowGateHarness::new(&["CosmoDownloader.exe", "--minimized"]);
    assert!(!harness.is_main_window_open.load(Ordering::SeqCst));

    // Second instance launched by user:
    let second_instance_origin = determine_launch_origin(&["CosmoDownloader.exe".to_string()]);
    assert_eq!(second_instance_origin, LaunchOrigin::User);

    // Handover triggers gate with SecondInstanceLaunchedByUser
    let opened = harness.show_main_window(MainWindowReason::SecondInstanceLaunchedByUser, &ctx);
    assert!(opened);
    assert!(harness.is_main_window_open.load(Ordering::SeqCst));
    assert_eq!(harness.visible_windows(), vec!["Cosmo Downloader".to_string()]);
}

/// Scenario 7: Clipboard link detected
/// Expected: Only "Download File Info" window opens. Main window MUST NOT open.
#[test]
fn test_scenario_7_clipboard_link_detected() {
    let harness_args = WindowGateHarness::new(&["CosmoDownloader.exe", "--clipboard"]);
    assert_eq!(harness_args.origin, LaunchOrigin::Clipboard);
    assert!(!harness_args.is_main_window_open.load(Ordering::SeqCst));

    // While running in tray, clipboard event fires
    let mut harness = WindowGateHarness::new(&["CosmoDownloader.exe", "--minimized"]);
    assert!(!harness.is_main_window_open.load(Ordering::SeqCst));

    harness.show_download_dialog(DownloadDialogRequest {
        url: "https://example.com/clipboard_file.zip".to_string(),
        ..Default::default()
    });

    let visible = harness.visible_windows();
    assert_eq!(visible, vec!["Download File Info".to_string()]);
    assert!(!visible.contains(&"Cosmo Downloader".to_string()));
}

/// Scenario 8: Download finished / dialog closed / Cancel pressed
/// Expected: Main window stays closed. Never opens as a fallback!
#[test]
fn test_scenario_8_download_finished_dialog_closed_cancel_pressed() {
    let mut harness = WindowGateHarness::new(&["CosmoDownloader.exe", "--from-extension"]);

    harness.show_download_dialog(DownloadDialogRequest {
        url: "https://example.com/file.zip".to_string(),
        ..Default::default()
    });
    assert_eq!(harness.visible_windows(), vec!["Download File Info".to_string()]);

    // User clicks Cancel
    harness.close_dialog(0);
    assert!(harness.visible_windows().is_empty());
    assert!(
        !harness.is_main_window_open.load(Ordering::SeqCst),
        "Main window must NOT open when dialog cancels or finishes!"
    );
}

/// Scenario 9: Browser request for a different video while first dialog is open
/// Expected: Two "Download File Info" dialogs open. Main window MUST NOT be visible.
#[test]
fn test_scenario_9_multiple_browser_requests_different_videos() {
    let mut harness = WindowGateHarness::new(&["CosmoDownloader.exe", "--from-extension"]);

    harness.show_download_dialog(DownloadDialogRequest {
        url: "https://www.youtube.com/watch?v=video_aaa".to_string(),
        ..Default::default()
    });

    harness.show_download_dialog(DownloadDialogRequest {
        url: "https://www.youtube.com/watch?v=video_bbb".to_string(),
        ..Default::default()
    });

    let visible = harness.visible_windows();
    assert_eq!(
        visible,
        vec![
            "Download File Info".to_string(),
            "Download File Info".to_string()
        ]
    );
    assert!(!visible.contains(&"Cosmo Downloader".to_string()));
}

// ============================================================================
// Gate Rejection & Panic Enforcement Tests
// ============================================================================

/// When SettingOpenMainOnDownloadStart is false, gate returns false and does not show main window
#[test]
fn test_gate_rejects_setting_when_option_is_false() {
    let ctx = Context::default();
    let mut harness = WindowGateHarness::new(&["CosmoDownloader.exe", "--from-extension"]);
    harness.setting_open_main_on_download_start = false;

    let success = harness.show_main_window(MainWindowReason::SettingOpenMainOnDownloadStart, &ctx);
    assert!(!success);
    assert!(!harness.is_main_window_open.load(Ordering::SeqCst));
    assert!(harness.visible_windows().is_empty());
}

/// In debug builds, any call with a forbidden reason panics
#[test]
#[should_panic(expected = "Forbidden attempt to show main window: reason=extension_request")]
fn test_gate_panics_on_forbidden_reason_in_debug() {
    let ctx = Context::default();
    let mut harness = WindowGateHarness::new(&["CosmoDownloader.exe", "--from-extension"]);
    harness.show_main_window(MainWindowReason::Forbidden("extension_request"), &ctx);
}
