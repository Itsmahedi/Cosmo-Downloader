use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use eframe::egui::Pos2;
use rustget_app::{
    app::DownloadDialogRequest,
    dialogs::download_window::{DownloadWindowStage, DownloadWindowState},
};
use rustget_core::youtube::normalize_url_key;

struct TestAppMock {
    pub download_windows: Vec<DownloadWindowState>,
    pub recent_dialog_keys: HashMap<String, Instant>,
    pub download_started_at: HashMap<String, Instant>,
    pub pending_dialog_queue: VecDeque<DownloadDialogRequest>,
    pub is_main_window_open: Arc<AtomicBool>,
    pub is_popup_mode: bool,
    pub running_downloads: Vec<String>, // active normalized keys
    pub request_log: Vec<String>,
}

impl TestAppMock {
    fn new(main_open: bool, popup_mode: bool) -> Self {
        Self {
            download_windows: Vec::new(),
            recent_dialog_keys: HashMap::new(),
            download_started_at: HashMap::new(),
            pending_dialog_queue: VecDeque::new(),
            is_main_window_open: Arc::new(AtomicBool::new(main_open)),
            is_popup_mode: popup_mode,
            running_downloads: Vec::new(),
            request_log: Vec::new(),
        }
    }

    fn show_download_dialog(
        &mut self,
        req: DownloadDialogRequest,
        source: &str,
    ) {
        let timestamp = chrono::Local::now().format("%Y-%m-%d %H:%M:%S%.3f");
        self.request_log.push(format!(
            "[{}] source='{}' url='{}'",
            timestamp, source, req.url
        ));

        let url = req.url.trim();
        let key = if !url.is_empty() {
            normalize_url_key(url)
        } else {
            String::new()
        };

        if !key.is_empty() {
            let now = Instant::now();

            // STEP 2 Check 1: Already has an open dialog or opened within last 2 seconds
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
                self.request_log.push(format!(
                    "DEDUPE: Suppressing duplicate dialog for key '{}' (already open at index {})",
                    key, existing_idx
                ));
                self.download_windows[existing_idx].is_open = true;
                return;
            } else if recent_open {
                self.request_log.push(format!(
                    "DEDUPE: Suppressing duplicate dialog for key '{}' (opened within last 2s)",
                    key
                ));
                if let Some((existing_idx, _)) = self.download_windows.iter().enumerate().rev().find(|(_, w)| {
                    normalize_url_key(&w.url) == key
                }) {
                    self.download_windows[existing_idx].is_open = true;
                }
                return;
            }

            // STEP 2 Check 2: Download already running or started in last 5 seconds
            let download_recently_started = self
                .download_started_at
                .get(&key)
                .map(|t| now.duration_since(*t) < Duration::from_secs(5))
                .unwrap_or(false);

            let is_download_active = self.running_downloads.iter().any(|d_key| d_key == &key);

            if download_recently_started || is_download_active {
                self.request_log.push(format!(
                    "DEDUPE: Suppressing dialog for key '{}' (download already active or started within 5s)",
                    key
                ));
                return;
            }
        }

        // Up to 5 windows open at once. Extra ones wait in queue.
        let open_count = self.download_windows.iter().filter(|w| w.is_open).count();
        if open_count >= 5 {
            self.request_log.push(format!(
                "QUEUE: 5 dialogs already open, queueing request for '{}'",
                req.url
            ));
            self.pending_dialog_queue.push_back(req);
            return;
        }

        // Consistent position - centered over main window if visible, otherwise screen work area where mouse is
        let (base_x, base_y) = if self.is_main_window_open.load(Ordering::Relaxed) && !self.is_popup_mode {
            // Main window center: (1000/2 - 560/2, 640/2 - 350/2) = (220, 145)
            (220.0, 145.0)
        } else {
            // Mouse work area center: (1920/2 - 560/2, 1080/2 - 350/2) = (680, 365)
            (680.0, 365.0)
        };

        let cascade_idx = open_count % 5;
        let cascade_offset = (cascade_idx as f32) * 24.0;
        let win_pos = Pos2::new(base_x + cascade_offset, base_y + cascade_offset);

        let mut wnd = DownloadWindowState::new(
            url.to_string(),
            "video.mp4".to_string(),
            "Video".to_string(),
            "C:\\Downloads".to_string(),
        );
        wnd.cascade_index = cascade_idx;
        wnd.window_pos = Some(win_pos);
        wnd.is_open = true;

        if !key.is_empty() {
            self.recent_dialog_keys.insert(key.clone(), Instant::now());
        }

        self.download_windows.push(wnd);
        self.request_log.push(format!(
            "DIALOG_OPEN: Created dialog idx={} for key '{}' at ({}, {}) [cascade={}]",
            self.download_windows.len() - 1, key, win_pos.x, win_pos.y, cascade_idx
        ));
    }

    fn start_download(&mut self, window_idx: usize) {
        if window_idx < self.download_windows.len() {
            let key = normalize_url_key(&self.download_windows[window_idx].url);
            if !key.is_empty() {
                self.download_started_at.insert(key.clone(), Instant::now());
                self.running_downloads.push(key);
            }
            self.download_windows[window_idx].stage = DownloadWindowStage::Downloading;
        }
    }

    fn close_dialog(&mut self, window_idx: usize) {
        if window_idx < self.download_windows.len() {
            self.download_windows[window_idx].is_open = false;
            // Pop queue if available
            let open_count = self.download_windows.iter().filter(|w| w.is_open).count();
            if open_count < 5 {
                if let Some(next_req) = self.pending_dialog_queue.pop_front() {
                    self.show_download_dialog(next_req, "queue");
                }
            }
        }
    }
}

#[test]
fn test_1_app_not_running_click_video_download_button_once() {
    // App not running: starts in popup mode (cold start)
    let mut app = TestAppMock::new(false, true);

    let req = DownloadDialogRequest {
        url: "https://www.youtube.com/watch?v=dQw4w9WgXcQ".to_string(),
        filename: Some("Rick Astley - Never Gonna Give You Up.mp4".to_string()),
        cookies: None,
        referer: None,
        user_agent: None,
        num_segments: None,
        ..Default::default()
    };

    app.show_download_dialog(req, "extension");

    // Exactly ONE dialog, and log shows one request
    assert_eq!(app.download_windows.len(), 1, "Must open exactly ONE dialog");
    assert!(app.download_windows[0].is_open);
    assert_eq!(app.download_windows[0].stage, DownloadWindowStage::Info);
    assert!(!app.is_main_window_open.load(Ordering::Relaxed), "Main window must NOT be opened");

    let requests_received: Vec<&String> = app.request_log.iter().filter(|l| l.contains("source='extension'")).collect();
    assert_eq!(requests_received.len(), 1, "Log must show exactly one extension request");
}

#[test]
fn test_2_app_already_running_click_once() {
    // App already running: main dashboard open
    let mut app = TestAppMock::new(true, false);

    let req = DownloadDialogRequest {
        url: "https://www.youtube.com/watch?v=dQw4w9WgXcQ".to_string(),
        filename: Some("Rick Astley - Never Gonna Give You Up.mp4".to_string()),
        cookies: None,
        referer: None,
        user_agent: None,
        num_segments: None,
        ..Default::default()
    };

    app.show_download_dialog(req, "extension");

    // Exactly ONE dialog opened as viewport, centered over main window
    assert_eq!(app.download_windows.len(), 1, "Must open exactly ONE dialog");
    assert!(app.download_windows[0].is_open);
    let pos = app.download_windows[0].window_pos.expect("Must have window_pos");
    // Main window center: (220, 145)
    assert_eq!(pos, Pos2::new(220.0, 145.0), "Dialog must be centered over main window");
}

#[test]
fn test_3_click_button_3_times_quickly_still_one_dialog() {
    let mut app = TestAppMock::new(true, false);

    let req1 = DownloadDialogRequest {
        url: "https://www.youtube.com/watch?v=dQw4w9WgXcQ".to_string(),
        filename: Some("video.mp4".to_string()),
        cookies: None,
        referer: None,
        user_agent: None,
        num_segments: None,
        ..Default::default()
    };
    let req2 = DownloadDialogRequest {
        url: "https://www.youtube.com/watch?v=dQw4w9WgXcQ&list=RD123&index=2".to_string(),
        filename: Some("video.mp4".to_string()),
        cookies: None,
        referer: None,
        user_agent: None,
        num_segments: None,
        ..Default::default()
    };
    let req3 = DownloadDialogRequest {
        url: "https://youtu.be/dQw4w9WgXcQ?si=abc&t=10s".to_string(),
        filename: Some("video.mp4".to_string()),
        cookies: None,
        referer: None,
        user_agent: None,
        num_segments: None,
        ..Default::default()
    };

    // Click 1
    app.show_download_dialog(req1, "extension");
    assert_eq!(app.download_windows.len(), 1);

    // Click 2 (rapid)
    app.show_download_dialog(req2, "extension");
    assert_eq!(app.download_windows.len(), 1, "Rapid click 2 must NOT open a second dialog");

    // Click 3 (rapid)
    app.show_download_dialog(req3, "extension");
    assert_eq!(app.download_windows.len(), 1, "Rapid click 3 must NOT open a third dialog");

    // Existing dialog is still open and focused
    assert!(app.download_windows[0].is_open);
}

#[test]
fn test_4_navigate_between_5_different_youtube_videos_click_button_once() {
    let mut app = TestAppMock::new(true, false);

    // Simulate SPA navigation across 5 videos
    let videos = [
        "https://www.youtube.com/watch?v=video_one",
        "https://www.youtube.com/watch?v=video_two",
        "https://www.youtube.com/watch?v=video_three",
        "https://www.youtube.com/watch?v=video_four",
        "https://www.youtube.com/watch?v=video_five",
    ];

    // User navigates between 5 videos without clicking on 1-4.
    // Finally on video 5, user clicks the button once.
    let final_req = DownloadDialogRequest {
        url: videos[4].to_string(),
        filename: Some("video_five.mp4".to_string()),
        cookies: None,
        referer: None,
        user_agent: None,
        num_segments: None,
        ..Default::default()
    };

    app.show_download_dialog(final_req, "extension");

    assert_eq!(app.download_windows.len(), 1, "Exactly ONE dialog opened");
    assert_eq!(normalize_url_key(&app.download_windows[0].url), "youtube:video_five");
    assert!(app.download_windows[0].is_open);
}

#[test]
fn test_5_open_two_different_videos_cascaded_24px() {
    let mut app = TestAppMock::new(true, false);

    let req_a = DownloadDialogRequest {
        url: "https://www.youtube.com/watch?v=video_AAA".to_string(),
        filename: Some("video_AAA.mp4".to_string()),
        cookies: None,
        referer: None,
        user_agent: None,
        num_segments: None,
        ..Default::default()
    };

    let req_b = DownloadDialogRequest {
        url: "https://www.youtube.com/watch?v=video_BBB".to_string(),
        filename: Some("video_BBB.mp4".to_string()),
        cookies: None,
        referer: None,
        user_agent: None,
        num_segments: None,
        ..Default::default()
    };

    app.show_download_dialog(req_a, "extension");
    app.show_download_dialog(req_b, "extension");

    // Two dialogs opened
    assert_eq!(app.download_windows.len(), 2, "Two distinct videos open two dialogs");
    assert!(app.download_windows[0].is_open);
    assert!(app.download_windows[1].is_open);

    let pos_a = app.download_windows[0].window_pos.unwrap();
    let pos_b = app.download_windows[1].window_pos.unwrap();

    // Verify 24px cascading offset
    assert_eq!(pos_b.x - pos_a.x, 24.0, "Cascade X offset must be exactly 24px");
    assert_eq!(pos_b.y - pos_a.y, 24.0, "Cascade Y offset must be exactly 24px");

    assert_eq!(app.download_windows[0].cascade_index, 0);
    assert_eq!(app.download_windows[1].cascade_index, 1);
}

#[test]
fn test_6_start_download_then_click_button_again_suppressed() {
    let mut app = TestAppMock::new(true, false);

    let req = DownloadDialogRequest {
        url: "https://www.youtube.com/watch?v=video_active".to_string(),
        filename: Some("video_active.mp4".to_string()),
        cookies: None,
        referer: None,
        user_agent: None,
        num_segments: None,
        ..Default::default()
    };

    // 1. Open dialog
    app.show_download_dialog(req.clone(), "extension");
    assert_eq!(app.download_windows.len(), 1);

    // 2. Press Start download
    app.start_download(0);
    assert_eq!(app.download_windows[0].stage, DownloadWindowStage::Downloading);

    // 3. User clicks the video Download button again in the browser for the same video
    app.show_download_dialog(req, "extension");

    // 4. Must NOT open a second dialog or start a second download
    assert_eq!(app.download_windows.len(), 1, "Must NOT open a second dialog while downloading");
    assert_eq!(app.running_downloads.len(), 1, "Must NOT start a second download");
}

#[test]
fn test_queue_overflow_more_than_5_dialogs() {
    let mut app = TestAppMock::new(true, false);

    for i in 0..7 {
        let req = DownloadDialogRequest {
            url: format!("https://example.com/file_{}.zip", i),
            filename: Some(format!("file_{}.zip", i)),
            cookies: None,
            referer: None,
            user_agent: None,
            num_segments: None,
            ..Default::default()
        };
        app.show_download_dialog(req, "extension");
    }

    // Maximum 5 open windows at once
    let open_count = app.download_windows.iter().filter(|w| w.is_open).count();
    assert_eq!(open_count, 5, "Maximum 5 dialogs open simultaneously");
    assert_eq!(app.pending_dialog_queue.len(), 2, "Extra 2 requests must wait in queue");

    // Close one window -> queue pops next request
    app.close_dialog(0);
    let open_count_after = app.download_windows.iter().filter(|w| w.is_open).count();
    assert_eq!(open_count_after, 5, "Queue pop keeps 5 dialogs open");
    assert_eq!(app.pending_dialog_queue.len(), 1, "1 remaining in queue");
}
