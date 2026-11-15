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
    /// View A: Downloading
    Downloading,
    /// View B: Paused
    Paused,
    /// View C: Complete
    Complete,
    /// View D: Failed
    Failed { reason: String },
}

pub struct DownloadWindowState {
    pub dialog_id: String,
    pub info_status: InfoLookupStatus,
    pub info_rx: Option<tokio::sync::mpsc::Receiver<Result<rustget_core::youtube::StreamingVideoInfo, String>>>,
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
    pub copy_button_copied_at: Option<Instant>,
    pub confirming_cancel: bool,
    pub cascade_index: usize,
    pub host: String,
    pub window_pos: Option<Pos2>,
}

impl DownloadWindowState {
    pub fn new(url: String, filename: String, category: String, save_dir: String) -> Self {
        let host = extract_host(&url);
        let now = Instant::now();
        let dialog_id = uuid::Uuid::new_v4().to_string();

        let is_stream = rustget_core::youtube::is_streaming_site(&url);
        let (info_status, info_rx) = if is_stream && !url.trim().is_empty() {
            crate::platform::log_daemon(&format!(
                "STEP: video info lookup started for url='{}' on dialog_id='{}'",
                url, dialog_id
            ));
            let (tx, rx) = tokio::sync::mpsc::channel(1);
            let lookup_url = url.clone();
            std::thread::spawn(move || {
                let rt = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build();
                if let Ok(rt) = rt {
                    let res = rt.block_on(rustget_core::youtube::query_streaming_video_info(&lookup_url));
                    let _ = tx.blocking_send(res);
                }
            });
            (InfoLookupStatus::ReadingInfo, Some(rx))
        } else {
            (InfoLookupStatus::Success, None)
        };

        Self {
            dialog_id,
            info_status,
            info_rx,
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
            file_size: None,
            is_probing: false,
            error_message: None,
            show_notes_expanded: false,
            show_advanced_expanded: false,

            downloaded_bytes: 0,
            total_bytes: None,
            speed_bps: 0.0,
            eta_seconds: None,
            speed_samples: VecDeque::new(),

            last_ui_update: now - Duration::from_millis(300),
            disp_downloaded: 0,
            disp_total: None,
            disp_speed_bps: 0.0,
            disp_eta_seconds: None,

            is_open: true,
            has_painted: false,
            copy_button_copied_at: None,
            confirming_cancel: false,
            cascade_index: 0,
            host,
            window_pos: None,
        }
    }

    pub fn poll_info_rx(&mut self) {
        if let Some(ref mut rx) = self.info_rx {
            if let Ok(res) = rx.try_recv() {
                match res {
                    Ok(info) => {
                        crate::platform::log_daemon(&format!(
                            "STEP: video info lookup finished for url='{}': title='{}', size={:?}",
                            self.url, info.title, info.filesize_approx
                        ));
                        self.filename = info.filename;
                        if let Some(sz) = info.filesize_approx {
                            self.file_size = Some(sz);
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
            }
        }
    }

    pub fn retry_info_lookup(&mut self) {
        if rustget_core::youtube::is_streaming_site(&self.url) && !self.url.trim().is_empty() {
            crate::platform::log_daemon(&format!(
                "STEP: video info lookup retried for url='{}' on dialog_id='{}'",
                self.url, self.dialog_id
            ));
            self.info_status = InfoLookupStatus::ReadingInfo;
            let (tx, rx) = tokio::sync::mpsc::channel(1);
            self.info_rx = Some(rx);
            let lookup_url = self.url.clone();
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

    pub fn use_link_as_is(&mut self) {
        crate::platform::log_daemon(&format!(
            "STEP: video info lookup skipped, using link as is for url='{}'",
            self.url
        ));
        self.info_status = InfoLookupStatus::Success;
        self.info_rx = None;
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

/// Renders the complete IDM-style download window content (Title Bar + Central Panel + Footer).
pub fn render_download_window_content(
    ctx: &egui::Context,
    pal: &ThemeColors,
    logo_texture: Option<&egui::TextureHandle>,
    state: &mut DownloadWindowState,
) -> DownloadWindowOutcome {
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

    if title_act.close {
        match state.stage {
            DownloadWindowStage::Info => {
                outcome = DownloadWindowOutcome::Close;
            }
            DownloadWindowStage::Downloading | DownloadWindowStage::Paused => {
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

    // Request repaint for active downloads
    if state.stage == DownloadWindowStage::Downloading {
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

    // 1. Title bar
    let title_rect = Rect::from_min_size(pos, Vec2::new(DOWNLOAD_WINDOW_WIDTH, 36.0));
    ui.allocate_new_ui(egui::UiBuilder::new().max_rect(title_rect), |ui| {
        let t_act = title_bar::render_title_bar(ui, ctx, pal, logo_texture, "Download File Info");
        if t_act.close {
            outcome = match state.stage {
                DownloadWindowStage::Info => DownloadWindowOutcome::Close,
                DownloadWindowStage::Downloading | DownloadWindowStage::Paused => {
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
                if t_act.close {
                    outcome = match state.stage {
                        DownloadWindowStage::Info => DownloadWindowOutcome::Close,
                        DownloadWindowStage::Downloading | DownloadWindowStage::Paused => {
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
    if state.confirming_cancel {
        render_cancel_confirmation(ui, pal, outcome);
        return;
    }

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
        DownloadWindowStage::Downloading => {
            render_view_a_downloading(ui, ctx, pal, state);
        }
        DownloadWindowStage::Paused => {
            render_view_b_paused(ui, pal, state);
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

    // File summary row: icon tile, file name (ellipsis), "Category · host" in muted text
    ui.horizontal(|ui| {
        let (icon_rect, _) = ui.allocate_exact_size(Vec2::splat(36.0), egui::Sense::hover());
        ui.painter().rect_filled(icon_rect, shapes::R_SM, pal.raise);
        let inner_icon_rect = icon_rect.shrink(9.0);
        icons::draw_download(ui.painter(), inner_icon_rect, pal.accent);

        ui.add_space(shapes::S2);
        ui.vertical(|ui| {
            let fname = if !state.filename.trim().is_empty() {
                state.filename.trim()
            } else {
                "Downloading file..."
            };
            ui.add(
                egui::Label::new(
                    egui::RichText::new(fname)
                        .size(14.0)
                        .strong()
                        .color(pal.text),
                )
                .truncate(),
            );
            ui.add_space(2.0);
            ui.label(
                egui::RichText::new(format!("{} · {}", state.category, state.host))
                    .size(12.0)
                    .color(pal.muted),
            );
        });
    });

    ui.add_space(14.0);

    // Status line: "Downloading… 42%" (14px, weight 500)
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
        format!("Downloading… {:.0}%", ratio * 100.0)
    } else {
        format!("Downloading… {}", format_size_1dec(state.disp_downloaded))
    };

    ui.label(
        egui::RichText::new(status_text)
            .size(14.0)
            .strong()
            .color(pal.text),
    );

    ui.add_space(8.0);

    // Progress bar: 6px high, rounded, accent fill, track = line color
    let (track_rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 6.0), egui::Sense::hover());
    ui.painter().rect_filled(track_rect, 3.0, pal.line);

    if state.disp_total.is_some() {
        let fill_w = (track_rect.width() * ratio).max(if ratio > 0.0 { 4.0 } else { 0.0 });
        let fill_rect = Rect::from_min_size(track_rect.min, Vec2::new(fill_w, 6.0));
        ui.painter().rect_filled(fill_rect, 3.0, pal.accent);
    } else {
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
    }

    ui.add_space(8.0);

    // Stats line: "119.4 MB of 284.3 MB · 8.2 MB/s · 20 s left"
    let stats_text = if let Some(tot) = state.disp_total {
        let eta_str = state.disp_eta_seconds.map(format_time_left).unwrap_or_else(|| "--".to_string());
        format!(
            "{} of {} · {} · {} left",
            format_size_1dec(state.disp_downloaded),
            format_size_1dec(tot),
            format_speed_1dec(state.disp_speed_bps),
            eta_str
        )
    } else {
        format!("Downloading… {}", format_size_1dec(state.disp_downloaded))
    };

    ui.label(
        egui::RichText::new(stats_text)
            .size(12.0)
            .color(pal.muted)
            .monospace(),
    );

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
    ui.horizontal(|ui| {
        let (icon_rect, _) = ui.allocate_exact_size(Vec2::splat(36.0), egui::Sense::hover());
        ui.painter().rect_filled(icon_rect, shapes::R_SM, pal.raise);
        let inner_icon_rect = icon_rect.shrink(9.0);
        icons::draw_download(ui.painter(), inner_icon_rect, pal.muted);

        ui.add_space(shapes::S2);
        ui.vertical(|ui| {
            let fname = if !state.filename.trim().is_empty() {
                state.filename.trim()
            } else {
                "Downloading file..."
            };
            ui.add(
                egui::Label::new(
                    egui::RichText::new(fname)
                        .size(14.0)
                        .strong()
                        .color(pal.text),
                )
                .truncate(),
            );
            ui.add_space(2.0);
            ui.label(
                egui::RichText::new(format!("{} · {}", state.category, state.host))
                    .size(12.0)
                    .color(pal.muted),
            );
        });
    });

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
            .size(14.0)
            .strong()
            .color(pal.text),
    );

    ui.add_space(8.0);

    // Progress bar in muted color
    let (track_rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 6.0), egui::Sense::hover());
    ui.painter().rect_filled(track_rect, 3.0, pal.line);
    let fill_w = (track_rect.width() * ratio).max(if ratio > 0.0 { 4.0 } else { 0.0 });
    let fill_rect = Rect::from_min_size(track_rect.min, Vec2::new(fill_w, 6.0));
    ui.painter().rect_filled(fill_rect, 3.0, pal.muted);

    ui.add_space(8.0);

    // Stats line: no speed or time left
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

    // 36px check icon tile, "Download complete" (16px, weight 600), "Saved 284.34 MB" in muted text
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
                .map(format_size_1dec)
                .unwrap_or_else(|| "Saved".to_string());
            ui.label(
                egui::RichText::new(format!("Saved {}", saved_size))
                    .size(12.0)
                    .color(pal.muted),
            );
        });
    });

    ui.add_space(14.0);

    // Full bar in accent color
    let (track_rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 6.0), egui::Sense::hover());
    ui.painter().rect_filled(track_rect, 3.0, pal.accent);

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

    // 36px error icon tile, "Download failed" (16px, weight 600), short reason in muted text
    ui.horizontal(|ui| {
        let (icon_rect, _) = ui.allocate_exact_size(Vec2::splat(36.0), egui::Sense::hover());
        ui.painter().rect_filled(icon_rect, shapes::R_SM, pal.raise);
        // Draw cross icon
        let center = icon_rect.center();
        let r = 8.0;
        let stroke = Stroke::new(2.0_f32, pal.danger);
        ui.painter().line_segment([center + Vec2::new(-r, -r), center + Vec2::new(r, r)], stroke);
        ui.painter().line_segment([center + Vec2::new(r, -r), center + Vec2::new(-r, r)], stroke);

        ui.add_space(shapes::S2);
        ui.vertical(|ui| {
            ui.label(
                egui::RichText::new("Download failed")
                    .size(16.0)
                    .strong()
                    .color(pal.danger),
            );
            ui.add_space(2.0);
            ui.label(
                egui::RichText::new(reason)
                    .size(12.0)
                    .color(pal.muted),
            );
        });
    });

    ui.add_space(14.0);

    // Progress bar in danger color
    let (track_rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 6.0), egui::Sense::hover());
    ui.painter().rect_filled(track_rect, 3.0, pal.danger);

    ui.add_space(14.0);

    // Target path
    ui.label(egui::RichText::new("File").size(12.0).color(pal.muted));
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

fn render_cancel_confirmation(
    ui: &mut egui::Ui,
    pal: &ThemeColors,
    outcome: &mut DownloadWindowOutcome,
) {
    ui.vertical_centered(|ui| {
        ui.add_space(30.0);
        let (icon_rect, _) = ui.allocate_exact_size(Vec2::splat(40.0), egui::Sense::hover());
        ui.painter().rect_filled(icon_rect, shapes::R_SM, pal.raise);
        let center = icon_rect.center();
        let r = 9.0;
        let stroke = Stroke::new(2.0_f32, pal.danger);
        ui.painter().line_segment([center + Vec2::new(-r, -r), center + Vec2::new(r, r)], stroke);
        ui.painter().line_segment([center + Vec2::new(r, -r), center + Vec2::new(-r, r)], stroke);

        ui.add_space(16.0);
        ui.label(
            egui::RichText::new("Cancel this download?")
                .size(16.0)
                .strong()
                .color(pal.text),
        );
        ui.add_space(6.0);
        ui.label(
            egui::RichText::new("The partial file will be deleted.")
                .size(13.0)
                .color(pal.muted),
        );

        ui.add_space(24.0);

        ui.horizontal(|ui| {
            let keep_btn = egui::Button::new(
                egui::RichText::new("Keep downloading").size(13.0).color(pal.text),
            )
            .fill(Color32::TRANSPARENT)
            .stroke(Stroke::new(1.0_f32, pal.line))
            .min_size(Vec2::new(140.0, shapes::CONTROL_HEIGHT))
            .rounding(shapes::R_SM);

            if ui.add(keep_btn).clicked() {
                *outcome = DownloadWindowOutcome::DismissCancel;
            }

            ui.add_space(12.0);

            let cancel_btn = egui::Button::new(
                egui::RichText::new("Cancel download").size(13.0).color(pal.on_accent),
            )
            .fill(pal.danger)
            .min_size(Vec2::new(130.0, shapes::CONTROL_HEIGHT))
            .rounding(shapes::R_SM);

            if ui.add(cancel_btn).clicked() {
                *outcome = DownloadWindowOutcome::ConfirmCancel;
            }
        });
    });
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
            DownloadWindowStage::Downloading => {
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
                if close_btn.clicked() {
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
                if close_btn.clicked() {
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
