use std::collections::VecDeque;
use std::time::{Duration, Instant};
use eframe::egui::{self, Color32, Margin, Pos2, Rect, Stroke, Vec2};
use crate::theme::{shapes, ThemeColors};
use crate::icons;
use crate::components::title_bar;
use crate::dialogs::add_download::{
    render_add_download_body,
    AddDialogOutcome, AddDialogState,
};

pub const DOWNLOAD_WINDOW_WIDTH: f32 = 560.0;
pub const DOWNLOAD_WINDOW_HEIGHT: f32 = 350.0;

static NEXT_DIALOG_ID: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
static SHARED_PROBE_CLIENT: std::sync::OnceLock<reqwest::Client> = std::sync::OnceLock::new();

fn get_shared_probe_client() -> &'static reqwest::Client {
    SHARED_PROBE_CLIENT.get_or_init(|| {
        reqwest::Client::builder()
            .timeout(Duration::from_secs(3))
            .redirect(reqwest::redirect::Policy::limited(10))
            .build()
            .unwrap_or_else(|_| reqwest::Client::new())
    })
}

impl Drop for DownloadWindowState {
    fn drop(&mut self) {
        if let Some(ref abort) = self.info_abort {
            abort.abort();
        }
        if let Some(ref abort) = self.probe_abort {
            abort.abort();
        }
        self.info_rx = None;
        self.probe_rx = None;
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum InfoLookupStatus {
    ReadingInfo,
    Success,
    Failed { error: String },
}

#[derive(Clone, Debug, PartialEq)]
pub enum DownloadWindowStage {
    /// Initial "Download File Info" dialog
    Info,
    /// Connecting to server (before first bytes)
    Connecting,
    /// View A: Downloading
    Downloading,
    /// View B: Paused
    Paused,
    /// Processing/merging (yt-dlp merge phase)
    Processing,
    /// View C: Complete
    Complete,
    /// View D: Failed
    Failed { reason: String },
}

pub struct DownloadWindowState {
    pub id: u64,
    pub dialog_id: String,
    pub created_at: Instant,
    pub topmost_cleared: bool,
    pub info_status: InfoLookupStatus,
    pub info_rx: Option<tokio::sync::mpsc::Receiver<Result<rustget_core::youtube::StreamingVideoInfo, String>>>,
    pub probe_rx: Option<tokio::sync::mpsc::Receiver<Result<rustget_core::probe::FileInfo, String>>>,
    pub info_abort: Option<tokio::task::AbortHandle>,
    pub probe_abort: Option<tokio::task::AbortHandle>,
    pub task_id: String,
    pub stage: DownloadWindowStage,
    pub url: String,
    pub filename: String,
    pub category: String,
    pub save_dir: String,
    pub target_path: String,
    pub connections: usize,
    pub speed_limit_kb: u64,
    pub checksum: String,
    pub remember_category: bool,
    pub description: String,
    pub file_size: Option<u64>,
    pub selected_quality: Option<String>,
    pub selected_format_id: Option<String>,
    pub is_size_approx: Option<bool>,
    pub formatted_size: Option<String>,
    pub is_probing: bool,
    pub error_message: Option<String>,
    pub show_notes_expanded: bool,
    pub show_advanced_expanded: bool,

    // Runtime download metrics
    pub downloaded_bytes: u64,
    pub total_bytes: Option<u64>,
    pub speed_bps: f64,
    pub eta_seconds: Option<u64>,
    pub speed_samples: VecDeque<(Instant, u64)>,

    // Throttled UI display metrics (updated at most 4x/sec = 250ms)
    pub last_ui_update: Instant,
    pub disp_downloaded: u64,
    pub disp_total: Option<u64>,
    pub disp_speed_bps: f64,
    pub disp_eta_seconds: Option<u64>,

    // UI state
    pub is_open: bool,
    pub has_painted: bool,
    pub initial_pos_applied: bool,
    pub copy_button_copied_at: Option<Instant>,
    pub confirming_cancel: bool,
    pub cascade_index: usize,
    pub host: String,
    pub window_pos: Option<Pos2>,
}

impl DownloadWindowState {
    pub fn viewport_id(&self) -> egui::ViewportId {
        egui::ViewportId::from_hash_of(("dialog", self.id))
    }

    pub fn new(url: String, mut filename: String, category: String, save_dir: String) -> Self {
        let host = extract_host(&url);
        let now = Instant::now();
        let id = NEXT_DIALOG_ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let dialog_id = id.to_string();

        let is_torrent = rustget_core::torrent::is_torrent_url(&url);
        let mut torrent_size = None;
        if is_torrent {
            if let Some((meta_name, size)) = rustget_core::torrent::read_torrent_file_metadata(&url) {
                if filename.is_empty() || filename.eq_ignore_ascii_case("download") || filename.ends_with(".torrent") || filename.ends_with(".TORRENT") {
                    filename = meta_name;
                }
                torrent_size = Some(size);
            } else if filename.is_empty() || filename.eq_ignore_ascii_case("download") {
                filename = rustget_core::torrent::extract_torrent_name(&url);
            }
        }

        let is_stream = !is_torrent && rustget_core::youtube::is_streaming_site(&url);
        let has_real_filename = !filename.is_empty()
            && !filename.eq_ignore_ascii_case("YouTube_Video.mp4")
            && !filename.eq_ignore_ascii_case("watch.mp4")
            && !filename.eq_ignore_ascii_case("video.mp4");

        let mut info_abort = None;
        let mut probe_abort = None;

        let (info_status, info_rx, probe_rx) = if is_stream && !url.trim().is_empty() {
            if has_real_filename {
                crate::platform::log_daemon(&format!(
                    "STEP: video info lookup bypassed (real filename provided by extension): url='{}', filename='{}'",
                    url, filename
                ));
                (InfoLookupStatus::Success, None, None)
            } else {
                crate::platform::log_daemon(&format!(
                    "STEP: video info lookup started for url='{}' on dialog_id='{}'",
                    url, dialog_id
                ));
                let (tx, rx) = tokio::sync::mpsc::channel(1);
                let lookup_url = url.clone();
                if let Ok(handle) = tokio::runtime::Handle::try_current() {
                    let task = handle.spawn(async move {
                        let res = rustget_core::youtube::query_streaming_video_info(&lookup_url).await;
                        let _ = tx.send(res).await;
                    });
                    info_abort = Some(task.abort_handle());
                } else {
                    std::thread::spawn(move || {
                        let rt = tokio::runtime::Builder::new_current_thread()
                            .enable_all()
                            .build();
                        if let Ok(rt) = rt {
                            let res = rt.block_on(rustget_core::youtube::query_streaming_video_info(&lookup_url));
                            let _ = tx.blocking_send(res);
                        }
                    });
                }
                (InfoLookupStatus::ReadingInfo, Some(rx), None)
            }
        } else if !url.trim().is_empty() && (url.starts_with("http://") || url.starts_with("https://")) {
            crate::platform::log_daemon(&format!(
                "STEP: generic file probe started for url='{}' on dialog_id='{}'",
                url, dialog_id
            ));
            let (tx, rx) = tokio::sync::mpsc::channel(1);
            let probe_url_str = url.clone();
            if let Ok(handle) = tokio::runtime::Handle::try_current() {
                let task = handle.spawn(async move {
                    let parsed_res = rustget_core::probe::validate_http_url(&probe_url_str);
                    match parsed_res {
                        Ok(parsed) => {
                            let client = get_shared_probe_client();
                            let res = rustget_core::probe::probe_url(client, &parsed, &reqwest::header::HeaderMap::new()).await;
                            match res {
                                Ok(fi) => { let _ = tx.send(Ok(fi)).await; }
                                Err(e) => { let _ = tx.send(Err(e.to_string())).await; }
                            }
                        }
                        Err(e) => {
                            let _ = tx.send(Err(e.to_string())).await;
                        }
                    }
                });
                probe_abort = Some(task.abort_handle());
            } else {
                std::thread::spawn(move || {
                    let rt = tokio::runtime::Builder::new_current_thread()
                        .enable_all()
                        .build();
                    if let Ok(rt) = rt {
                        rt.block_on(async {
                            let parsed_res = rustget_core::probe::validate_http_url(&probe_url_str);
                            match parsed_res {
                                Ok(parsed) => {
                                    let client = get_shared_probe_client();
                                    let res = rustget_core::probe::probe_url(client, &parsed, &reqwest::header::HeaderMap::new()).await;
                                    match res {
                                        Ok(fi) => { let _ = tx.send(Ok(fi)).await; }
                                        Err(e) => { let _ = tx.send(Err(e.to_string())).await; }
                                    }
                                }
                                Err(e) => {
                                    let _ = tx.send(Err(e.to_string())).await;
                                }
                            }
                        });
                    }
                });
            }
            (InfoLookupStatus::ReadingInfo, None, Some(rx))
        } else {
            (InfoLookupStatus::Success, None, None)
        };

        let (cached_quality, cached_size, cached_size_formatted, cached_approx, cached_fid) =
            if is_stream {
                if let Some(cached) = rustget_core::youtube::get_cached_video_formats(&url) {
                    if let Some(ref best) = cached.best {
                        (
                            Some(best.label.clone()),
                            best.size,
                            Some(best.size_formatted.clone()),
                            Some(best.is_approx),
                            Some(best.format_id.clone()),
                        )
                    } else {
                        (None, None, None, None, None)
                    }
                } else {
                    (None, None, None, None, None)
                }
            } else if let Some(sz) = torrent_size {
                (
                    None,
                    Some(sz),
                    Some(format_size_1dec(sz)),
                    Some(false),
                    None,
                )
            } else {
                (None, None, None, None, None)
            };

        Self {
            id,
            dialog_id,
            created_at: now,
            topmost_cleared: false,
            info_status,
            info_rx,
            probe_rx,
            info_abort,
            probe_abort,
            task_id: String::new(),
            stage: DownloadWindowStage::Info,
            url,
            filename,
            category,
            save_dir,
            target_path: String::new(),
            connections: 8,
            speed_limit_kb: 0,
            checksum: String::new(),
            remember_category: true,
            description: String::new(),
            file_size: cached_size,
            selected_quality: cached_quality,
            selected_format_id: cached_fid,
            is_size_approx: cached_approx,
            formatted_size: cached_size_formatted,
            is_probing: false,
            error_message: None,
            show_notes_expanded: false,
            show_advanced_expanded: false,

            downloaded_bytes: 0,
            total_bytes: cached_size,
            speed_bps: 0.0,
            eta_seconds: None,
            speed_samples: VecDeque::new(),

            last_ui_update: now - Duration::from_millis(300),
            disp_downloaded: 0,
            disp_total: cached_size,
            disp_speed_bps: 0.0,
            disp_eta_seconds: None,

            is_open: true,
            has_painted: false,
            initial_pos_applied: false,
            copy_button_copied_at: None,
            confirming_cancel: false,
            cascade_index: 0,
            host,
            window_pos: None,
        }
    }

    pub fn poll_info_rx(&mut self) {
        let _ = self.poll_probes();
    }

    pub fn poll_probes(&mut self) -> bool {
        let mut updated = false;
        if let Some(ref mut rx) = self.info_rx {
            if let Ok(res) = rx.try_recv() {
                match res {
                    Ok(info) => {
                        crate::platform::log_daemon(&format!(
                            "STEP: video info lookup finished for url='{}': title='{}', size={:?}",
                            self.url, info.title, info.filesize_approx
                        ));
                        self.filename = info.filename;
                        if let Some(cached) = rustget_core::youtube::get_cached_video_formats(&self.url) {
                            if self.selected_quality.is_none() {
                                if let Some(ref best) = cached.best {
                                    self.selected_quality = Some(best.label.clone());
                                    self.formatted_size = Some(best.size_formatted.clone());
                                    self.is_size_approx = Some(best.is_approx);
                                    self.selected_format_id = Some(best.format_id.clone());
                                }
                            }
                        }
                        if let Some(sz) = info.filesize_approx {
                            if self.file_size.is_none() {
                                self.file_size = Some(sz);
                            }
                        }
                        self.info_status = InfoLookupStatus::Success;
                    }
                    Err(err) => {
                        crate::platform::log_daemon(&format!(
                            "STEP: video info lookup failed for url='{}': {}",
                            self.url, err
                        ));
                        self.info_status = InfoLookupStatus::Failed { error: err };
                    }
                }
                self.info_rx = None;
                updated = true;
            }
        }
        if let Some(ref mut rx) = self.probe_rx {
            if let Ok(res) = rx.try_recv() {
                match res {
                    Ok(info) => {
                        crate::platform::log_daemon(&format!(
                            "STEP: generic file probe finished for url='{}': suggested_filename='{}', size={:?}",
                            self.url, info.suggested_filename, info.content_length
                        ));
                        let cur_name = self.filename.trim().to_ascii_lowercase();
                        if cur_name.is_empty()
                            || cur_name == "download"
                            || cur_name == "download.bin"
                            || cur_name == "watch"
                            || cur_name == "video"
                            || (!self.filename.contains('.') && info.suggested_filename.contains('.'))
                        {
                            self.filename = info.suggested_filename;
                        }
                        self.file_size = info.content_length;
                        self.info_status = InfoLookupStatus::Success;
                    }
                    Err(err) => {
                        crate::platform::log_daemon(&format!(
                            "STEP: generic file probe failed (fallback to unknown size) for url='{}': {}",
                            self.url, err
                        ));
                        self.file_size = None;
                        self.info_status = InfoLookupStatus::Success;
                    }
                }
                self.probe_rx = None;
                updated = true;
            }
        }
        updated
    }

    pub fn retry_info_lookup(&mut self) {
        if rustget_core::youtube::is_streaming_site(&self.url) && !self.url.trim().is_empty() {
            if let Some(ref abort) = self.info_abort {
                abort.abort();
            }
            crate::platform::log_daemon(&format!(
                "STEP: video info lookup retried for url='{}' on dialog_id='{}'",
                self.url, self.dialog_id
            ));
            self.info_status = InfoLookupStatus::ReadingInfo;
            let (tx, rx) = tokio::sync::mpsc::channel(1);
            self.info_rx = Some(rx);
            let lookup_url = self.url.clone();
            if let Ok(handle) = tokio::runtime::Handle::try_current() {
                let task = handle.spawn(async move {
                    let res = rustget_core::youtube::query_streaming_video_info(&lookup_url).await;
                    let _ = tx.send(res).await;
                });
                self.info_abort = Some(task.abort_handle());
            } else {
                std::thread::spawn(move || {
                    let rt = tokio::runtime::Builder::new_current_thread()
                        .enable_all()
                        .build();
                    if let Ok(rt) = rt {
                        let res = rt.block_on(rustget_core::youtube::query_streaming_video_info(&lookup_url));
                        let _ = tx.blocking_send(res);
                    }
                });
            }
        }
    }

    pub fn use_link_as_is(&mut self) {
        if let Some(ref abort) = self.info_abort {
            abort.abort();
            self.info_abort = None;
        }
        if let Some(ref abort) = self.probe_abort {
            abort.abort();
            self.probe_abort = None;
        }
        crate::platform::log_daemon(&format!(
            "STEP: video info lookup skipped, using link as is for url='{}'",
            self.url
        ));
        self.info_status = InfoLookupStatus::Success;
        self.info_rx = None;
        self.probe_rx = None;
        if self.filename.is_empty() || self.filename == "video.mp4" {
            self.filename = "download.mp4".to_string();
        }
    }

    pub fn update_progress(&mut self, downloaded: u64, total: Option<u64>, now: Instant) {
        self.downloaded_bytes = downloaded;
        if total.is_some() {
            self.total_bytes = total;
        }

        // Add to speed samples
        self.speed_samples.push_back((now, downloaded));
        let three_sec_ago = now.checked_sub(Duration::from_secs(3)).unwrap_or(now);
        while self.speed_samples.len() > 1 && self.speed_samples.front().map(|(t, _)| *t < three_sec_ago).unwrap_or(false) {
            self.speed_samples.pop_front();
        }

        // Moving average speed over the last 3 seconds
        if let (Some((first_t, first_b)), Some((last_t, last_b))) = (self.speed_samples.front(), self.speed_samples.back()) {
            let dt = (*last_t - *first_t).as_secs_f64();
            if dt > 0.05 && *last_b >= *first_b {
                self.speed_bps = (*last_b - *first_b) as f64 / dt;
            }
        }

        // Time left = remaining size / average speed
        if let Some(tot) = self.total_bytes {
            if tot > downloaded && self.speed_bps > 10.0 {
                let remaining = tot - downloaded;
                self.eta_seconds = Some((remaining as f64 / self.speed_bps).ceil() as u64);
            } else if tot <= downloaded {
                self.eta_seconds = Some(0);
            } else {
                self.eta_seconds = None;
            }
        } else {
            self.eta_seconds = None;
        }

        // Throttle UI update to at most 4 times per second (250ms)
        if now.duration_since(self.last_ui_update) >= Duration::from_millis(250) {
            self.last_ui_update = now;
            self.disp_downloaded = self.downloaded_bytes;
            self.disp_total = self.total_bytes;
            self.disp_speed_bps = self.speed_bps;
            self.disp_eta_seconds = self.eta_seconds;
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DownloadWindowOutcome {
    None,
    StartDownload,
    DownloadLater,
    UrlChanged,
    CategoryChanged,
    OpenSettings,
    Pause,
    Resume,
    RunInBackground,
    CancelClicked,
    ConfirmCancel,
    DismissCancel,
    OpenFile,
    ShowInFolder,
    Retry,
    Close,
    Minimize,
}

pub fn extract_host(url: &str) -> String {
    if let Ok(parsed) = url::Url::parse(url) {
        if let Some(h) = parsed.host_str() {
            return h.to_string();
        }
    }
    "download".to_string()
}

pub fn format_size_1dec(bytes: u64) -> String {
    if bytes < 1024 {
        format!("{} B", bytes)
    } else if bytes < 1024 * 1024 {
        format!("{:.1} KB", bytes as f64 / 1024.0)
    } else if bytes < 1024 * 1024 * 1024 {
        format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
    } else {
        format!("{:.1} GB", bytes as f64 / (1024.0 * 1024.0 * 1024.0))
    }
}

pub fn format_speed_1dec(bps: f64) -> String {
    if bps < 1024.0 {
        format!("{:.0} B/s", bps)
    } else if bps < 1024.0 * 1024.0 {
        format!("{:.1} KB/s", bps / 1024.0)
    } else if bps < 1024.0 * 1024.0 * 1024.0 {
        format!("{:.1} MB/s", bps / (1024.0 * 1024.0))
    } else {
        format!("{:.1} GB/s", bps / (1024.0 * 1024.0 * 1024.0))
    }
}

pub fn format_time_left(seconds: u64) -> String {
    if seconds < 60 {
        format!("{} s", seconds)
    } else if seconds < 3600 {
        let mins = seconds / 60;
        let secs = seconds % 60;
        if secs > 0 {
            format!("{} min {} s", mins, secs)
        } else {
            format!("{} min", mins)
        }
    } else {
        let hours = seconds / 3600;
        let remaining_mins = (seconds % 3600) / 60;
        if remaining_mins > 0 {
            format!("{} h {} min", hours, remaining_mins)
        } else {
            format!("{} h", hours)
        }
    }
}

pub fn format_failed_reason(raw: &str) -> String {
    let lower = raw.to_ascii_lowercase();
    if lower.contains("connection lost") || lower.contains("connection reset") || lower.contains("network") || lower.contains("timed out") || lower.contains("timeout") || lower.contains("broken pipe") {
        "Connection lost (resumable)".to_string()
    } else if lower.contains("403") || lower.contains("410") || lower.contains("forbidden") || lower.contains("expired") || lower.contains("gone") {
        "Link expired or forbidden (HTTP 403/410)".to_string()
    } else if lower.contains("space") || lower.contains("disk full") || lower.contains("no space left") {
        "Disk full".to_string()
    } else if lower.contains("access denied") || lower.contains("permission denied") || lower.contains("not writable") || lower.contains("readonly") {
        "Access denied or path not writable".to_string()
    } else if lower.contains("antivirus") || lower.contains("virus") || lower.contains("operation did not complete successfully because the file contains a virus") || lower.contains("locked") {
        "File locked by antivirus".to_string()
    } else {
        raw.to_string()
    }
}

pub fn build_file_summary_meta(state: &DownloadWindowState) -> String {
    let size_str = if let Some(tot) = state.total_bytes.or(state.file_size) {
        Some(format_size_1dec(tot))
    } else if let Some(ref fs) = state.formatted_size {
        Some(fs.clone())
    } else {
        None
    };

    let quality = state.selected_quality.as_deref();

    match (quality, size_str) {
        (Some(q), Some(s)) => format!("{} · {} · {}", state.category, q, s),
        (Some(q), None) => format!("{} · {}", state.category, q),
        (None, Some(s)) => format!("{} · {}", state.category, s),
        (None, None) => {
            if let Ok(parsed) = url::Url::parse(state.url.trim()) {
                let host = parsed.host_str().unwrap_or("link").trim_start_matches("www.");
                format!("{} · {}", state.category, host)
            } else {
                format!("{} · File", state.category)
            }
        }
    }
}

pub fn render_file_summary_row(
    ui: &mut egui::Ui,
    pal: &ThemeColors,
    state: &DownloadWindowState,
) {
    let card_w = ui.available_width();
    let card_rect = ui.allocate_space(Vec2::new(card_w, 48.0)).1;

    ui.painter().line_segment(
        [card_rect.left_bottom(), card_rect.right_bottom()],
        Stroke::new(1.0_f32, pal.line),
    );

    let mid_y = card_rect.center().y;
    let icon_box = Rect::from_center_size(Pos2::new(card_rect.left() + 18.0, mid_y), Vec2::splat(34.0));
    ui.painter().rect_filled(icon_box, shapes::R_SM, pal.accent_soft);
    let icon_rect = Rect::from_center_size(icon_box.center(), Vec2::splat(18.0));
    icons::draw_category_symbol(ui.painter(), icon_rect, &state.category, pal.accent);

    let display_name = if state.filename.trim().is_empty() {
        "File will be resolved from link"
    } else {
        state.filename.as_str()
    };
    let meta_text = build_file_summary_meta(state);

    let text_rect = Rect::from_min_max(
        Pos2::new(card_rect.left() + 44.0, card_rect.top() + 4.0),
        Pos2::new(card_rect.right() - 8.0, card_rect.bottom() - 4.0),
    );

    ui.allocate_new_ui(egui::UiBuilder::new().max_rect(text_rect), |ui| {
        ui.set_clip_rect(text_rect);
        ui.spacing_mut().item_spacing.y = 2.0;
        ui.add(
            egui::Label::new(
                egui::RichText::new(display_name)
                    .size(13.5)
                    .strong()
                    .color(pal.text),
            )
            .truncate(),
        );

        ui.add(
            egui::Label::new(
                egui::RichText::new(meta_text)
                    .size(12.0)
                    .color(pal.muted),
            )
            .truncate(),
        );
    });
}

/// Renders the complete IDM-style download window content (Title Bar + Central Panel + Footer).
pub fn render_download_window_content(
    ctx: &egui::Context,
    pal: &ThemeColors,
    logo_texture: Option<&egui::TextureHandle>,
    state: &mut DownloadWindowState,
) -> DownloadWindowOutcome {
    crate::theme::ensure_fonts_for_text(ctx, &state.filename);
    crate::theme::ensure_fonts_for_text(ctx, &state.url);
    if !state.has_painted {
        #[cfg(windows)]
        {
            let hwnd = crate::platform::windows::get_dialog_window();
            if !hwnd.is_null() {
                state.has_painted = true;
                crate::platform::log_daemon(&format!(
                    "STEP: first paint for window stage={:?}, url='{}'",
                    state.stage, state.url
                ));
                crate::platform::windows::show_download_window_hwnd(hwnd);
            } else {
                ctx.request_repaint();
            }
        }
        #[cfg(not(windows))]
        {
            state.has_painted = true;
        }
    }

    state.poll_info_rx();
    if (state.info_rx.is_some() || state.probe_rx.is_some()) && state.created_at.elapsed() < Duration::from_secs(5) {
        ctx.request_repaint_after(Duration::from_millis(250));
    }

    let mut outcome = DownloadWindowOutcome::None;

    // 1. Top Title Bar (Height: 36px, stays "Download File Info" in every state)
    let title_id = egui::Id::new((ctx.viewport_id(), "dl_wnd_title_bar", &state.dialog_id));
    let title_act = egui::TopBottomPanel::top(title_id)
        .resizable(false)
        .exact_height(36.0)
        .frame(egui::Frame::none().fill(pal.raise).inner_margin(Margin::ZERO))
        .show(ctx, |ui| {
            title_bar::render_title_bar(
                ui,
                ctx,
                pal,
                logo_texture,
                "Download File Info",
            )
        })
        .inner;

    if title_act.minimize {
        ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(true));
        outcome = DownloadWindowOutcome::Minimize;
    }

    if title_act.close {
        match state.stage {
            DownloadWindowStage::Info => {
                outcome = DownloadWindowOutcome::Close;
            }
            DownloadWindowStage::Connecting | DownloadWindowStage::Downloading | DownloadWindowStage::Paused | DownloadWindowStage::Processing => {
                // X button during download closes window to background
                outcome = DownloadWindowOutcome::RunInBackground;
            }
            DownloadWindowStage::Complete | DownloadWindowStage::Failed { .. } => {
                outcome = DownloadWindowOutcome::Close;
            }
        }
    }

    // 2. Bottom Footer (Height: 59px = 1px border + 12px padding + 34px buttons + 12px padding)
    let footer_id = egui::Id::new((ctx.viewport_id(), "dl_wnd_footer", &state.dialog_id));
    egui::TopBottomPanel::bottom(footer_id)
        .resizable(false)
        .exact_height(59.0)
        .frame(
            egui::Frame::none()
                .fill(pal.surface)
                .inner_margin(Margin { left: 24.0, right: 24.0, top: 0.0, bottom: 0.0 }),
        )
        .show(ctx, |ui| {
            render_window_footer(ui, pal, state, &mut outcome);
        });

    // 3. Central Panel (Height: 285px = 380 - 36 - 59)
    egui::CentralPanel::default()
        .frame(
            egui::Frame::none()
                .fill(pal.surface)
                .inner_margin(Margin { left: 24.0, right: 24.0, top: 20.0, bottom: 16.0 }),
        )
        .show(ctx, |ui| {
            render_window_body(ui, ctx, pal, state, &mut outcome);
        });

    // Request repaint for active downloads (4 Hz)
    if matches!(state.stage, DownloadWindowStage::Downloading | DownloadWindowStage::Connecting | DownloadWindowStage::Processing) {
        ctx.request_repaint_after(Duration::from_millis(250));
    }

    outcome
}

/// Renders the complete IDM-style download window into a designated rectangle (used for multi-window previews & cascading tests).
pub fn render_download_window_card(
    ui: &mut egui::Ui,
    ctx: &egui::Context,
    pal: &ThemeColors,
    logo_texture: Option<&egui::TextureHandle>,
    state: &mut DownloadWindowState,
    pos: Pos2,
) -> DownloadWindowOutcome {
    let mut outcome = DownloadWindowOutcome::None;
    let card_rect = Rect::from_min_size(pos, Vec2::new(DOWNLOAD_WINDOW_WIDTH, DOWNLOAD_WINDOW_HEIGHT));

    // Shadow & window body
    ui.painter().rect_filled(card_rect, shapes::R, pal.surface);
    ui.painter().rect_stroke(card_rect, shapes::R, egui::Stroke::new(1.0_f32, pal.line));

    let bg_resp = ui.interact(card_rect, ui.id().with(("dialog_bg_drag", &state.dialog_id)), egui::Sense::click_and_drag());
    if bg_resp.drag_started() || bg_resp.dragged() {
        ctx.send_viewport_cmd(egui::ViewportCommand::StartDrag);
        #[cfg(windows)]
        crate::platform::drag_window_under_cursor();
    }

    // 1. Title bar
    let title_rect = Rect::from_min_size(pos, Vec2::new(DOWNLOAD_WINDOW_WIDTH, 36.0));
    ui.allocate_new_ui(egui::UiBuilder::new().max_rect(title_rect), |ui| {
        let t_act = title_bar::render_title_bar(ui, ctx, pal, logo_texture, "Download File Info");
        if t_act.minimize {
            ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(true));
            outcome = DownloadWindowOutcome::Minimize;
        }
        if t_act.close {
            outcome = match state.stage {
                DownloadWindowStage::Info => DownloadWindowOutcome::Close,
                DownloadWindowStage::Connecting | DownloadWindowStage::Downloading | DownloadWindowStage::Paused | DownloadWindowStage::Processing => {
                    DownloadWindowOutcome::RunInBackground
                }
                DownloadWindowStage::Complete | DownloadWindowStage::Failed { .. } => {
                    DownloadWindowOutcome::Close
                }
            };
        }
    });

    // 2. Body
    let body_rect = Rect::from_min_size(
        pos + Vec2::new(24.0, 48.0),
        Vec2::new(DOWNLOAD_WINDOW_WIDTH - 48.0, 235.0),
    );
    ui.allocate_new_ui(egui::UiBuilder::new().max_rect(body_rect), |ui| {
        render_window_body(ui, ctx, pal, state, &mut outcome);
    });

    // 3. Footer
    let footer_rect = Rect::from_min_size(
        pos + Vec2::new(24.0, DOWNLOAD_WINDOW_HEIGHT - 52.0),
        Vec2::new(DOWNLOAD_WINDOW_WIDTH - 48.0, 48.0),
    );
    ui.allocate_new_ui(egui::UiBuilder::new().max_rect(footer_rect), |ui| {
        render_window_footer(ui, pal, state, &mut outcome);
    });

    outcome
}

/// Renders the complete IDM-style download window into a centered modal inside the main window.
pub fn render_download_window_modal(
    ctx: &egui::Context,
    pal: &ThemeColors,
    logo_texture: Option<&egui::TextureHandle>,
    state: &mut DownloadWindowState,
    cascade_offset: f32,
) -> DownloadWindowOutcome {
    crate::theme::ensure_fonts_for_text(ctx, &state.filename);
    crate::theme::ensure_fonts_for_text(ctx, &state.url);
    if !state.has_painted {
        state.has_painted = true;
        crate::platform::log_daemon(&format!(
            "STEP: first paint for window stage={:?}, url='{}'",
            state.stage, state.url
        ));
    }

    state.poll_info_rx();
    if state.info_rx.is_some() {
        ctx.request_repaint_after(Duration::from_millis(100));
    }

    let mut outcome = DownloadWindowOutcome::None;

    let window_id = egui::Id::new(("dl_modal_window", &state.dialog_id));
    egui::Window::new(format!("Download File Info###{}", state.dialog_id))
        .id(window_id)
        .order(egui::Order::Foreground)
        .collapsible(false)
        .resizable(false)
        .title_bar(false)
        .fixed_size(Vec2::new(DOWNLOAD_WINDOW_WIDTH, DOWNLOAD_WINDOW_HEIGHT))
        .anchor(
            egui::Align2::CENTER_CENTER,
            Vec2::new(cascade_offset, cascade_offset),
        )
        .frame(
            egui::Frame::none()
                .fill(pal.surface)
                .rounding(shapes::R)
                .stroke(Stroke::new(1.0_f32, pal.line))
                .shadow(egui::epaint::Shadow {
                    offset: Vec2::new(0.0, 8.0),
                    blur: 24.0,
                    spread: 0.0,
                    color: Color32::from_black_alpha(120),
                }),
        )
        .show(ctx, |ui| {
            ui.spacing_mut().item_spacing = Vec2::ZERO;

            // 1. Title bar (Height: 36px)
            let (title_rect, _) = ui.allocate_exact_size(
                Vec2::new(DOWNLOAD_WINDOW_WIDTH, 36.0),
                egui::Sense::hover(),
            );
            ui.allocate_new_ui(egui::UiBuilder::new().max_rect(title_rect), |ui| {
                let t_act = title_bar::render_title_bar(
                    ui,
                    ctx,
                    pal,
                    logo_texture,
                    "Download File Info",
                );
                if t_act.minimize {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(true));
                    outcome = DownloadWindowOutcome::Minimize;
                }
                if t_act.close {
                    outcome = match state.stage {
                        DownloadWindowStage::Info => DownloadWindowOutcome::Close,
                        DownloadWindowStage::Connecting | DownloadWindowStage::Downloading | DownloadWindowStage::Paused | DownloadWindowStage::Processing => {
                            DownloadWindowOutcome::RunInBackground
                        }
                        DownloadWindowStage::Complete | DownloadWindowStage::Failed { .. } => {
                            DownloadWindowOutcome::Close
                        }
                    };
                }
            });

            // 2. Body (Height: 255px = 350 - 36 - 59)
            let (body_rect, _) = ui.allocate_exact_size(
                Vec2::new(DOWNLOAD_WINDOW_WIDTH, 255.0),
                egui::Sense::hover(),
            );
            ui.allocate_new_ui(
                egui::UiBuilder::new().max_rect(body_rect.shrink2(Vec2::new(24.0, 10.0))),
                |ui| {
                    render_window_body(ui, ctx, pal, state, &mut outcome);
                },
            );

            // 3. Footer (Height: 59px)
            let (footer_rect, _) = ui.allocate_exact_size(
                Vec2::new(DOWNLOAD_WINDOW_WIDTH, 59.0),
                egui::Sense::hover(),
            );
            ui.allocate_new_ui(
                egui::UiBuilder::new().max_rect(footer_rect.shrink2(Vec2::new(24.0, 0.0))),
                |ui| {
                    render_window_footer(ui, pal, state, &mut outcome);
                },
            );
        });

    if state.stage == DownloadWindowStage::Downloading {
        ctx.request_repaint_after(Duration::from_millis(250));
    }

    outcome
}

fn render_window_body(
    ui: &mut egui::Ui,
    ctx: &egui::Context,
    pal: &ThemeColors,
    state: &mut DownloadWindowState,
    outcome: &mut DownloadWindowOutcome,
) {
    match &state.stage {
        DownloadWindowStage::Info => {
            let is_reading = state.info_status == InfoLookupStatus::ReadingInfo;
            let info_err = match &state.info_status {
                InfoLookupStatus::Failed { error } => Some(error.as_str()),
                _ => None,
            };
            let mut add_state = AddDialogState {
                url: &mut state.url,
                filename: &mut state.filename,
                category: &mut state.category,
                save_dir: &mut state.save_dir,
                connections: &mut state.connections,
                speed_limit_kb: &mut state.speed_limit_kb,
                checksum: &mut state.checksum,
                remember_category: &mut state.remember_category,
                description: &mut state.description,
                file_size: state.file_size,
                quality: state.selected_quality.as_deref(),
                formatted_size: state.formatted_size.as_deref(),
                is_probing: state.is_probing,
                is_reading_info: is_reading,
                info_error: info_err,
                error_message: state.error_message.as_deref(),
                show_notes_expanded: &mut state.show_notes_expanded,
                show_advanced_expanded: &mut state.show_advanced_expanded,
            };
            let mut add_outcome = AddDialogOutcome {
                start_now: false,
                download_later: false,
                cancel: false,
                open_settings: false,
                url_changed: false,
                category_changed: false,
                retry_info: false,
                use_link_as_is: false,
            };
            render_add_download_body(ui, pal, &mut add_state, &mut add_outcome);
            if add_outcome.retry_info {
                state.retry_info_lookup();
            }
            if add_outcome.use_link_as_is {
                state.use_link_as_is();
            }
            if add_outcome.url_changed {
                state.host = extract_host(&state.url);
                state.retry_info_lookup();
                *outcome = DownloadWindowOutcome::UrlChanged;
            }
            if add_outcome.category_changed {
                *outcome = DownloadWindowOutcome::CategoryChanged;
            }
            if add_outcome.open_settings {
                *outcome = DownloadWindowOutcome::OpenSettings;
            }
        }
        DownloadWindowStage::Connecting => {
            render_view_a_downloading(ui, ctx, pal, state);
        }
        DownloadWindowStage::Downloading => {
            render_view_a_downloading(ui, ctx, pal, state);
        }
        DownloadWindowStage::Paused => {
            render_view_b_paused(ui, pal, state);
        }
        DownloadWindowStage::Processing => {
            render_view_a_downloading(ui, ctx, pal, state);
        }
        DownloadWindowStage::Complete => {
            render_view_c_complete(ui, pal, state, outcome);
        }
        DownloadWindowStage::Failed { reason } => {
            render_view_d_failed(ui, pal, state, reason);
        }
    }
}

fn render_view_a_downloading(
    ui: &mut egui::Ui,
    ctx: &egui::Context,
    pal: &ThemeColors,
    state: &DownloadWindowState,
) {
    ui.spacing_mut().item_spacing.y = 0.0;

    // File summary row in every view
    render_file_summary_row(ui, pal, state);

    ui.add_space(14.0);

    // Status line: "Connecting…", "Downloading… 42%", or "Processing…" (16px, weight 600)
    let raw_ratio = if let Some(tot) = state.total_bytes.or(state.disp_total) {
        if tot > 0 {
            (state.downloaded_bytes as f32 / tot as f32).clamp(0.0, 1.0)
        } else {
            0.0
        }
    } else {
        0.0
    };

    let bar_id = egui::Id::new((ctx.viewport_id(), "dl_progress_bar", &state.dialog_id));
    let smooth_ratio = if matches!(state.stage, DownloadWindowStage::Downloading) {
        ctx.animate_value_with_time(bar_id, raw_ratio, 0.25)
    } else {
        raw_ratio
    };

    let status_text = match state.stage {
        DownloadWindowStage::Connecting => "Connecting…".to_string(),
        DownloadWindowStage::Processing => "Processing…".to_string(),
        _ => {
            if state.disp_total.is_some() || state.total_bytes.is_some() {
                format!("Downloading… {:.0}%", smooth_ratio * 100.0)
            } else {
                format!("Downloading… {}", format_size_1dec(state.downloaded_bytes))
            }
        }
    };

    ui.label(
        egui::RichText::new(status_text)
            .size(16.0)
            .strong()
            .color(pal.text),
    );

    ui.add_space(8.0);

    // Progress bar: 6px high, rounded, accent fill, track = raise color
    let (track_rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 6.0), egui::Sense::hover());
    ui.painter().rect_filled(track_rect, 3.0, pal.raise);

    let has_known_total = state.disp_total.is_some() || state.file_size.is_some();
    let use_indeterminate = (state.stage == DownloadWindowStage::Processing)
        || (state.stage == DownloadWindowStage::Connecting && !has_known_total)
        || (state.stage != DownloadWindowStage::Connecting && state.disp_total.is_none());

    if use_indeterminate {
        // Indeterminate bar animation
        let time = ctx.input(|i| i.time);
        let cycle = (time * 0.8).fract() as f32;
        let bar_w = track_rect.width() * 0.3;
        let start_x = track_rect.left() + (track_rect.width() + bar_w) * cycle - bar_w;
        let ind_rect = Rect::from_min_size(
            Pos2::new(start_x.clamp(track_rect.left(), track_rect.right()), track_rect.top()),
            Vec2::new(bar_w, 6.0),
        ).intersect(track_rect);
        ui.painter().rect_filled(ind_rect, 3.0, pal.accent);
    } else {
        let fill_w = (track_rect.width() * smooth_ratio).max(if smooth_ratio > 0.0 { 4.0 } else { 0.0 });
        let fill_rect = Rect::from_min_size(track_rect.min, Vec2::new(fill_w, 6.0));
        ui.painter().rect_filled(fill_rect, 3.0, pal.accent);
    }

    ui.add_space(8.0);

    // Stats line: "27.7 MB of 66.0 MB · 5.2 MB/s · 7 s left" (12px, muted, tabular numbers)
    // C) Cancel confirmation: replaces the stats line with "Cancel this download? The partial file will be deleted."
    if state.confirming_cancel {
        ui.label(
            egui::RichText::new("Cancel this download? The partial file will be deleted.")
                .size(12.0)
                .color(pal.danger)
                .strong(),
        );
    } else {
        let stats_text = match state.stage {
            DownloadWindowStage::Connecting => {
                let is_torrent = rustget_core::torrent::is_torrent_url(&state.url);
                if let Some(tot) = state.disp_total.or(state.file_size) {
                    if is_torrent {
                        format!("Connecting to peers… (0.0 B of {})", format_size_1dec(tot))
                    } else {
                        format!("Connecting… (0.0 B of {})", format_size_1dec(tot))
                    }
                } else if is_torrent {
                    "Connecting to peers…".to_string()
                } else {
                    "Connecting…".to_string()
                }
            }
            DownloadWindowStage::Processing => "Processing…".to_string(),
            _ => {
                if let Some(tot) = state.disp_total {
                    let eta_str = state.disp_eta_seconds.map(format_time_left).unwrap_or_else(|| "--".to_string());
                    format!(
                        "{} of {} · {} · {} left",
                        format_size_1dec(state.disp_downloaded),
                        format_size_1dec(tot),
                        format_speed_1dec(state.disp_speed_bps),
                        eta_str
                    )
                } else {
                    format!("Downloaded {}", format_size_1dec(state.disp_downloaded))
                }
            }
        };

        ui.label(
            egui::RichText::new(stats_text)
                .size(12.0)
                .color(pal.muted)
                .monospace(),
        );
    }

    ui.add_space(14.0);

    // "Saving to" label + read-only path field
    ui.label(egui::RichText::new("Saving to").size(12.0).color(pal.muted));
    ui.add_space(4.0);

    let full_target = if !state.target_path.is_empty() {
        state.target_path.clone()
    } else {
        let clean_dir = state.save_dir.trim_end_matches(['\\', '/']);
        format!("{}\\{}", clean_dir, state.filename)
    };

    let mut path_disp = full_target.clone();
    let text_edit = egui::TextEdit::singleline(&mut path_disp)
        .desired_width(ui.available_width())
        .interactive(false)
        .margin(Margin::symmetric(10.0, 7.0));
    ui.add(text_edit).on_hover_text(&full_target);
}

fn render_view_b_paused(
    ui: &mut egui::Ui,
    pal: &ThemeColors,
    state: &DownloadWindowState,
) {
    ui.spacing_mut().item_spacing.y = 0.0;

    // File summary row
    render_file_summary_row(ui, pal, state);

    ui.add_space(14.0);

    // Status line: "Paused · 42%"
    let ratio = if let Some(tot) = state.disp_total {
        if tot > 0 {
            (state.disp_downloaded as f32 / tot as f32).clamp(0.0, 1.0)
        } else {
            0.0
        }
    } else {
        0.0
    };

    let status_text = if state.disp_total.is_some() {
        format!("Paused · {:.0}%", ratio * 100.0)
    } else {
        format!("Paused · {}", format_size_1dec(state.disp_downloaded))
    };

    ui.label(
        egui::RichText::new(status_text)
            .size(16.0)
            .strong()
            .color(pal.text),
    );

    ui.add_space(8.0);

    // Progress bar in muted color (track = raise color)
    let (track_rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 6.0), egui::Sense::hover());
    ui.painter().rect_filled(track_rect, 3.0, pal.raise);
    let fill_w = (track_rect.width() * ratio).max(if ratio > 0.0 { 4.0 } else { 0.0 });
    let fill_rect = Rect::from_min_size(track_rect.min, Vec2::new(fill_w, 6.0));
    ui.painter().rect_filled(fill_rect, 3.0, pal.muted);

    ui.add_space(8.0);

    // Stats line: no speed or time left. If cancel confirmation, show message
    if state.confirming_cancel {
        ui.label(
            egui::RichText::new("Cancel this download? The partial file will be deleted.")
                .size(12.0)
                .color(pal.danger)
                .strong(),
        );
    } else {
        let stats_text = if let Some(tot) = state.disp_total {
            format!("{} of {}", format_size_1dec(state.disp_downloaded), format_size_1dec(tot))
        } else {
            format!("Paused at {}", format_size_1dec(state.disp_downloaded))
        };

        ui.label(
            egui::RichText::new(stats_text)
                .size(12.0)
                .color(pal.muted)
                .monospace(),
        );
    }

    ui.add_space(14.0);

    // "Saving to"
    ui.label(egui::RichText::new("Saving to").size(12.0).color(pal.muted));
    ui.add_space(4.0);

    let full_target = if !state.target_path.is_empty() {
        state.target_path.clone()
    } else {
        let clean_dir = state.save_dir.trim_end_matches(['\\', '/']);
        format!("{}\\{}", clean_dir, state.filename)
    };

    let mut path_disp = full_target.clone();
    let text_edit = egui::TextEdit::singleline(&mut path_disp)
        .desired_width(ui.available_width())
        .interactive(false)
        .margin(Margin::symmetric(10.0, 7.0));
    ui.add(text_edit).on_hover_text(&full_target);
}

fn render_view_c_complete(
    ui: &mut egui::Ui,
    pal: &ThemeColors,
    state: &mut DownloadWindowState,
    _outcome: &mut DownloadWindowOutcome,
) {
    ui.spacing_mut().item_spacing.y = 0.0;

    render_file_summary_row(ui, pal, state);

    ui.add_space(12.0);

    // 36px check icon tile, "Download complete" (16px, weight 600), "Saved 66.0 MB" in muted text
    ui.horizontal(|ui| {
        let (icon_rect, _) = ui.allocate_exact_size(Vec2::splat(36.0), egui::Sense::hover());
        ui.painter().rect_filled(icon_rect, shapes::R_SM, pal.raise);
        let inner_rect = icon_rect.shrink(8.0);
        icons::draw_check(ui.painter(), inner_rect, pal.accent);

        ui.add_space(shapes::S2);
        ui.vertical(|ui| {
            ui.label(
                egui::RichText::new("Download complete")
                    .size(16.0)
                    .strong()
                    .color(pal.text),
            );
            ui.add_space(2.0);
            let saved_size = state.total_bytes
                .or(Some(state.downloaded_bytes))
                .or(state.file_size)
                .map(format_size_1dec)
                .unwrap_or_else(|| "66.0 MB".to_string());
            ui.label(
                egui::RichText::new(format!("Saved {}", saved_size))
                    .size(12.0)
                    .color(pal.muted),
            );
        });
    });

    ui.add_space(14.0);

    // "Saved file" label, "Copy path" small button at right
    let full_target = if !state.target_path.is_empty() {
        state.target_path.clone()
    } else {
        let clean_dir = state.save_dir.trim_end_matches(['\\', '/']);
        format!("{}\\{}", clean_dir, state.filename)
    };

    let mut clicked_copy = false;
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new("Saved file").size(12.0).color(pal.muted));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let is_copied = state.copy_button_copied_at.map(|t| t.elapsed() < Duration::from_millis(1500)).unwrap_or(false);
            let label = if is_copied { "Copied" } else { "Copy path" };
            let copy_btn = egui::Button::new(
                egui::RichText::new(label)
                    .size(11.5)
                    .color(pal.accent),
            )
            .fill(Color32::TRANSPARENT);

            if ui.add(copy_btn).clicked() {
                ui.output_mut(|o| o.copied_text = full_target.clone());
                clicked_copy = true;
            }
        });
    });
    if clicked_copy {
        state.copy_button_copied_at = Some(Instant::now());
    }

    ui.add_space(4.0);

    // Read-only path field below
    let mut path_disp = full_target.clone();
    let text_edit = egui::TextEdit::singleline(&mut path_disp)
        .desired_width(ui.available_width())
        .interactive(false)
        .margin(Margin::symmetric(10.0, 7.0));
    ui.add(text_edit).on_hover_text(&full_target);
}

fn render_view_d_failed(
    ui: &mut egui::Ui,
    pal: &ThemeColors,
    state: &DownloadWindowState,
    reason: &str,
) {
    ui.spacing_mut().item_spacing.y = 0.0;

    render_file_summary_row(ui, pal, state);

    ui.add_space(10.0);

    // 36px error icon tile, "Download failed" (16px, weight 600), short reason and "42% is saved and can be resumed"
    ui.horizontal(|ui| {
        let (icon_rect, _) = ui.allocate_exact_size(Vec2::splat(36.0), egui::Sense::hover());
        ui.painter().rect_filled(icon_rect, shapes::R_SM, pal.raise);
        let center = icon_rect.center();
        ui.painter().text(
            center,
            egui::Align2::CENTER_CENTER,
            "!",
            egui::FontId::proportional(22.0),
            pal.danger,
        );

        ui.add_space(shapes::S2);
        ui.vertical(|ui| {
            ui.label(
                egui::RichText::new("Download failed")
                    .size(16.0)
                    .strong()
                    .color(pal.danger),
            );
            ui.add_space(2.0);
            let pct = if let Some(tot) = state.disp_total {
                if tot > 0 {
                    format!("{:.0}%", (state.disp_downloaded as f32 / tot as f32 * 100.0).clamp(0.0, 100.0))
                } else {
                    format_size_1dec(state.disp_downloaded)
                }
            } else {
                format_size_1dec(state.disp_downloaded)
            };
            let nice_reason = format_failed_reason(reason);
            let subtext = format!("{} · {} is saved and can be resumed", nice_reason, pct);
            ui.label(
                egui::RichText::new(subtext)
                    .size(12.0)
                    .color(pal.muted),
            );
        });
    });

    ui.add_space(10.0);

    // Progress bar in danger color
    let (track_rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 6.0), egui::Sense::hover());
    ui.painter().rect_filled(track_rect, 3.0, pal.raise);
    let ratio = if let Some(tot) = state.disp_total {
        if tot > 0 {
            (state.disp_downloaded as f32 / tot as f32).clamp(0.0, 1.0)
        } else {
            0.0
        }
    } else {
        0.0
    };
    let fill_w = (track_rect.width() * ratio).max(if ratio > 0.0 { 4.0 } else { 0.0 });
    let fill_rect = Rect::from_min_size(track_rect.min, Vec2::new(fill_w, 6.0));
    ui.painter().rect_filled(fill_rect, 3.0, pal.danger);

    ui.add_space(10.0);

    // Target path
    ui.label(egui::RichText::new("Saving to").size(12.0).color(pal.muted));
    ui.add_space(4.0);

    let full_target = if !state.target_path.is_empty() {
        state.target_path.clone()
    } else {
        let clean_dir = state.save_dir.trim_end_matches(['\\', '/']);
        format!("{}\\{}", clean_dir, state.filename)
    };

    let mut path_disp = full_target.clone();
    let text_edit = egui::TextEdit::singleline(&mut path_disp)
        .desired_width(ui.available_width())
        .interactive(false)
        .margin(Margin::symmetric(10.0, 7.0));
    ui.add(text_edit).on_hover_text(&full_target);
}

fn render_window_footer(
    ui: &mut egui::Ui,
    pal: &ThemeColors,
    state: &DownloadWindowState,
    outcome: &mut DownloadWindowOutcome,
) {
    ui.spacing_mut().item_spacing.y = 0.0;

    // 1px top border directly above buttons across the footer width
    let (sep_rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 1.0), egui::Sense::hover());
    let left_x = sep_rect.left() - 24.0;
    let right_x = sep_rect.right() + 24.0;
    ui.painter().line_segment(
        [Pos2::new(left_x, sep_rect.top()), Pos2::new(right_x, sep_rect.top())],
        Stroke::new(1.0_f32, pal.line),
    );

    ui.add_space(12.0); // 12px padding directly above the buttons

    ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
        ui.spacing_mut().item_spacing.x = 0.0;
        ui.spacing_mut().item_spacing.y = 0.0;

        if state.confirming_cancel {
            // Cancel confirmation footer: [Cancel download] [Keep downloading] (primary)
            let keep_btn = primary_button(ui, pal, "Keep downloading", 140.0);
            if keep_btn.clicked() {
                *outcome = DownloadWindowOutcome::DismissCancel;
            }

            ui.add_space(shapes::S2);

            let cancel_btn = outline_button(ui, pal, "Cancel download", 130.0);
            if cancel_btn.clicked() {
                *outcome = DownloadWindowOutcome::ConfirmCancel;
            }

            if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                *outcome = DownloadWindowOutcome::DismissCancel;
            }
            return;
        }

        match &state.stage {
            DownloadWindowStage::Info => {
                let is_empty_link = state.url.trim().is_empty();

                // Start download (primary) 34 height
                let (btn_rect, btn_resp) = ui.allocate_exact_size(
                    Vec2::new(138.0, shapes::CONTROL_HEIGHT),
                    if is_empty_link { egui::Sense::hover() } else { egui::Sense::click() },
                );
                let bg = if is_empty_link {
                    pal.line
                } else if btn_resp.hovered() {
                    pal.accent.gamma_multiply(1.08)
                } else {
                    pal.accent
                };
                let text_color = if is_empty_link { pal.muted } else { pal.on_accent };
                ui.painter().rect_filled(btn_rect, shapes::R_SM, bg);
                let icon_rect = Rect::from_center_size(Pos2::new(btn_rect.left() + 18.0, btn_rect.center().y), Vec2::splat(14.0));
                icons::draw_download(ui.painter(), icon_rect, text_color);
                ui.painter().text(
                    Pos2::new(btn_rect.left() + 32.0, btn_rect.center().y),
                    egui::Align2::LEFT_CENTER,
                    "Start download",
                    egui::FontId::proportional(13.0),
                    text_color,
                );

                if !is_empty_link && btn_resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                    *outcome = DownloadWindowOutcome::StartDownload;
                }

                ui.add_space(shapes::S2); // 8px gap

                // Cancel (outline) 72px wide, 34px height
                let (cancel_rect, cancel_resp) = ui.allocate_exact_size(
                    Vec2::new(72.0, shapes::CONTROL_HEIGHT),
                    egui::Sense::click(),
                );
                let cancel_resp = cancel_resp.on_hover_cursor(egui::CursorIcon::PointingHand);
                let cancel_bg = if cancel_resp.hovered() { pal.raise } else { Color32::TRANSPARENT };
                ui.painter().rect(cancel_rect.shrink(0.5), shapes::R_SM, cancel_bg, Stroke::new(1.0_f32, pal.line));
                ui.painter().text(
                    cancel_rect.center(),
                    egui::Align2::CENTER_CENTER,
                    "Cancel",
                    egui::FontId::proportional(13.0),
                    pal.text,
                );

                if cancel_resp.clicked() {
                    *outcome = DownloadWindowOutcome::Close;
                }

                if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                    *outcome = DownloadWindowOutcome::Close;
                }
                if !is_empty_link && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                    *outcome = DownloadWindowOutcome::StartDownload;
                }
            }
            DownloadWindowStage::Connecting | DownloadWindowStage::Downloading | DownloadWindowStage::Processing => {
                // View A: [Run in background] [Pause] [Cancel], all outline buttons
                // Right-to-left layout: Cancel first, then Pause, then Run in background

                // Cancel
                let cancel_btn = outline_button(ui, pal, "Cancel", 72.0);
                if cancel_btn.clicked() {
                    *outcome = DownloadWindowOutcome::CancelClicked;
                }

                ui.add_space(shapes::S2);

                // Pause
                let pause_btn = outline_button(ui, pal, "Pause", 72.0);
                if pause_btn.clicked() {
                    *outcome = DownloadWindowOutcome::Pause;
                }

                ui.add_space(shapes::S2);

                // Run in background
                let bg_btn = outline_button(ui, pal, "Run in background", 140.0);
                if bg_btn.clicked() {
                    *outcome = DownloadWindowOutcome::RunInBackground;
                }
            }
            DownloadWindowStage::Paused => {
                // View B: [Run in background] [Resume] (primary) [Cancel]

                // Cancel
                let cancel_btn = outline_button(ui, pal, "Cancel", 72.0);
                if cancel_btn.clicked() {
                    *outcome = DownloadWindowOutcome::CancelClicked;
                }

                ui.add_space(shapes::S2);

                // Resume (primary)
                let resume_btn = primary_button(ui, pal, "Resume", 80.0);
                if resume_btn.clicked() {
                    *outcome = DownloadWindowOutcome::Resume;
                }

                ui.add_space(shapes::S2);

                // Run in background
                let bg_btn = outline_button(ui, pal, "Run in background", 140.0);
                if bg_btn.clicked() {
                    *outcome = DownloadWindowOutcome::RunInBackground;
                }
            }
            DownloadWindowStage::Complete => {
                // View C: [Open file] (primary) [Show in folder] [Close]

                // Close (outline)
                let close_btn = outline_button(ui, pal, "Close", 72.0);
                if close_btn.clicked() || ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                    *outcome = DownloadWindowOutcome::Close;
                }

                ui.add_space(shapes::S2);

                // Show in folder (outline)
                let folder_btn = outline_button(ui, pal, "Show in folder", 125.0);
                if folder_btn.clicked() {
                    *outcome = DownloadWindowOutcome::ShowInFolder;
                }

                ui.add_space(shapes::S2);

                // Open file (primary)
                let open_btn = primary_button(ui, pal, "Open file", 96.0);
                if open_btn.clicked() {
                    *outcome = DownloadWindowOutcome::OpenFile;
                }
            }
            DownloadWindowStage::Failed { .. } => {
                // View D: [Retry] (primary) [Close]

                // Close (outline)
                let close_btn = outline_button(ui, pal, "Close", 72.0);
                if close_btn.clicked() || ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                    *outcome = DownloadWindowOutcome::Close;
                }

                ui.add_space(shapes::S2);

                // Retry (primary)
                let retry_btn = primary_button(ui, pal, "Retry", 80.0);
                if retry_btn.clicked() {
                    *outcome = DownloadWindowOutcome::Retry;
                }
            }
        }
    });

    ui.add_space(12.0); // 12px padding below buttons
}

fn outline_button(ui: &mut egui::Ui, pal: &ThemeColors, text: &str, width: f32) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(
        Vec2::new(width, shapes::CONTROL_HEIGHT),
        egui::Sense::click(),
    );
    let resp = resp.on_hover_cursor(egui::CursorIcon::PointingHand);
    let bg = if resp.hovered() { pal.raise } else { Color32::TRANSPARENT };
    ui.painter().rect(rect.shrink(0.5), shapes::R_SM, bg, Stroke::new(1.0_f32, pal.line));
    ui.painter().text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        text,
        egui::FontId::proportional(13.0),
        pal.text,
    );
    resp
}

fn primary_button(ui: &mut egui::Ui, pal: &ThemeColors, text: &str, width: f32) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(
        Vec2::new(width, shapes::CONTROL_HEIGHT),
        egui::Sense::click(),
    );
    let resp = resp.on_hover_cursor(egui::CursorIcon::PointingHand);
    let bg = if resp.hovered() { pal.accent.gamma_multiply(1.08) } else { pal.accent };
    ui.painter().rect_filled(rect, shapes::R_SM, bg);
    ui.painter().text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        text,
        egui::FontId::proportional(13.0),
        pal.on_accent,
    );
    resp
}
