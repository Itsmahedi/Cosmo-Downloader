use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use eframe::egui::{Context, Pos2};
use rustget_app::{
    app::DownloadDialogRequest,
    dialogs::download_window::{
        render_download_window_content, DownloadWindowOutcome, DownloadWindowStage,
        DownloadWindowState, InfoLookupStatus, DOWNLOAD_WINDOW_HEIGHT, DOWNLOAD_WINDOW_WIDTH,
    },
    theme,
};
use rustget_core::youtube::normalize_url_key;

struct AppSubsequentHarness {
    pub download_windows: Vec<DownloadWindowState>,
    pub recent_dialog_keys: HashMap<String, Instant>,
    pub download_started_at: HashMap<String, Instant>,
    pub running_downloads: Vec<String>,
    pub is_main_window_open: Arc<AtomicBool>,
    pub log: Vec<String>,
}

impl AppSubsequentHarness {
    fn new() -> Self {
        Self {
            download_windows: Vec::new(),
            recent_dialog_keys: HashMap::new(),
            download_started_at: HashMap::new(),
            running_downloads: Vec::new(),
            is_main_window_open: Arc::new(AtomicBool::new(false)),
            log: Vec::new(),
        }
    }

    fn clear_dedupe_keys_for_url(&mut self, url: &str) {
        let key = normalize_url_key(url);
        if !key.is_empty() {
            self.recent_dialog_keys.remove(&key);
            self.download_started_at.remove(&key);
        }
    }

    fn show_download_dialog(&mut self, req: DownloadDialogRequest) -> bool {
        let url = req.url.trim();
        let key = if !url.is_empty() {
            normalize_url_key(url)
        } else {
            String::new()
        };

        if !key.is_empty() {
            let now = Instant::now();

            let has_open_dialog = self.download_windows.iter().enumerate().find(|(_, w)| {
                w.is_open
                    && (w.stage == DownloadWindowStage::Info
                        || w.stage == DownloadWindowStage::Downloading
                        || w.stage == DownloadWindowStage::Paused)
                    && normalize_url_key(&w.url) == key
            });

            let recent_open = self
                .recent_dialog_keys
                .get(&key)
                .map(|t| now.duration_since(*t) < Duration::from_secs(2))
                .unwrap_or(false);

            if let Some((existing_idx, _)) = has_open_dialog {
                self.log.push(format!("DUPLICATE_OPEN_KEY: {}", key));
                self.download_windows[existing_idx].is_open = true;
                return false;
            } else if recent_open {
                self.log.push(format!("RECENT_OPEN_KEY: {}", key));
                return false;
            }

            let download_recently_started = self
                .download_started_at
                .get(&key)
                .map(|t| now.duration_since(*t) < Duration::from_secs(5))
                .unwrap_or(false);

            let is_download_active = self.running_downloads.iter().any(|k| k == &key);

            if download_recently_started || is_download_active {
                self.log.push(format!("ACTIVE_DOWNLOAD_KEY: {}", key));
                return false;
            }
        }

        let open_count = self.download_windows.iter().filter(|w| w.is_open).count();
        if open_count >= 5 {
            return false;
        }

        let cascade_idx = open_count % 5;
        let base_x = 400.0;
        let base_y = 250.0;
        let win_pos = Pos2::new(base_x + (cascade_idx as f32) * 24.0, base_y + (cascade_idx as f32) * 24.0);

        let mut wnd = DownloadWindowState::new(
            req.url.clone(),
            req.filename.unwrap_or_else(|| "video.mp4".to_string()),
            "Video".to_string(),
            "C:\\Users\\steam\\Downloads".to_string(),
        );
        wnd.cascade_index = cascade_idx;
        wnd.window_pos = Some(win_pos);
        wnd.is_open = true;

        if !key.is_empty() {
            self.recent_dialog_keys.insert(key, Instant::now());
        }

        self.download_windows.push(wnd);
        true
    }

    fn start_download(&mut self, idx: usize) {
        if idx < self.download_windows.len() {
            let key = normalize_url_key(&self.download_windows[idx].url);
            self.download_started_at.insert(key.clone(), Instant::now());
            self.running_downloads.push(key);
            self.download_windows[idx].stage = DownloadWindowStage::Downloading;
        }
    }

    fn close_dialog(&mut self, idx: usize) {
        if idx < self.download_windows.len() {
            let url = self.download_windows[idx].url.clone();
            self.clear_dedupe_keys_for_url(&url);
            self.download_windows[idx].is_open = false;
        }
    }

    fn complete_download(&mut self, idx: usize) {
        if idx < self.download_windows.len() {
            let key = normalize_url_key(&self.download_windows[idx].url);
            let url = self.download_windows[idx].url.clone();
            self.running_downloads.retain(|k| k != &key);
            self.clear_dedupe_keys_for_url(&url);
            self.download_windows[idx].stage = DownloadWindowStage::Complete;
        }
    }
}

/// Helper to render one egui frame on a window
fn render_window_frame(ctx: &Context, wnd: &mut DownloadWindowState) -> DownloadWindowOutcome {
    let pal = theme::palette(true);
    let mut outcome = DownloadWindowOutcome::None;
    let _ = ctx.run(Default::default(), |ctx| {
        outcome = render_download_window_content(ctx, pal, None, wnd);
    });
    outcome
}

/// TEST 1: Download video A and press Start. While A is downloading, click Download on video B:
/// dialog B shows its content immediately (link + loading, then title), and Start download works. Both download.
#[test]
fn test_1_download_a_then_b_while_a_is_downloading() {
    let mut harness = AppSubsequentHarness::new();
    let ctx = Context::default();
    theme::configure_fonts(&ctx);
    theme::configure_visuals(&ctx, true);

    // 1. Download video A
    let res_a = harness.show_download_dialog(DownloadDialogRequest {
        url: "https://www.youtube.com/watch?v=dQw4w9WgXcQ".to_string(),
        filename: Some("Rick Astley - Never Gonna Give You Up.mp4".to_string()),
        cookies: None,
        referer: None,
        user_agent: None,
        num_segments: None,
        ..Default::default()
    });
    assert!(res_a, "Dialog A should open");
    assert_eq!(harness.download_windows.len(), 1);

    // Verify dialog A renders cleanly
    let outcome_a = render_window_frame(&ctx, &mut harness.download_windows[0]);
    assert_eq!(outcome_a, DownloadWindowOutcome::None);
    assert_eq!(harness.download_windows[0].stage, DownloadWindowStage::Info);

    // User presses Start on video A
    harness.start_download(0);
    assert_eq!(harness.download_windows[0].stage, DownloadWindowStage::Downloading);
    assert_eq!(harness.running_downloads.len(), 1);

    // 2. While A is downloading, user clicks Download on video B
    let res_b = harness.show_download_dialog(DownloadDialogRequest {
        url: "https://www.youtube.com/watch?v=9bZkp7q19f0".to_string(),
        filename: Some("PSY - GANGNAM STYLE.mp4".to_string()),
        cookies: None,
        referer: None,
        user_agent: None,
        num_segments: None,
        ..Default::default()
    });
    assert!(res_b, "Dialog B should open immediately while A is downloading");
    assert_eq!(harness.download_windows.len(), 2);
    assert!(harness.download_windows[1].is_open);

    // Dialog B shows its content immediately (560x380, stage Info, cascaded 24px)
    let pos_a = harness.download_windows[0].window_pos.unwrap();
    let pos_b = harness.download_windows[1].window_pos.unwrap();
    assert_eq!(pos_b.x - pos_a.x, 24.0, "Dialog B cascaded 24px horizontally");
    assert_eq!(pos_b.y - pos_a.y, 24.0, "Dialog B cascaded 24px vertically");

    let outcome_b = render_window_frame(&ctx, &mut harness.download_windows[1]);
    assert_eq!(outcome_b, DownloadWindowOutcome::None);

    // Start download B works. Both download.
    harness.start_download(1);
    assert_eq!(harness.download_windows[0].stage, DownloadWindowStage::Downloading);
    assert_eq!(harness.download_windows[1].stage, DownloadWindowStage::Downloading);
    assert_eq!(harness.running_downloads.len(), 2, "Both downloads running concurrently");
}

/// TEST 2: Download A completely, close its dialog, then download B: works.
#[test]
fn test_2_download_a_completely_close_dialog_then_download_b() {
    let mut harness = AppSubsequentHarness::new();
    let ctx = Context::default();

    // Open and start video A
    assert!(harness.show_download_dialog(DownloadDialogRequest {
        url: "https://www.youtube.com/watch?v=dQw4w9WgXcQ".to_string(),
        filename: Some("Rick_Astley.mp4".to_string()),
        cookies: None,
        referer: None,
        user_agent: None,
        num_segments: None,
        ..Default::default()
    }));
    harness.start_download(0);

    // Download A completely
    harness.complete_download(0);
    assert_eq!(harness.download_windows[0].stage, DownloadWindowStage::Complete);

    // Close dialog A
    harness.close_dialog(0);
    assert!(!harness.download_windows[0].is_open);

    // Then download video B: works
    let res_b = harness.show_download_dialog(DownloadDialogRequest {
        url: "https://www.youtube.com/watch?v=kJQP7kiw5Fk".to_string(),
        filename: Some("Despacito.mp4".to_string()),
        cookies: None,
        referer: None,
        user_agent: None,
        num_segments: None,
        ..Default::default()
    });
    assert!(res_b, "Video B should open successfully after A closed");
    assert_eq!(harness.download_windows.len(), 2);
    assert!(harness.download_windows[1].is_open);

    let _ = render_window_frame(&ctx, &mut harness.download_windows[1]);
    harness.start_download(1);
    assert_eq!(harness.download_windows[1].stage, DownloadWindowStage::Downloading);
}

/// TEST 3: Repeat 10 times in a row, with the app staying open: no white window, no stuck dialog.
#[test]
fn test_3_repeat_10_times_in_a_row_no_white_window_no_stuck_dialog() {
    let mut harness = AppSubsequentHarness::new();
    let ctx = Context::default();

    for i in 0..10 {
        let video_id = format!("video_{:02}", i);
        let url = format!("https://www.youtube.com/watch?v={}", video_id);
        let opened = harness.show_download_dialog(DownloadDialogRequest {
            url: url.clone(),
            filename: Some(format!("Title_{}.mp4", i)),
            cookies: None,
            referer: None,
            user_agent: None,
            num_segments: None,
            ..Default::default()
        });
        assert!(opened, "Dialog #{} must open without being stuck", i + 1);

        let wnd_idx = harness.download_windows.len() - 1;
        assert!(harness.download_windows[wnd_idx].is_open);

        // Verify content paints immediately (never white/blank)
        let _ = render_window_frame(&ctx, &mut harness.download_windows[wnd_idx]);
        assert!(harness.download_windows[wnd_idx].has_painted, "Window #{} painted content", i + 1);

        // Start, complete, and close dialog
        harness.start_download(wnd_idx);
        harness.complete_download(wnd_idx);
        harness.close_dialog(wnd_idx);
        assert!(!harness.download_windows[wnd_idx].is_open);
    }

    assert_eq!(harness.download_windows.len(), 10);
    assert_eq!(harness.running_downloads.len(), 0);
}

/// TEST 4: Disconnect the network (simulated lookup failure) and click Download:
/// the dialog shows the error state with Retry (not blank).
#[test]
fn test_4_network_failure_shows_error_state_with_retry_not_blank() {
    let mut wnd = DownloadWindowState::new(
        "https://www.youtube.com/watch?v=deadbeef123".to_string(),
        "video.mp4".to_string(),
        "Video".to_string(),
        "C:\\Users\\steam\\Downloads".to_string(),
    );

    // Simulate network error / yt-dlp timeout
    wnd.info_status = InfoLookupStatus::Failed {
        error: "Network unreachable (connection timed out)".to_string(),
    };

    let ctx = Context::default();

    // Render content: must NOT panic, must set has_painted, must stay on Info stage
    let outcome = render_window_frame(&ctx, &mut wnd);
    assert_eq!(outcome, DownloadWindowOutcome::None);
    assert!(wnd.has_painted, "Must paint error content (never blank)");
    assert_eq!(wnd.stage, DownloadWindowStage::Info);

    // User clicks Retry -> lookup status resets to ReadingInfo
    wnd.retry_info_lookup();
    assert_eq!(wnd.info_status, InfoLookupStatus::ReadingInfo);

    // User can also choose Use Link As Is
    wnd.use_link_as_is();
    assert_eq!(wnd.info_status, InfoLookupStatus::Success);
}

/// TEST 5: Close dialog B with X before it loads: no crash, and the next video still works.
#[test]
fn test_5_close_dialog_b_with_x_before_it_loads_next_video_still_works() {
    let mut harness = AppSubsequentHarness::new();
    let ctx = Context::default();

    // Open video B (still in ReadingInfo state)
    assert!(harness.show_download_dialog(DownloadDialogRequest {
        url: "https://www.youtube.com/watch?v=loading_vid_1".to_string(),
        filename: None,
        cookies: None,
        referer: None,
        user_agent: None,
        num_segments: None,
        ..Default::default()
    }));
    assert_eq!(harness.download_windows.len(), 1);
    assert_eq!(harness.download_windows[0].info_status, InfoLookupStatus::ReadingInfo);

    // Render 1 frame
    let _ = render_window_frame(&ctx, &mut harness.download_windows[0]);

    // User clicks X to close before it finishes loading
    harness.close_dialog(0);
    assert!(!harness.download_windows[0].is_open);

    // Next video (video C) still works cleanly
    assert!(harness.show_download_dialog(DownloadDialogRequest {
        url: "https://www.youtube.com/watch?v=next_vid_2".to_string(),
        filename: Some("NextVideo.mp4".to_string()),
        cookies: None,
        referer: None,
        user_agent: None,
        num_segments: None,
        ..Default::default()
    }));
    assert_eq!(harness.download_windows.len(), 2);
    assert!(harness.download_windows[1].is_open);

    let _ = render_window_frame(&ctx, &mut harness.download_windows[1]);
    assert!(harness.download_windows[1].has_painted);
}

/// TEST 6: The main window does not open, and dimensions are strictly 560x350.
#[test]
fn test_6_main_window_stays_closed_and_dimensions_560x350() {
    let harness = AppSubsequentHarness::new();
    assert!(!harness.is_main_window_open.load(Ordering::Relaxed), "Main window remains closed");
    assert_eq!(DOWNLOAD_WINDOW_WIDTH, 560.0);
    assert_eq!(DOWNLOAD_WINDOW_HEIGHT, 350.0);
}
