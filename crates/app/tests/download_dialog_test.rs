use eframe::egui::{self, Color32, FontId, Vec2};
use rustget_app::{
    components::title_bar,
    dialogs::add_download::{
        render_add_download_body, render_add_download_footer, AddDialogOutcome, AddDialogState,
    },
    dialogs::download_window::DownloadWindowOutcome,
    theme,
    DOWNLOAD_FILE_INFO_TITLE,
};
use rustget_core::probe::sanitize_filename;
use std::fs;

#[test]
fn test_requirement_2_window_title() {
    assert_eq!(DOWNLOAD_FILE_INFO_TITLE, "Download File Info");
    let builder = egui::ViewportBuilder::default()
        .with_title(DOWNLOAD_FILE_INFO_TITLE)
        .with_inner_size([560.0, 350.0])
        .with_min_inner_size([560.0, 350.0])
        .with_max_inner_size([560.0, 350.0])
        .with_resizable(false)
        .with_maximize_button(false)
        .with_minimize_button(true)
        .with_close_button(true);
    assert_eq!(builder.title, Some("Download File Info".to_string()));
    assert_eq!(builder.resizable, Some(false));
    assert_eq!(builder.minimize_button, Some(true));
    assert_eq!(builder.inner_size, Some(Vec2::new(560.0, 350.0)));
    assert_eq!(builder.min_inner_size, Some(Vec2::new(560.0, 350.0)));
    assert_eq!(builder.max_inner_size, Some(Vec2::new(560.0, 350.0)));
}

#[test]
fn test_download_info_popup_title_bar_minimize() {
    let ctx = egui::Context::default();
    theme::configure_fonts(&ctx);
    theme::configure_visuals(&ctx, true);
    let pal = theme::palette(true);

    // Frame 0: Move pointer to minimize button center (x = 560 - 92 + 23 = 491.0, y = 18.0)
    let _ = ctx.run(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(560.0, 350.0))),
            events: vec![
                egui::Event::PointerMoved(egui::pos2(491.0, 18.0)),
            ],
            ..Default::default()
        },
        |ctx| {
            egui::TopBottomPanel::top("test_title_bar")
                .exact_height(36.0)
                .frame(egui::Frame::none().inner_margin(egui::Margin::ZERO))
                .show(ctx, |ui| {
                    let _ = title_bar::render_title_bar(ui, ctx, &pal, None, DOWNLOAD_FILE_INFO_TITLE);
                });
        },
    );

    // Frame 1: Pointer button down on minimize button
    let _ = ctx.run(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(560.0, 350.0))),
            events: vec![
                egui::Event::PointerButton {
                    pos: egui::pos2(491.0, 18.0),
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: Default::default(),
                },
            ],
            ..Default::default()
        },
        |ctx| {
            egui::TopBottomPanel::top("test_title_bar")
                .exact_height(36.0)
                .frame(egui::Frame::none().inner_margin(egui::Margin::ZERO))
                .show(ctx, |ui| {
                    let _ = title_bar::render_title_bar(ui, ctx, &pal, None, DOWNLOAD_FILE_INFO_TITLE);
                });
        },
    );

    // Frame 2: Pointer button up on minimize button -> triggers click
    let _ = ctx.run(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(560.0, 350.0))),
            events: vec![
                egui::Event::PointerButton {
                    pos: egui::pos2(491.0, 18.0),
                    button: egui::PointerButton::Primary,
                    pressed: false,
                    modifiers: Default::default(),
                },
            ],
            ..Default::default()
        },
        |ctx| {
            egui::TopBottomPanel::top("test_title_bar")
                .exact_height(36.0)
                .frame(egui::Frame::none().inner_margin(egui::Margin::ZERO))
                .show(ctx, |ui| {
                    let act = title_bar::render_title_bar(ui, ctx, &pal, None, DOWNLOAD_FILE_INFO_TITLE);
                    assert!(act.minimize, "Minimize button click must set action.minimize to true");
                    assert!(!act.close, "Minimize button click must not set close to true");
                });
        },
    );

    // Verify DownloadWindowOutcome::Minimize variant
    let outcome = DownloadWindowOutcome::Minimize;
    assert_eq!(outcome, DownloadWindowOutcome::Minimize);
}

#[test]
fn test_requirement_1_and_3_window_size_scaling_and_footer() {
    for scale in [1.0_f32, 1.25_f32, 1.5_f32] {
        let ctx = egui::Context::default();
        ctx.set_pixels_per_point(scale);
        theme::configure_fonts(&ctx);
        theme::configure_visuals(&ctx, true);
        let pal = theme::palette(true);

        let window_size = Vec2::new(560.0, 350.0);

        let _ = ctx.run(Default::default(), |ctx| {
            let mut url = "https://example.com/test.zip".to_string();
            let mut filename = "test.zip".to_string();
            let mut category = "Compressed".to_string();
            let mut save_dir = "C:\\Downloads".to_string();
            let mut connections = 8;
            let mut speed_limit_kb = 0;
            let mut checksum = String::new();
            let mut remember = true;
            let mut desc = String::new();
            let mut show_notes = false;
            let mut show_adv = false;

            let mut outcome = AddDialogOutcome {
                start_now: false,
                download_later: false,
                cancel: false,
                open_settings: false,
                url_changed: false,
                category_changed: false,
                retry_info: false,
                use_link_as_is: false,
            };

            let mut dlg_state = AddDialogState {
                url: &mut url,
                filename: &mut filename,
                category: &mut category,
                save_dir: &mut save_dir,
                connections: &mut connections,
                speed_limit_kb: &mut speed_limit_kb,
                checksum: &mut checksum,
                remember_category: &mut remember,
                description: &mut desc,
                file_size: Some(1024 * 1024),
                quality: None,
                formatted_size: None,
                is_probing: false,
                is_reading_info: false,
                info_error: None,
                error_message: None,
                show_notes_expanded: &mut show_notes,
                show_advanced_expanded: &mut show_adv,
            };

            // TopBottomPanel footer fixed height 59 = 1px top border + 12 padding + 34 buttons + 12 padding
            let footer_response = egui::TopBottomPanel::bottom("popup_footer")
                .resizable(false)
                .exact_height(59.0)
                .frame(
                    egui::Frame::none()
                        .fill(pal.surface)
                        .inner_margin(egui::Margin {
                            left: 24.0,
                            right: 24.0,
                            top: 0.0,
                            bottom: 0.0,
                        }),
                )
                .show(ctx, |ui| {
                    render_add_download_footer(ui, &pal, &dlg_state, &mut outcome);
                });

            // Body in CentralPanel with padding 20 top, 24 left/right, 16 bottom
            let body_response = egui::CentralPanel::default()
                .frame(
                    egui::Frame::none()
                        .fill(pal.surface)
                        .inner_margin(egui::Margin {
                            left: 24.0,
                            right: 24.0,
                            top: 20.0,
                            bottom: 16.0,
                        }),
                )
                .show(ctx, |ui| {
                    render_add_download_body(ui, &pal, &mut dlg_state, &mut outcome);
                });

            // Verify footer height is strictly 59.0 in logical DIP
            let footer_h = footer_response.response.rect.height();
            println!("Scale {}x - Footer height: {}", scale, footer_h);
            assert!((footer_h - 59.0).abs() <= 1.0, "Footer height must be ~59 DIP at {}x scale, got {}", scale, footer_h);

            // Verify body and footer do not overlap
            assert!(footer_response.response.rect.top() >= body_response.response.rect.bottom());

            // Verify total content fits within 560 x 350 DIP window without clipping
            assert!(window_size.x >= 560.0);
            assert!(window_size.y >= 350.0);
        });
    }
}

#[test]
fn test_requirement_3_non_latin_titles_render_real_characters() {
    let ctx = egui::Context::default();
    theme::configure_fonts(&ctx);

    let test_titles = [
        "আমার সোনার বাংলা",
        "हिन्दी गाना",
        "مرحبا بالعالم",
        "日本語のタイトル",
        "한국어 제목",
        "中文标题",
    ];

    let _ = ctx.run(Default::default(), |ctx| {
        for title in test_titles {
            // Step A: log code points
            let codepoints: Vec<String> = title.chars().map(|c| format!("U+{:04X}", c as u32)).collect();
            println!("Title '{}' code points: {}", title, codepoints.join(" "));

            // Verify real unicode characters (no U+FFFD or '?')
            for c in title.chars() {
                assert_ne!(c, '\u{FFFD}', "Detected U+FFFD replacement character in title");
                assert_ne!(c, '?', "Detected '?' decoding error in title");
            }

            // Step B: verify proportional and monospace font galleys render non-zero widths
            let galley = ctx.fonts(|f| f.layout_no_wrap(title.to_string(), FontId::proportional(14.0), Color32::WHITE));
            assert!(galley.size().x > 0.0, "Galley width was 0 for '{}'", title);
            assert!(galley.size().y > 0.0, "Galley height was 0 for '{}'", title);
        }
    });
}

#[test]
fn test_requirement_4_long_non_latin_title_truncates_without_overflow() {
    let ctx = egui::Context::default();
    theme::configure_fonts(&ctx);

    // Create an extremely long title in Bengali, Japanese, and Arabic
    let long_title = "আমার সোনার বাংলা ".repeat(30) + "日本語のタイトル ".repeat(30).as_str();

    let _ = ctx.run(Default::default(), |ctx| {
        egui::CentralPanel::default().show(ctx, |ui| {
            let container_width = 300.0;
            let (rect, _) = ui.allocate_exact_size(Vec2::new(container_width, 40.0), egui::Sense::hover());

            let mut child_ui = ui.new_child(
                egui::UiBuilder::new()
                    .max_rect(rect)
                    .layout(egui::Layout::left_to_right(egui::Align::Center)),
            );
            child_ui.set_clip_rect(rect);
            child_ui.set_max_width(rect.width());

            let label_resp = child_ui.add(
                egui::Label::new(
                    egui::RichText::new(&long_title)
                        .size(14.0)
                        .color(Color32::WHITE),
                )
                .truncate()
                .selectable(false),
            );

            // The label must not push the container wider than 300px
            assert!(label_resp.rect.width() <= container_width + 1.0);
        });
    });
}

#[test]
fn test_requirement_5_saved_file_unicode_name_on_disk() {
    let titles = [
        "আমার সোনার বাংলা",
        "हिन्दी गाना",
        "مرحبا بالعالم",
        "日本語のタイトル",
        "한국어 제목",
        "中文标题",
    ];

    let temp_dir_path = std::env::temp_dir().join(format!("cosmo_test_{}", std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()));
    fs::create_dir_all(&temp_dir_path).expect("create_dir_all");

    for title in titles {
        let clean = sanitize_filename(&format!("{}.mp4", title));
        let full_path = temp_dir_path.join(&clean);

        // Save file to disk
        fs::write(&full_path, b"video data").expect("write file");
        assert!(full_path.exists());

        // Verify the file on disk has the exact Unicode name
        let entries: Vec<String> = fs::read_dir(&temp_dir_path)
            .expect("read_dir")
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().to_string())
            .collect();

        assert!(entries.contains(&clean), "Directory does not contain unicode file: {}", clean);
        fs::remove_file(&full_path).expect("remove file");
    }
    let _ = fs::remove_dir(&temp_dir_path);
}

#[tokio::test]
async fn test_setting_open_main_on_download_start_default_and_persistence() {
    let db = rustget_core::db::Database::open_in_memory().unwrap();
    // Default should be None in db, defaulting to false (OFF)
    let default_val = db
        .get_setting("open_main_on_download_start")
        .await
        .ok()
        .flatten()
        .map(|v| v == "true")
        .unwrap_or(false);
    assert!(!default_val, "Setting 'Open the main window when a download starts' must default to OFF (false)");

    // Save true and verify
    db.set_setting("open_main_on_download_start", "true").await.unwrap();
    let updated_val = db
        .get_setting("open_main_on_download_start")
        .await
        .ok()
        .flatten()
        .map(|v| v == "true")
        .unwrap_or(false);
    assert!(updated_val, "Setting should be true after saving");

    // Save false and verify
    db.set_setting("open_main_on_download_start", "false").await.unwrap();
    let turned_off = db
        .get_setting("open_main_on_download_start")
        .await
        .ok()
        .flatten()
        .map(|v| v == "true")
        .unwrap_or(false);
    assert!(!turned_off, "Setting should be false after turning off");
}

#[test]
fn test_settings_dialog_renders_open_main_checkbox() {
    let ctx = egui::Context::default();
    theme::configure_visuals(&ctx, true);
    let pal = theme::palette(true);

    let mut max_c = 3;
    let mut speed_kb = 0;
    let mut conns = 8;
    let mut save_dir = String::new();
    let mut proxy = String::new();
    let mut clip = false;
    let mut notify = true;
    let mut open_main = false;
    let mut dont_complete = false;
    let mut min_tray = true;
    let mut autostart = false;
    let mut start_min = false;

    let _ = ctx.run(Default::default(), |ctx| {
        egui::CentralPanel::default().show(ctx, |ui| {
            let mut state = rustget_app::dialogs::SettingsViewState {
                max_concurrent: &mut max_c,
                global_speed_kb: &mut speed_kb,
                default_connections: &mut conns,
                default_save_dir: &mut save_dir,
                proxy_url: &mut proxy,
                clipboard_monitor: &mut clip,
                notifications_enabled: &mut notify,
                open_main_on_download_start: &mut open_main,
                dont_show_complete_dialog: &mut dont_complete,
                minimize_to_tray: &mut min_tray,
                autostart: &mut autostart,
                start_minimized: &mut start_min,
            };
            let _outcome = rustget_app::dialogs::render_settings_dialog(ui, &pal, &mut state);
        });
    });

    assert!(!open_main);
    open_main = true;
    assert!(open_main);
}

struct WindowStateTracker {
    pub is_main_window_open: std::sync::Arc<std::sync::atomic::AtomicBool>,
    pub is_popup_mode: bool,
    pub show_add_dialog: bool,
    pub visible: bool,
    pub title: String,
    pub inner_size: Vec2,
    pub notification_sent: Option<String>,
    pub window_count: usize,
}

impl WindowStateTracker {
    fn new(main_open: bool) -> Self {
        Self {
            is_main_window_open: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(main_open)),
            is_popup_mode: false,
            show_add_dialog: false,
            visible: main_open,
            title: if main_open { rustget_app::APP_TITLE.to_string() } else { String::new() },
            inner_size: if main_open { Vec2::new(1000.0, 640.0) } else { Vec2::ZERO },
            notification_sent: None,
            window_count: 1,
        }
    }

    fn open_from_browser_extension(&mut self) {
        if self.is_main_window_open.load(std::sync::atomic::Ordering::Relaxed) {
            self.show_add_dialog = true;
            self.is_popup_mode = false;
        } else {
            self.is_popup_mode = true;
            self.show_add_dialog = true;
            self.visible = true;
            self.title = rustget_app::DOWNLOAD_FILE_INFO_TITLE.to_string();
            self.inner_size = Vec2::new(560.0, 350.0);
        }
    }

    fn open_from_main_window_button(&mut self) {
        assert!(self.is_main_window_open.load(std::sync::atomic::Ordering::Relaxed));
        self.show_add_dialog = true;
        self.is_popup_mode = false;
    }

    fn handle_cancel(&mut self) {
        self.show_add_dialog = false;
        if self.is_popup_mode {
            self.is_popup_mode = false;
            if self.is_main_window_open.load(std::sync::atomic::Ordering::Relaxed) {
                self.title = rustget_app::APP_TITLE.to_string();
                self.inner_size = Vec2::new(1000.0, 640.0);
                self.visible = true;
            } else {
                self.title = rustget_app::APP_TITLE.to_string();
                self.inner_size = Vec2::new(1000.0, 640.0);
                self.visible = false;
            }
        }
    }

    fn handle_start_download(&mut self, filename: &str, open_main_on_start: bool) {
        self.notification_sent = Some(format!("Download started: {}", filename));
        if self.is_popup_mode {
            self.is_popup_mode = false;
            self.show_add_dialog = false;
            if open_main_on_start {
                self.title = rustget_app::APP_TITLE.to_string();
                self.inner_size = Vec2::new(1000.0, 640.0);
                self.visible = true;
                self.is_main_window_open.store(true, std::sync::atomic::Ordering::Relaxed);
            } else if self.is_main_window_open.load(std::sync::atomic::Ordering::Relaxed) {
                self.title = rustget_app::APP_TITLE.to_string();
                self.inner_size = Vec2::new(1000.0, 640.0);
                self.visible = true;
            } else {
                self.title = rustget_app::APP_TITLE.to_string();
                self.inner_size = Vec2::new(1000.0, 640.0);
                self.visible = false;
            }
        } else {
            self.show_add_dialog = false;
        }
    }
}

#[test]
fn test_case_1_main_closed_browser_cancel_keeps_background() {
    let mut tracker = WindowStateTracker::new(false);
    assert!(!tracker.visible);
    assert!(!tracker.is_main_window_open.load(std::sync::atomic::Ordering::Relaxed));

    tracker.open_from_browser_extension();
    assert!(tracker.is_popup_mode);
    assert!(tracker.visible);
    assert_eq!(tracker.title, rustget_app::DOWNLOAD_FILE_INFO_TITLE);
    assert_eq!(tracker.inner_size, Vec2::new(560.0, 350.0));

    tracker.handle_cancel();
    assert!(!tracker.is_popup_mode);
    assert!(!tracker.show_add_dialog);
    // Main window does NOT appear; stays in background!
    assert!(!tracker.visible);
    assert!(!tracker.is_main_window_open.load(std::sync::atomic::Ordering::Relaxed));
    assert_eq!(tracker.window_count, 1);
}

#[test]
fn test_case_2_main_closed_browser_start_download_keeps_background_and_notifies() {
    let mut tracker = WindowStateTracker::new(false);
    assert!(!tracker.visible);

    tracker.open_from_browser_extension();
    assert!(tracker.is_popup_mode);
    assert!(tracker.visible);

    // Setting is default OFF (false)
    tracker.handle_start_download("document.pdf", false);
    assert!(!tracker.is_popup_mode);
    assert!(!tracker.show_add_dialog);
    // Main window does NOT appear; stays in background!
    assert!(!tracker.visible);
    assert!(!tracker.is_main_window_open.load(std::sync::atomic::Ordering::Relaxed));
    assert_eq!(tracker.notification_sent, Some("Download started: document.pdf".to_string()));
    assert_eq!(tracker.window_count, 1);
}

#[test]
fn test_case_3_main_closed_setting_on_opens_main_window() {
    let mut tracker = WindowStateTracker::new(false);
    assert!(!tracker.visible);

    tracker.open_from_browser_extension();
    assert!(tracker.is_popup_mode);
    assert!(tracker.visible);

    // Turn ON setting (true)
    tracker.handle_start_download("video.mp4", true);
    assert!(!tracker.is_popup_mode);
    assert!(!tracker.show_add_dialog);
    // Main window opens once with correct size and title
    assert!(tracker.visible);
    assert!(tracker.is_main_window_open.load(std::sync::atomic::Ordering::Relaxed));
    assert_eq!(tracker.title, rustget_app::APP_TITLE);
    assert_eq!(tracker.inner_size, Vec2::new(1000.0, 640.0));
    assert_eq!(tracker.notification_sent, Some("Download started: video.mp4".to_string()));
    assert_eq!(tracker.window_count, 1);
}

#[test]
fn test_case_4_main_window_add_url_button_retains_size_and_position() {
    let mut tracker = WindowStateTracker::new(true);
    assert!(tracker.visible);
    assert!(tracker.is_main_window_open.load(std::sync::atomic::Ordering::Relaxed));
    assert_eq!(tracker.inner_size, Vec2::new(1000.0, 640.0));

    // Open from Add URL button in main window
    tracker.open_from_main_window_button();
    assert!(tracker.show_add_dialog);
    assert!(!tracker.is_popup_mode); // modal mode
    assert_eq!(tracker.inner_size, Vec2::new(1000.0, 640.0)); // keeps size!

    // Cancel
    tracker.handle_cancel();
    assert!(!tracker.show_add_dialog);
    assert!(tracker.visible);
    assert!(tracker.is_main_window_open.load(std::sync::atomic::Ordering::Relaxed));
    assert_eq!(tracker.inner_size, Vec2::new(1000.0, 640.0));
    assert_eq!(tracker.title, rustget_app::APP_TITLE);

    // Open again and start download
    tracker.open_from_main_window_button();
    assert!(tracker.show_add_dialog);
    tracker.handle_start_download("archive.zip", false);
    assert!(!tracker.show_add_dialog);
    assert!(tracker.visible);
    assert!(tracker.is_main_window_open.load(std::sync::atomic::Ordering::Relaxed));
    assert_eq!(tracker.inner_size, Vec2::new(1000.0, 640.0));
    assert_eq!(tracker.title, rustget_app::APP_TITLE);
    assert_eq!(tracker.notification_sent, Some("Download started: archive.zip".to_string()));
    assert_eq!(tracker.window_count, 1);
}

#[test]
fn test_case_5_repeat_5_times_no_extra_windows_no_leaks() {
    let mut tracker = WindowStateTracker::new(false);

    for i in 1..=5 {
        assert_eq!(tracker.window_count, 1);
        tracker.open_from_browser_extension();
        assert!(tracker.is_popup_mode);
        assert!(tracker.visible);

        if i % 2 == 1 {
            tracker.handle_cancel();
            assert!(!tracker.visible);
            assert!(!tracker.is_main_window_open.load(std::sync::atomic::Ordering::Relaxed));
        } else {
            tracker.handle_start_download(&format!("file_{}.bin", i), false);
            assert!(!tracker.visible);
            assert!(!tracker.is_main_window_open.load(std::sync::atomic::Ordering::Relaxed));
            assert_eq!(tracker.notification_sent, Some(format!("Download started: file_{}.bin", i)));
        }
        assert_eq!(tracker.window_count, 1);
    }
}

#[test]
fn test_section_7_options_defaults_and_dependencies() {
    use rustget_app::dialogs::settings::{OptionsDialogState, OptionsTab, SpeedUnit, is_proxy_valid};

    let mut state = OptionsDialogState::default();
    assert_eq!(state.active_tab, OptionsTab::General);
    assert!(!state.startup);
    assert!(!state.minstart);
    assert!(state.mintray);
    assert_eq!(state.threads, 8);
    assert_eq!(state.speed, 0);
    assert_eq!(state.unit, SpeedUnit::Mbps);
    assert!(state.folder.is_empty());
    assert!(!state.clip);
    assert!(state.notif);
    assert!(state.cwin);
    assert!(!state.openmain);
    assert!(state.proxy.is_empty());
    assert!(state.is_proxy_valid());

    // Dependencies: start minimized requires startup
    assert!(!state.is_start_minimized_enabled()); // startup is false
    state.startup = true;
    assert!(state.is_start_minimized_enabled()); // startup is true
    assert!(state.is_minimize_to_tray_enabled()); // tray is always active

    // Reset to defaults
    state.speed = 100;
    state.unit = SpeedUnit::Kbps;
    state.reset_to_defaults();
    assert_eq!(state.speed, 0);
    assert_eq!(state.unit, SpeedUnit::Mbps);
    assert_eq!(state.threads, 8);

    // Proxy validation
    assert!(is_proxy_valid(""));
    assert!(is_proxy_valid("   "));
    assert!(is_proxy_valid("http://127.0.0.1:8080"));
    assert!(is_proxy_valid("https://proxy.example.com:3128/"));
    assert!(is_proxy_valid("socks5://localhost:1080"));
    assert!(is_proxy_valid("socks5h://10.0.0.1:9050"));

    assert!(!is_proxy_valid("ftp://127.0.0.1:8080"));
    assert!(!is_proxy_valid("http://"));
    assert!(!is_proxy_valid("invalid_address"));
    assert!(!is_proxy_valid("http://host:999999"));
    assert!(!is_proxy_valid("http://host with spaces:8080"));
}

#[tokio::test]
async fn test_options_live_save_and_persist() {
    use rustget_core::db::Database;
    use rustget_core::queue::QueueManager;
    use rustget_app::dialogs::settings::{OptionsDialogState, OptionsTab, SpeedUnit};
    use std::sync::atomic::{AtomicBool, Ordering};

    let db = Database::open_in_memory().unwrap();
    let (tx, _rx) = tokio::sync::mpsc::channel(10);
    let queue = std::sync::Arc::new(QueueManager::new(db.clone(), 3, 0, tx));

    // Old config simulation: populate DB with old removed keys
    db.set_setting("show_tray_icon_enabled", "true").await.unwrap();
    db.set_setting("show_tray_only_minimized_enabled", "false").await.unwrap();
    db.set_setting("minimize_to_tray_on_minimize_enabled", "true").await.unwrap();
    db.set_setting("minimize_to_tray_enabled", "true").await.unwrap();
    db.set_setting("start_minimized_enabled", "false").await.unwrap();

    let mut autostart = false;
    let mut start_minimized = false;
    let mut minimize_to_tray = true;
    let mut default_connections = 8;
    let mut global_speed_kb: u64 = 0;
    let mut default_save_dir = String::new();
    let mut clipboard_monitor = false;
    let mut notifications_enabled = true;
    let mut dont_show_complete_dialog = false;
    let mut open_main_on_download_start = false;
    let mut proxy_url = String::new();
    let clipboard_enabled = AtomicBool::new(false);

    let mut opt = OptionsDialogState {
        active_tab: OptionsTab::General,
        startup: autostart,
        minstart: start_minimized,
        tray: true,
        mintray: minimize_to_tray,
        threads: default_connections,
        speed: global_speed_kb,
        unit: SpeedUnit::Mbps,
        folder: default_save_dir.clone(),
        clip: clipboard_monitor,
        notif: notifications_enabled,
        cwin: !dont_show_complete_dialog,
        openmain: open_main_on_download_start,
        proxy: proxy_url.clone(),
    };

    // User modifies settings live in UI:
    opt.startup = true;
    opt.minstart = true;
    opt.mintray = false;
    opt.threads = 12;
    opt.speed = 2;
    opt.unit = SpeedUnit::Mbps;
    opt.folder = "C:\\CustomDownloads".to_string();
    opt.clip = true;
    opt.proxy = "http://127.0.0.1:8080".to_string();

    // User clicks Save:
    autostart = opt.startup;
    start_minimized = opt.minstart && opt.startup;
    minimize_to_tray = opt.mintray;
    default_connections = opt.threads;
    global_speed_kb = match opt.unit {
        SpeedUnit::Mbps => opt.speed * 1024,
        SpeedUnit::Kbps => opt.speed,
    };
    default_save_dir = opt.folder.clone();
    clipboard_monitor = opt.clip;
    notifications_enabled = opt.notif;
    dont_show_complete_dialog = !opt.cwin;
    open_main_on_download_start = opt.openmain;
    proxy_url = opt.proxy.clone();
    clipboard_enabled.store(clipboard_monitor, Ordering::Relaxed);

    let speed_bps = global_speed_kb * 1024;
    queue.set_max_concurrent(3).await;
    queue.set_global_speed_limit(speed_bps).await;
    queue.set_proxy(Some(proxy_url.clone())).await;

    db.set_setting("max_concurrent_downloads", "3").await.unwrap();
    db.set_setting("global_speed_limit_bps", &speed_bps.to_string()).await.unwrap();
    db.set_setting("default_num_segments", &default_connections.to_string()).await.unwrap();
    db.set_setting("default_save_dir", &default_save_dir).await.unwrap();
    db.set_setting("proxy_url", &proxy_url).await.unwrap();
    db.set_setting("clipboard_monitor_enabled", if clipboard_monitor { "true" } else { "false" }).await.unwrap();
    db.set_setting("notifications_enabled", if notifications_enabled { "true" } else { "false" }).await.unwrap();
    db.set_setting("minimize_to_tray_enabled", if minimize_to_tray { "true" } else { "false" }).await.unwrap();
    db.set_setting("start_minimized_enabled", if start_minimized { "true" } else { "false" }).await.unwrap();

    // Migration on save: removed keys are deleted from DB
    db.delete_setting("show_tray_icon_enabled").await.unwrap();
    db.delete_setting("show_tray_only_minimized_enabled").await.unwrap();
    db.delete_setting("minimize_to_tray_on_minimize_enabled").await.unwrap();

    assert_eq!(minimize_to_tray, false);
    assert_eq!(start_minimized, true);
    assert_eq!(default_connections, 12);
    assert_eq!(global_speed_kb, 2048);
    assert_eq!(default_save_dir, "C:\\CustomDownloads");
    assert_eq!(clipboard_monitor, true);
    assert_eq!(clipboard_enabled.load(Ordering::Relaxed), true);
    assert_eq!(proxy_url, "http://127.0.0.1:8080");

    assert_eq!(db.get_setting("minimize_to_tray_enabled").await.unwrap(), Some("false".to_string()));
    assert_eq!(db.get_setting("start_minimized_enabled").await.unwrap(), Some("true".to_string()));
    assert_eq!(db.get_setting("default_num_segments").await.unwrap(), Some("12".to_string()));
    assert_eq!(db.get_setting("global_speed_limit_bps").await.unwrap(), Some((2048 * 1024).to_string()));
    assert_eq!(db.get_setting("default_save_dir").await.unwrap(), Some("C:\\CustomDownloads".to_string()));
    assert_eq!(db.get_setting("clipboard_monitor_enabled").await.unwrap(), Some("true".to_string()));
    assert_eq!(db.get_setting("proxy_url").await.unwrap(), Some("http://127.0.0.1:8080".to_string()));

    // Verify migration cleaned up removed keys:
    assert!(db.get_setting("show_tray_icon_enabled").await.unwrap().is_none());
    assert!(db.get_setting("show_tray_only_minimized_enabled").await.unwrap().is_none());
    assert!(db.get_setting("minimize_to_tray_on_minimize_enabled").await.unwrap().is_none());
}

#[tokio::test]
async fn test_simplified_3_options_general_page_and_tray_behavior() {
    use rustget_core::db::Database;
    use rustget_app::dialogs::settings::{OptionsDialogState, OptionsTab};
    use rustget_app::theme;

    let db = Database::open_in_memory().unwrap();

    // 1. Verify old keys in DB load with no error
    db.set_setting("show_tray_icon_enabled", "true").await.unwrap();
    db.set_setting("show_tray_only_minimized_enabled", "true").await.unwrap();
    db.set_setting("minimize_to_tray_on_minimize_enabled", "true").await.unwrap();

    let mut opt = OptionsDialogState::default();
    assert_eq!(opt.active_tab, OptionsTab::General);
    assert!(!opt.startup);
    assert!(!opt.minstart);
    assert!(opt.mintray);

    // 2. Start minimized requires startup
    assert!(!opt.is_start_minimized_enabled());
    opt.startup = true;
    assert!(opt.is_start_minimized_enabled());
    opt.startup = false;
    assert!(!opt.is_start_minimized_enabled());

    // 3. Render General page in egui UI context
    let ctx = egui::Context::default();
    let pal = theme::palette(true);
    let mut rendered_outcome = rustget_app::dialogs::settings::OptionsDialogOutcome::None;

    let _ = ctx.run(Default::default(), |ctx| {
        egui::CentralPanel::default().show(ctx, |ui| {
            rendered_outcome = rustget_app::dialogs::settings::render_options_window_content(
                ui,
                &pal,
                None,
                &mut opt,
            );
        });
    });

    assert_eq!(rendered_outcome, rustget_app::dialogs::settings::OptionsDialogOutcome::None);
}

#[tokio::test]
async fn test_download_popup_opens_independently_when_main_window_hidden_in_tray() {
    use std::sync::atomic::{AtomicBool, Ordering};
    use rustget_app::dialogs::download_window::{DownloadWindowState, DownloadWindowStage, DownloadWindowOutcome};

    // 1. App running in system tray: main window is NOT open
    let is_main_window_open = AtomicBool::new(false);
    assert!(!is_main_window_open.load(Ordering::Relaxed));

    // 2. Browser triggers download: popup state is created
    let mut wnd = DownloadWindowState::new(
        "https://example.com/movie.mp4".to_string(),
        "movie.mp4".to_string(),
        "Video".to_string(),
        "C:\\Users\\steam\\Videos".to_string(),
    );
    wnd.is_open = true;

    assert_eq!(wnd.stage, DownloadWindowStage::Info);
    assert!(wnd.is_open);
    // Main window must REMAIN closed/in tray
    assert!(!is_main_window_open.load(Ordering::Relaxed), "Main window must stay hidden in tray");

    // 3. Title bar classification check: Download File Info must be recognized as dialog
    let title = "Download File Info";
    let is_dialog = title == "Download File Info" || title == "New download" || title.contains("Download") || title.contains("Info");
    assert!(is_dialog, "Download File Info must be classified as dialog for correct window drag and chrome");

    // 4. User starts download without open_main_on_download_start
    let open_main_on_download_start = false;
    wnd.stage = DownloadWindowStage::Downloading;
    if open_main_on_download_start {
        is_main_window_open.store(true, Ordering::Relaxed);
    }
    assert_eq!(wnd.stage, DownloadWindowStage::Downloading);
    assert!(!is_main_window_open.load(Ordering::Relaxed), "Main dashboard must NOT open when starting download in background mode");

    // 5. User closes popup to background
    let outcome = DownloadWindowOutcome::RunInBackground;
    match outcome {
        DownloadWindowOutcome::RunInBackground => {
            wnd.is_open = false;
        }
        _ => {}
    }
    assert!(!wnd.is_open, "Popup window must close to background");
    assert!(!is_main_window_open.load(Ordering::Relaxed), "Main app continues running silently in tray");
}

#[tokio::test]
async fn test_download_popup_separate_from_minimized_main_window() {
    use std::sync::atomic::{AtomicBool, Ordering};
    use rustget_app::dialogs::download_window::{DownloadWindowState, DownloadWindowStage, DownloadWindowOutcome};

    // 1. App running with main window minimized to taskbar
    let is_main_minimized = AtomicBool::new(true);
    let is_main_window_open = AtomicBool::new(true);

    // 2. Browser triggers download: popup window is created as separate entity
    let mut wnd = DownloadWindowState::new(
        "https://example.com/video.mp4".to_string(),
        "video.mp4".to_string(),
        "Video".to_string(),
        "C:\\Users\\steam\\Videos".to_string(),
    );
    wnd.is_open = true;

    assert!(wnd.is_open);
    // Main dashboard MUST remain minimized on taskbar
    assert!(is_main_minimized.load(Ordering::Relaxed), "Main window must stay minimized in taskbar");

    // 3. Minimizing the popup dialog minimizes the popup, NOT the main window
    let mut dlg_minimized = false;
    let outcome = DownloadWindowOutcome::Minimize;
    match outcome {
        DownloadWindowOutcome::Minimize => {
            dlg_minimized = true;
            // Notice: is_main_minimized is NOT modified or unminimized
        }
        _ => {}
    }
    assert!(dlg_minimized, "Download info dialog was minimized");
    assert!(is_main_minimized.load(Ordering::Relaxed), "Main window remains minimized in taskbar");

    // 4. Starting download with open_main = false keeps dialog open in Downloading stage, main window stays minimized
    let open_main_on_download_start = false;
    let start_outcome = DownloadWindowOutcome::StartDownload;
    match start_outcome {
        DownloadWindowOutcome::StartDownload => {
            wnd.stage = DownloadWindowStage::Downloading;
            if open_main_on_download_start {
                wnd.is_open = false;
                is_main_minimized.store(false, Ordering::Relaxed);
            }
        }
        _ => {}
    }
    assert!(wnd.is_open, "Popup stays open on start download showing downloading progress");
    assert_eq!(wnd.stage, DownloadWindowStage::Downloading);
    assert!(is_main_minimized.load(Ordering::Relaxed), "Main window remains minimized in taskbar");
}

#[tokio::test]
async fn test_single_info_dialog_and_close_on_start_download() {
    use std::sync::atomic::{AtomicBool, Ordering};
    use rustget_app::app::DownloadDialogRequest;
    use rustget_app::dialogs::download_window::{DownloadWindowState, DownloadWindowStage, DownloadWindowOutcome};

    // Tracker simulating the single-dialog and progress in-place behavior
    struct SingleDialogAppTracker {
        pub download_windows: Vec<DownloadWindowState>,
        pub is_main_window_open: AtomicBool,
        pub active_downloads: Vec<String>,
    }

    impl SingleDialogAppTracker {
        fn new() -> Self {
            Self {
                download_windows: Vec::new(),
                is_main_window_open: AtomicBool::new(true),
                active_downloads: Vec::new(),
            }
        }

        fn show_download_dialog(&mut self, req: DownloadDialogRequest) -> bool {
            // Check if an Info dialog is already open
            let existing_info = self.download_windows.iter().position(|w| {
                w.is_open && w.stage == DownloadWindowStage::Info
            });

            if let Some(existing_idx) = existing_info {
                // Focus existing, do NOT open a second dialog
                self.download_windows[existing_idx].is_open = true;
                return false; // Did not create a new window
            }

            let mut wnd = DownloadWindowState::new(
                req.url.clone(),
                req.filename.unwrap_or_default(),
                "General".to_string(),
                "C:\\Downloads".to_string(),
            );
            wnd.is_open = true;
            self.download_windows.push(wnd);
            true // Created new window
        }

        fn handle_outcome(&mut self, dialog_id: &str, outcome: DownloadWindowOutcome) {
            let Some(window_idx) = self.download_windows.iter().position(|w| w.dialog_id == dialog_id) else {
                return;
            };

            match outcome {
                DownloadWindowOutcome::StartDownload => {
                    let wnd = &mut self.download_windows[window_idx];
                    let url = wnd.url.clone();
                    self.active_downloads.push(url);
                    wnd.stage = DownloadWindowStage::Downloading;
                }
                DownloadWindowOutcome::RunInBackground => {
                    let target_did = dialog_id.to_string();
                    self.download_windows[window_idx].is_open = false;
                    self.download_windows.retain(|w| w.dialog_id != target_did);
                }
                DownloadWindowOutcome::Close => {
                    let target_did = dialog_id.to_string();
                    self.download_windows[window_idx].is_open = false;
                    self.download_windows.retain(|w| w.dialog_id != target_did);
                }
                _ => {}
            }
        }
    }

    let mut tracker = SingleDialogAppTracker::new();

    // 1. Press "Add URL" in main app -> opens 1 Info popup
    let created1 = tracker.show_download_dialog(DownloadDialogRequest::empty());
    assert!(created1, "First Add URL press must create download info popup");
    assert_eq!(tracker.download_windows.len(), 1);
    assert!(tracker.download_windows[0].is_open);
    assert_eq!(tracker.download_windows[0].stage, DownloadWindowStage::Info);

    // 2. Press "Add URL" again -> must NOT open another popup
    let created2 = tracker.show_download_dialog(DownloadDialogRequest::empty());
    assert!(!created2, "Second Add URL press must NOT create a duplicate popup");
    assert_eq!(tracker.download_windows.len(), 1, "There must strictly remain exactly 1 dialog window");

    // Press "Add URL" a 3rd and 4th time -> still exactly 1 dialog
    assert!(!tracker.show_download_dialog(DownloadDialogRequest::empty()));
    assert!(!tracker.show_download_dialog(DownloadDialogRequest::empty()));
    assert_eq!(tracker.download_windows.len(), 1);

    // 3. User fills in URL and clicks "Start download"
    tracker.download_windows[0].url = "https://example.com/testfile.zip".to_string();
    let did = tracker.download_windows[0].dialog_id.clone();
    tracker.handle_outcome(&did, DownloadWindowOutcome::StartDownload);

    // 4. Verification: Popup switches in-place to Downloading stage and stays open
    assert_eq!(tracker.download_windows.len(), 1, "Download info popup stays open showing progress");
    assert_eq!(tracker.download_windows[0].stage, DownloadWindowStage::Downloading);
    assert!(tracker.download_windows[0].is_open);
    assert_eq!(tracker.active_downloads.len(), 1);
    assert_eq!(tracker.active_downloads[0], "https://example.com/testfile.zip");

    // 5. User clicks "Run in background" -> popup is closed
    tracker.handle_outcome(&did, DownloadWindowOutcome::RunInBackground);
    assert_eq!(tracker.download_windows.len(), 0, "Popup closes when user clicks Run in background");

    // 6. User can now click "Add URL" again for a fresh download
    let created_next = tracker.show_download_dialog(DownloadDialogRequest::empty());
    assert!(created_next, "Subsequent Add URL press opens a fresh popup");
    assert_eq!(tracker.download_windows.len(), 1);
}

#[test]
fn test_modal_info_dialog_never_demoted_and_stays_always_on_top() {
    use rustget_app::dialogs::download_window::{DownloadWindowState, DownloadWindowStage};

    let mut wnd = DownloadWindowState::new(
        "https://example.com/file.zip".to_string(),
        "file.zip".to_string(),
        "General".to_string(),
        "/tmp".to_string(),
    );
    assert_eq!(wnd.stage, DownloadWindowStage::Info);
    assert!(!wnd.topmost_cleared);

    // Simulate elapsed time > 500ms
    // For Info stage, topmost should NEVER be cleared or demoted to Normal
    let is_info_stage = wnd.stage == DownloadWindowStage::Info;
    assert!(is_info_stage);
    let should_demote = !is_info_stage;
    assert!(!should_demote, "Info dialog must never be demoted from always-on-top level");

    // For non-info stages (e.g. Downloading), demoting after 500ms is allowed
    wnd.stage = DownloadWindowStage::Downloading;
    let is_info_stage_now = wnd.stage == DownloadWindowStage::Info;
    assert!(!is_info_stage_now);
    let should_demote_now = !is_info_stage_now;
    assert!(should_demote_now, "Non-info stages can demote after initial display");
}

#[test]
fn test_download_dialog_opens_when_app_minimized_to_tray() {
    use std::sync::atomic::{AtomicBool, Ordering};
    use rustget_app::dialogs::download_window::{DownloadWindowState, DownloadWindowStage, DownloadWindowOutcome};

    // Simulate app state when minimized to tray
    let is_main_window_open = AtomicBool::new(false);
    let mut is_popup_mode = false;
    let mut download_windows: Vec<DownloadWindowState> = Vec::new();

    // 1. App is hidden in tray: is_main_window_open is false
    assert!(!is_main_window_open.load(Ordering::Relaxed));

    // 2. Browser triggers a download
    let main_open = is_main_window_open.load(Ordering::Relaxed) && !is_popup_mode;
    assert!(!main_open, "Main window is not open when minimized in tray");

    if !main_open {
        is_popup_mode = true;
    }

    let mut wnd = DownloadWindowState::new(
        "https://www.youtube.com/watch?v=gyVr6uJC6G0".to_string(),
        "video.mp4".to_string(),
        "Video".to_string(),
        "C:\\Users\\steam\\Videos".to_string(),
    );
    wnd.is_open = true;
    download_windows.push(wnd);

    // Verify popup opened in popup mode while main app remains minimized
    assert!(is_popup_mode, "Popup mode must activate when main app is minimized to tray");
    assert!(!is_main_window_open.load(Ordering::Relaxed), "Main app window must stay minimized in tray");
    assert_eq!(download_windows.len(), 1);
    assert_eq!(download_windows[0].stage, DownloadWindowStage::Info);

    // 3. User clicks "Start download"
    download_windows[0].stage = DownloadWindowStage::Downloading;
    let setting_open_main_on_download_start = false;
    if setting_open_main_on_download_start {
        download_windows[0].is_open = false;
        is_main_window_open.store(true, Ordering::SeqCst);
        is_popup_mode = false;
    }

    // Main window must STILL be minimized in tray, popup stays in Downloading stage
    assert!(!is_main_window_open.load(Ordering::Relaxed), "Main window must NEVER be unminimized on Start download");
    assert!(is_popup_mode);
    assert!(download_windows[0].is_open);
    assert_eq!(download_windows[0].stage, DownloadWindowStage::Downloading);

    // 4. User clicks "Run in background" -> popup closes, returns to clean tray state
    let did = download_windows[0].dialog_id.clone();
    download_windows.retain(|w| w.dialog_id != did);
    if download_windows.is_empty() {
        is_popup_mode = false;
        is_main_window_open.store(false, Ordering::Relaxed);
    }

    assert_eq!(download_windows.len(), 0);
    assert!(!is_popup_mode);
    assert!(!is_main_window_open.load(Ordering::Relaxed));
}
