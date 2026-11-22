use std::collections::{HashMap, HashSet, VecDeque};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use eframe::egui::{self, Margin, Pos2, Stroke, Vec2};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use rustget_core::{
    categorize,
    category::{Category, CategoryFilter},
    db::{Database, DownloadRecord},
    downloader::{DownloadOptions, DownloadProgress},
    queue::{QueueEvent, QueueManager},
    BridgeServer, ClipboardMonitor, ExtensionDownloadRequest,
};

use crate::components::*;
use crate::dialogs::*;
use crate::platform;
use crate::theme::{self, shapes};

pub const ICON_RGBA: &[u8] = include_bytes!("icon_32.rgba");
pub const APP_TITLE: &str = "Cosmo Downloader";
pub const DOWNLOAD_FILE_INFO_TITLE: &str = "Download File Info";

static GLOBAL_QUEUE: std::sync::OnceLock<Arc<QueueManager>> = std::sync::OnceLock::new();
static GLOBAL_TOKIO_HANDLE: std::sync::OnceLock<tokio::runtime::Handle> = std::sync::OnceLock::new();
static GLOBAL_POPUP_FILE: std::sync::Mutex<Option<String>> = std::sync::Mutex::new(None);
static SHUTTING_DOWN: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

pub fn set_global_queue(q: Arc<QueueManager>) {
    let _ = GLOBAL_QUEUE.set(q);
}

pub fn get_global_queue() -> Option<Arc<QueueManager>> {
    GLOBAL_QUEUE.get().cloned()
}

pub fn set_tokio_handle(handle: tokio::runtime::Handle) {
    let _ = GLOBAL_TOKIO_HANDLE.set(handle);
}

pub fn get_tokio_handle() -> Option<tokio::runtime::Handle> {
    GLOBAL_TOKIO_HANDLE.get().cloned()
}

pub fn set_global_popup_file(popup_file: Option<String>) {
    if let Ok(mut guard) = GLOBAL_POPUP_FILE.lock() {
        *guard = popup_file;
    }
}

pub fn is_shutting_down() -> bool {
    SHUTTING_DOWN.load(Ordering::SeqCst)
}

pub fn direct_graceful_shutdown() -> ! {
    if !SHUTTING_DOWN.swap(true, Ordering::SeqCst) {
        platform::log_daemon("Direct graceful shutdown requested; destroying tray icon & saving active state...");
        
        // 1. Immediately destroy tray icon so Windows removes it without delay or ghosting
        platform::destroy_system_tray();
        
        // 2. Remove temporary standalone popup file if present
        if let Ok(mut guard) = GLOBAL_POPUP_FILE.lock() {
            if let Some(ref p) = *guard {
                let _ = std::fs::remove_file(p);
            }
            *guard = None;
        }
        
        // 3. Gracefully stop active tasks, save DB records, and flush WAL
        if let Some(q) = get_global_queue() {
            let q = Arc::clone(&q);
            if let Some(rt) = get_tokio_handle() {
                rt.block_on(async move {
                    q.shutdown_and_save().await;
                });
            } else if let Ok(rt) = tokio::runtime::Handle::try_current() {
                rt.block_on(async move {
                    q.shutdown_and_save().await;
                });
            }
        }
        
        platform::log_daemon("Direct graceful shutdown complete.");
    }
    std::process::exit(0);
}

pub fn compute_status_counts(downloads: &[DownloadRecord]) -> nav_rail::StatusCounts {
    let mut counts = nav_rail::StatusCounts {
        all: downloads.len(),
        downloading: 0,
        completed: 0,
        paused: 0,
    };
    for d in downloads {
        if d.status == "downloading" || d.status == "queued" {
            counts.downloading += 1;
        } else if d.status == "completed" {
            counts.completed += 1;
        } else if d.status == "paused" || d.status == "failed" {
            counts.paused += 1;
        }
    }
    counts
}

pub fn compute_category_counts(downloads: &[DownloadRecord]) -> CategoryCounts {
    let mut counts = CategoryCounts {
        all: downloads.len(),
        video: 0,
        music: 0,
        documents: 0,
        compressed: 0,
        programs: 0,
        other: 0,
    };
    for d in downloads {
        if CategoryFilter::Video.matches(&d.category) {
            counts.video += 1;
        } else if CategoryFilter::Music.matches(&d.category) {
            counts.music += 1;
        } else if CategoryFilter::Documents.matches(&d.category) {
            counts.documents += 1;
        } else if CategoryFilter::Compressed.matches(&d.category) {
            counts.compressed += 1;
        } else if CategoryFilter::Programs.matches(&d.category) {
            counts.programs += 1;
        } else {
            counts.other += 1;
        }
    }
    counts
}

pub const MAIN_VIEWPORT_ID: &str = "main_viewport";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LaunchOrigin {
    User,
    Autostart,
    Extension,
    Protocol,
    Clipboard,
}

pub fn determine_launch_origin(args: &[String]) -> LaunchOrigin {
    for a in args.iter().skip(1) {
        let clean = a.trim_matches('"').trim_matches('\'');
        if clean == "--from-extension" || clean == "--popup-file" || clean.starts_with("--popup-file=") || clean == "--popup" {
            return LaunchOrigin::Extension;
        }
        if clean == "--minimized" || clean == "--daemon" || clean == "--autostart" {
            return LaunchOrigin::Autostart;
        }
        if clean == "--clipboard" {
            return LaunchOrigin::Clipboard;
        }
        if clean == "--add-url" || clean.starts_with("--add-url=")
            || clean.starts_with("cosmodownloader://")
            || clean.starts_with("rustget://")
            || clean.starts_with("http://")
            || clean.starts_with("https://")
            || clean.starts_with("magnet:")
            || clean.ends_with(".torrent")
            || clean.ends_with(".TORRENT")
        {
            return LaunchOrigin::Protocol;
        }
    }
    LaunchOrigin::User
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MainWindowReason {
    UserLaunchedManually,
    TrayLeftClick,
    TrayMenuOpen,
    SecondInstanceLaunchedByUser,
    UserClickedOpenMainWindow,
    SettingOpenMainOnDownloadStart,
    Forbidden(&'static str),
}

#[derive(Clone, Debug)]
pub struct DownloadDialogRequest {
    pub url: String,
    pub filename: Option<String>,
    pub cookies: Option<String>,
    pub referer: Option<String>,
    pub user_agent: Option<String>,
    pub num_segments: Option<usize>,
}

impl DownloadDialogRequest {
    pub fn empty() -> Self {
        Self {
            url: String::new(),
            filename: None,
            cookies: None,
            referer: None,
            user_agent: None,
            num_segments: None,
        }
    }
}

impl Default for DownloadDialogRequest {
    fn default() -> Self {
        Self::empty()
    }
}

pub struct CosmoDownloaderApp {
    pub queue: Arc<QueueManager>,
    pub downloads: Vec<DownloadRecord>,
    pub active_progress: HashMap<String, DownloadProgress>,
    pub ema_speeds: HashMap<String, f64>,
    pub selected_status: nav_rail::StatusFilter,
    pub selected_category: CategoryFilter,
    pub selected_ids: HashSet<String>,
    pub search_query: String,
    pub sort_col: download_table::SortColumn,
    pub sort_asc: bool,
    pub pending_delete_ids: Vec<String>,
    pub delete_also_files: bool,
    pub is_dark: bool,
    pub logo_texture: Option<egui::TextureHandle>,

    // Dialogs & Popup Mode
    pub is_popup_mode: bool,
    pub is_standalone_popup: bool,
    pub standalone_popup_file: Option<String>,
    pub popup_opened_at: Option<std::time::Instant>,
    pub show_add_dialog: bool,
    pub show_settings_dialog: bool,
    pub show_about_dialog: bool,
    pub completed_dialog_record: Option<DownloadRecord>,
    pub active_popup_task_id: Option<String>,
    pub active_popup_filename: String,
    pub active_popup_url: String,
    pub active_popup_target_path: String,
    pub pending_task_rx: Option<tokio::sync::mpsc::Receiver<String>>,
    pub download_windows: Vec<DownloadWindowState>,
    pub window_task_tx: tokio::sync::mpsc::Sender<(usize, String)>,
    pub window_task_rx: tokio::sync::mpsc::Receiver<(usize, String)>,

    // Deduplication & queue fields
    pub recent_dialog_keys: HashMap<String, Instant>,
    pub download_started_at: HashMap<String, Instant>,
    pub pending_dialog_queue: VecDeque<DownloadDialogRequest>,
    pub extension_request_count: usize,

    // Add dialog fields
    pub add_url: String,
    pub add_filename: String,
    pub add_category: String,
    pub add_save_dir: String,
    pub add_connections: usize,
    pub add_speed_limit_kb: u64,
    pub add_checksum: String,
    pub add_cookies: Option<String>,
    pub add_referer: Option<String>,
    pub add_user_agent: Option<String>,
    pub add_file_size: Option<u64>,
    pub add_is_probing: bool,
    pub add_error_message: Option<String>,
    pub add_description: String,
    pub add_remember_category: bool,
    pub show_notes_expanded: bool,
    pub show_advanced_add: bool,
    pub size_probe_rx: Option<mpsc::Receiver<(Option<u64>, Option<String>)>>,
    pub category_save_dirs: HashMap<String, String>,

    // Settings fields
    pub setting_max_concurrent: usize,
    pub setting_global_speed_kb: u64,
    pub setting_default_connections: usize,
    pub setting_default_save_dir: String,
    pub setting_proxy_url: String,
    pub setting_clipboard_monitor: bool,
    pub setting_notifications_enabled: bool,
    pub setting_open_main_on_download_start: bool,
    pub setting_minimize_to_tray: bool,
    pub setting_show_tray_icon: bool,
    pub setting_autostart: bool,
    pub setting_start_minimized: bool,
    pub setting_dont_show_complete_dialog: bool,
    pub is_main_window_open: Arc<AtomicBool>,
    pub options_dialog_state: Option<OptionsDialogState>,

    // System tray
    pub has_tray_icon: bool,
    pub tray_tx: std::sync::mpsc::Sender<String>,
    pub tray_event_receiver: Option<std::sync::mpsc::Receiver<String>>,

    // Async event receivers
    pub event_rx: mpsc::Receiver<QueueEvent>,
    pub clipboard_rx: mpsc::Receiver<String>,
    pub clipboard_enabled: Arc<AtomicBool>,
    pub browser_dl_rx: mpsc::Receiver<ExtensionDownloadRequest>,
    pub show_rx: mpsc::Receiver<()>,
    pub quit_rx: mpsc::Receiver<()>,
    pub shutting_down: bool,
    pub initial_minimized_handled: bool,
    pub screenshot_path: Option<String>,
    pub frame_counter: usize,
    pub custom_scale: Option<f32>,
    pub hover_menu_name: Option<String>,
}

impl CosmoDownloaderApp {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let mut is_standalone_popup = false;
        let mut standalone_popup_file = None;
        let mut standalone_req = None;
        let mut initial_url_arg: Option<String> = None;
        let mut cli_view: Option<String> = None;
        let mut screenshot_path: Option<String> = None;
        let mut custom_scale: Option<f32> = None;
        let mut hover_menu_name: Option<String> = None;
        let mut force_show_add = false;

        let mut args_iter = std::env::args().skip(1);
        while let Some(arg) = args_iter.next() {
            let clean = arg.trim_matches('"').trim_matches('\'');
            if clean == "--popup-file" {
                if let Some(file_path) = args_iter.next().map(|s| s.trim_matches('"').to_string()) {
                    if let Ok(content) = std::fs::read_to_string(&file_path) {
                        let clean = content.trim_start_matches('\u{feff}');
                        if let Ok(req) = serde_json::from_str::<ExtensionDownloadRequest>(clean) {
                            standalone_req = Some(req);
                            is_standalone_popup = true;
                            standalone_popup_file = Some(file_path);
                        }
                    }
                }
            } else if clean.starts_with("--popup-file=") {
                let file_path = clean.trim_start_matches("--popup-file=").trim_matches('"').trim_matches('\'').to_string();
                if let Ok(content) = std::fs::read_to_string(&file_path) {
                    let clean_c = content.trim_start_matches('\u{feff}');
                    if let Ok(req) = serde_json::from_str::<ExtensionDownloadRequest>(clean_c) {
                        standalone_req = Some(req);
                        is_standalone_popup = true;
                        standalone_popup_file = Some(file_path);
                    }
                }
            } else if clean.starts_with("--view=") {
                cli_view = Some(clean.trim_start_matches("--view=").trim_matches('"').trim_matches('\'').to_string());
            } else if clean.starts_with("--screenshot=") {
                screenshot_path = Some(clean.trim_start_matches("--screenshot=").trim_matches('"').trim_matches('\'').to_string());
            } else if clean.starts_with("--scale=") {
                custom_scale = clean.trim_start_matches("--scale=").trim_matches('"').trim_matches('\'').parse().ok();
            } else if clean.starts_with("--hover-menu=") {
                hover_menu_name = Some(clean.trim_start_matches("--hover-menu=").trim_matches('"').trim_matches('\'').to_string());
            } else if clean == "--show-add" {
                force_show_add = true;
            } else if clean == "--add-url" {
                if let Some(next_u) = args_iter.next().map(|s| s.trim_matches('"').trim_matches('\'').to_string()) {
                    initial_url_arg = Some(next_u);
                }
            } else if clean.starts_with("--add-url=") {
                initial_url_arg = Some(clean.trim_start_matches("--add-url=").trim_matches('"').trim_matches('\'').to_string());
            } else if clean.starts_with("cosmodownloader://") {
                initial_url_arg = Some(clean.trim_start_matches("cosmodownloader://").to_string());
            } else if clean.starts_with("rustget://") {
                initial_url_arg = Some(clean.trim_start_matches("rustget://").to_string());
            } else if (clean.starts_with("http://") || clean.starts_with("https://")) && initial_url_arg.is_none() {
                initial_url_arg = Some(clean.to_string());
            }
        }

        #[cfg(windows)]
        {
            use raw_window_handle::HasWindowHandle;
            if let Ok(handle) = cc.window_handle() {
                if let raw_window_handle::RawWindowHandle::Win32(win32) = handle.as_raw() {
                    let hwnd = win32.hwnd.get() as *mut std::ffi::c_void;
                    platform::windows::set_main_dashboard_window(hwnd);
                }
            }
            platform::windows::init_shutdown_listener(&cc.egui_ctx);
        }

        let is_popup_arg = std::env::args().skip(1).any(|a| {
            let clean = a.trim_matches('"').trim_matches('\'');
            clean == "--popup"
                || clean == "--popup-file"
                || clean.starts_with("--popup-file=")
                || clean == "--add-url"
                || clean.starts_with("--add-url=")
                || clean.starts_with("cosmodownloader://")
                || clean.starts_with("rustget://")
                || clean.starts_with("http://")
                || clean.starts_with("https://")
                || clean.starts_with("--view=")
        }) || is_standalone_popup || cli_view.is_some() || initial_url_arg.is_some();
        let start_minimized_arg = std::env::args().any(|a| a == "--minimized" || a == "--daemon");
        let is_main_open = Arc::new(AtomicBool::new(!start_minimized_arg && !is_popup_arg));

        let db = Database::open_default().unwrap_or_else(|_| Database::open_in_memory().unwrap());
        let (event_tx, event_rx) = mpsc::channel(128);
        let queue = Arc::new(QueueManager::new(db.clone(), 3, 0, event_tx));
        set_global_queue(Arc::clone(&queue));

        platform::ensure_native_host_registered();

        let (bridge_tx, bridge_rx) = mpsc::channel(32);
        let (show_tx, show_rx) = mpsc::channel(16);
        let (quit_tx, quit_rx) = mpsc::channel(4);
        let daemon_online = std::net::TcpStream::connect("127.0.0.1:42424").is_ok();

        if !daemon_online {
            platform::log_daemon("Starting queue worker and bridge server on port 42424");
            let q_worker = Arc::clone(&queue);
            let q_cancel = CancellationToken::new();
            tokio::spawn(async move {
                q_worker.run_queue_worker(q_cancel).await;
            });

            let ctx_wake = cc.egui_ctx.clone();
            let ctx_wake_dl = cc.egui_ctx.clone();
            let ctx_wake_quit = cc.egui_ctx.clone();
            let is_main_open_wake = Arc::clone(&is_main_open);
            let bridge_server = BridgeServer::new(Arc::clone(&queue), 42424)
                .with_notifier(bridge_tx)
                .with_show_notifier(show_tx)
                .with_quit_notifier(quit_tx)
                .with_wake_callback(move || {
                    #[cfg(windows)]
                    platform::show_main_window();
                    ctx_wake.send_viewport_cmd(egui::ViewportCommand::Visible(true));
                    ctx_wake.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
                    ctx_wake.send_viewport_cmd(egui::ViewportCommand::Focus);
                    ctx_wake.send_viewport_cmd(egui::ViewportCommand::RequestUserAttention(egui::UserAttentionType::Critical));
                })
                .with_download_wake_callback(move || {
                    #[cfg(windows)]
                    platform::windows::grant_foreground_permission();
                    if !is_main_open_wake.load(Ordering::Relaxed) {
                        #[cfg(windows)]
                        platform::windows::show_download_popup_window();
                        ctx_wake_dl.send_viewport_cmd(egui::ViewportCommand::Visible(true));
                        ctx_wake_dl.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
                    }
                    ctx_wake_dl.request_repaint();
                });

            let bridge_cancel = CancellationToken::new();
            tokio::spawn(async move {
                let _ = bridge_server.run(bridge_cancel).await;
                ctx_wake_quit.request_repaint();
            });
        }

        let (autostart, reg_start_minimized) = platform::get_autostart_info();
        let (
            saved_max_conc,
            saved_speed_bps,
            saved_conns,
            saved_save_dir,
            saved_proxy,
            saved_clip,
            saved_notify,
            saved_open_main_start,
            saved_min_tray,
            saved_show_tray,
            saved_start_min,
            saved_dont_complete,
            saved_theme,
            saved_cat_dirs,
        ) = {
            let rt = tokio::runtime::Handle::current();
            rt.block_on(async {
                let db = queue.db();
                let max_c = db
                    .get_setting("max_concurrent_downloads")
                    .await
                    .ok()
                    .flatten()
                    .and_then(|v| v.parse::<usize>().ok())
                    .unwrap_or(3)
                    .clamp(1, 10);
                let speed_bps = db
                    .get_setting("global_speed_limit_bps")
                    .await
                    .ok()
                    .flatten()
                    .and_then(|v| v.parse::<u64>().ok())
                    .unwrap_or(0);
                let conns = db
                    .get_setting("default_num_segments")
                    .await
                    .ok()
                    .flatten()
                    .and_then(|v| v.parse::<usize>().ok())
                    .unwrap_or(8)
                    .clamp(1, 32);
                let save_dir = db
                    .get_setting("default_save_dir")
                    .await
                    .ok()
                    .flatten()
                    .unwrap_or_default();
                let proxy = db
                    .get_setting("proxy_url")
                    .await
                    .ok()
                    .flatten()
                    .unwrap_or_default();
                let clip = db
                    .get_setting("clipboard_monitor_enabled")
                    .await
                    .ok()
                    .flatten()
                    .map(|v| v == "true")
                    .unwrap_or(false);
                let notify = db
                    .get_setting("notifications_enabled")
                    .await
                    .ok()
                    .flatten()
                    .map(|v| v == "true")
                    .unwrap_or(true);
                let open_main_start = db
                    .get_setting("open_main_on_download_start")
                    .await
                    .ok()
                    .flatten()
                    .map(|v| v == "true")
                    .unwrap_or(false);
                let min_tray = db
                    .get_setting("minimize_to_tray_enabled")
                    .await
                    .ok()
                    .flatten()
                    .map(|v| v == "true")
                    .unwrap_or(true);
                let show_tray = db
                    .get_setting("show_tray_icon_enabled")
                    .await
                    .ok()
                    .flatten()
                    .map(|v| v == "true")
                    .unwrap_or(true);
                let start_min = db
                    .get_setting("start_minimized_enabled")
                    .await
                    .ok()
                    .flatten()
                    .map(|v| v == "true")
                    .unwrap_or(reg_start_minimized);
                let dont_complete = db
                    .get_setting("dont_show_complete_dialog")
                    .await
                    .ok()
                    .flatten()
                    .map(|v| v == "true")
                    .unwrap_or(false);
                let ui_theme = db.get_setting("ui_theme").await.ok().flatten();
                let mut cat_dirs = HashMap::new();
                for cat_key in ["video", "music", "documents", "compressed", "programs", "general"] {
                    let setting_k = format!("category_dir_{}", cat_key);
                    if let Ok(Some(val)) = db.get_setting(&setting_k).await {
                        if !val.trim().is_empty() {
                            cat_dirs.insert(cat_key.to_string(), val);
                        }
                    }
                }

                queue.set_max_concurrent(max_c).await;
                queue.set_global_speed_limit(speed_bps).await;
                if !proxy.trim().is_empty() {
                    queue.set_proxy(Some(proxy.clone())).await;
                }

                (
                    max_c,
                    speed_bps,
                    conns,
                    save_dir,
                    proxy,
                    clip,
                    notify,
                    open_main_start,
                    min_tray,
                    show_tray,
                    start_min,
                    dont_complete,
                    ui_theme,
                    cat_dirs,
                )
            })
        };

        let (clip_tx, clip_rx) = mpsc::channel(32);
        let clip_cancel = CancellationToken::new();
        let clip_wake_ctx = cc.egui_ctx.clone();
        let clip_monitor = ClipboardMonitor::new(clip_cancel)
            .with_enabled(saved_clip)
            .with_wake_callback(move || {
                clip_wake_ctx.request_repaint();
            });
        let clipboard_enabled = clip_monitor.enabled_flag();
        clip_monitor.start(clip_tx);

        let is_dark = match saved_theme.as_deref() {
            Some("light") => false,
            Some("dark") => true,
            _ => platform::system_prefers_dark(),
        };
        theme::configure_fonts(&cc.egui_ctx);
        theme::configure_visuals(&cc.egui_ctx, is_dark);

        let color_image = egui::ColorImage::from_rgba_unmultiplied([32, 32], ICON_RGBA);
        let logo_texture = Some(cc.egui_ctx.load_texture("cosmo_logo", color_image, egui::TextureOptions::LINEAR));

        let (tray_tx, tray_event_receiver) = std::sync::mpsc::channel();
        // Always initialize tray icon on the primary single-instance process (even if launched via --popup-file)
        let has_tray_icon = if !daemon_online && saved_show_tray {
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                platform::build_system_tray(&tray_tx, cc.egui_ctx.clone(), ICON_RGBA, APP_TITLE)
            }))
            .unwrap_or(false)
        } else {
            false
        };

        let initial_downloads = {
            let rt = tokio::runtime::Handle::current();
            rt.block_on(async {
                queue.db().list_downloads(None).await.unwrap_or_default()
            })
        };

        let initial_category = {
            let rt = tokio::runtime::Handle::current();
            rt.block_on(async {
                match queue.db().get_setting("ui_selected_category").await.unwrap_or(None).as_deref() {
                    Some("video") => CategoryFilter::Video,
                    Some("music") => CategoryFilter::Music,
                    Some("documents") => CategoryFilter::Documents,
                    Some("compressed") => CategoryFilter::Compressed,
                    Some("programs") => CategoryFilter::Programs,
                    Some("other") => CategoryFilter::Other,
                    _ => CategoryFilter::All,
                }
            })
        };

        let (window_task_tx, window_task_rx) = mpsc::channel(32);

        let mut app = Self {
            queue,
            downloads: initial_downloads,
            active_progress: HashMap::new(),
            ema_speeds: HashMap::new(),
            selected_status: nav_rail::StatusFilter::All,
            selected_category: initial_category,
            selected_ids: HashSet::new(),
            search_query: String::new(),
            sort_col: download_table::SortColumn::Added,
            sort_asc: false,
            pending_delete_ids: Vec::new(),
            delete_also_files: false,
            is_dark,
            logo_texture,

            is_popup_mode: is_popup_arg,
            is_standalone_popup,
            standalone_popup_file: {
                set_global_popup_file(standalone_popup_file.clone());
                standalone_popup_file
            },
            popup_opened_at: if is_popup_arg { Some(std::time::Instant::now()) } else { None },
            show_add_dialog: is_popup_arg,
            show_settings_dialog: false,
            show_about_dialog: false,
            completed_dialog_record: None,
            active_popup_task_id: None,
            active_popup_filename: String::new(),
            active_popup_url: String::new(),
            active_popup_target_path: String::new(),
            pending_task_rx: None,
            download_windows: Vec::new(),
            window_task_tx,
            window_task_rx,

            recent_dialog_keys: HashMap::new(),
            download_started_at: HashMap::new(),
            pending_dialog_queue: VecDeque::new(),
            extension_request_count: 0,

            add_url: String::new(),
            add_filename: String::new(),
            add_category: "General".to_string(),
            add_save_dir: if !saved_save_dir.trim().is_empty() {
                saved_save_dir.clone()
            } else {
                Category::General.default_dir().to_string_lossy().to_string()
            },
            add_connections: saved_conns,
            add_speed_limit_kb: 0,
            add_checksum: String::new(),
            add_cookies: None,
            add_referer: None,
            add_user_agent: None,
            add_file_size: None,
            add_is_probing: false,
            add_error_message: None,
            add_description: String::new(),
            add_remember_category: true,
            show_notes_expanded: false,
            show_advanced_add: false,
            size_probe_rx: None,
            category_save_dirs: saved_cat_dirs,

            setting_max_concurrent: saved_max_conc,
            setting_global_speed_kb: saved_speed_bps / 1024,
            setting_default_connections: saved_conns,
            setting_default_save_dir: saved_save_dir,
            setting_proxy_url: saved_proxy,
            setting_clipboard_monitor: saved_clip,
            setting_notifications_enabled: saved_notify,
            setting_open_main_on_download_start: saved_open_main_start,
            setting_minimize_to_tray: saved_min_tray,
            setting_show_tray_icon: saved_show_tray,
            setting_autostart: autostart,
            setting_start_minimized: saved_start_min,
            setting_dont_show_complete_dialog: saved_dont_complete,
            is_main_window_open: is_main_open,
            options_dialog_state: None,

            has_tray_icon,
            tray_tx,
            tray_event_receiver: Some(tray_event_receiver),

            event_rx,
            clipboard_rx: clip_rx,
            clipboard_enabled,
            browser_dl_rx: bridge_rx,
            show_rx,
            quit_rx,
            shutting_down: false,
            initial_minimized_handled: false,
            screenshot_path,
            frame_counter: 0,
            custom_scale,
            hover_menu_name,
        };

        if let Some(ref view) = cli_view {
            let mut wnd = DownloadWindowState::new(
                "https://speed.hetzner.de/100MB.bin".to_string(),
                "100MB.bin".to_string(),
                "General".to_string(),
                "C:\\Users\\steam\\Downloads".to_string(),
            );
            wnd.target_path = "C:\\Users\\steam\\Downloads\\100MB.bin".to_string();
            match view.as_str() {
                "downloading" => {
                    wnd.stage = DownloadWindowStage::Downloading;
                    wnd.downloaded_bytes = 119_400_000;
                    wnd.total_bytes = Some(284_300_000);
                    wnd.speed_bps = 8_200_000.0;
                    wnd.eta_seconds = Some(20);
                    wnd.disp_downloaded = 119_400_000;
                    wnd.disp_total = Some(284_300_000);
                    wnd.disp_speed_bps = 8_200_000.0;
                    wnd.disp_eta_seconds = Some(20);
                }
                "paused" => {
                    wnd.stage = DownloadWindowStage::Paused;
                    wnd.downloaded_bytes = 119_400_000;
                    wnd.total_bytes = Some(284_300_000);
                    wnd.disp_downloaded = 119_400_000;
                    wnd.disp_total = Some(284_300_000);
                }
                "complete" => {
                    wnd.stage = DownloadWindowStage::Complete;
                    wnd.downloaded_bytes = 284_340_000;
                    wnd.total_bytes = Some(284_340_000);
                    wnd.disp_downloaded = 284_340_000;
                    wnd.disp_total = Some(284_340_000);
                }
                "failed" => {
                    wnd.stage = DownloadWindowStage::Failed {
                        reason: "Network connection lost".to_string(),
                    };
                    wnd.downloaded_bytes = 52_000_000;
                    wnd.total_bytes = Some(284_300_000);
                    wnd.disp_downloaded = 52_000_000;
                    wnd.disp_total = Some(284_300_000);
                }
                "cancel" => {
                    wnd.stage = DownloadWindowStage::Downloading;
                    wnd.confirming_cancel = true;
                    wnd.downloaded_bytes = 119_400_000;
                    wnd.total_bytes = Some(284_300_000);
                    wnd.disp_downloaded = 119_400_000;
                    wnd.disp_total = Some(284_300_000);
                }
                "info" => {
                    wnd.url = "https://www.youtube.com/watch?v=B402rKl4bUg".to_string();
                    wnd.filename = "Olivia Rodrigo - the cure (Official Video).mp4".to_string();
                    wnd.category = "Video".to_string();
                    wnd.stage = DownloadWindowStage::Info;
                }
                "test3" => {
                    wnd.url = "https://www.youtube.com/watch?v=B402rKl4bUg".to_string();
                    wnd.filename = "Olivia Rodrigo - the cure (Official Video).mp4".to_string();
                    wnd.category = "Video".to_string();
                    wnd.stage = DownloadWindowStage::Info;
                }
                "test4" => {
                    wnd.url = "https://www.youtube.com/watch?v=kJQP7kiw5Fk".to_string();
                    wnd.filename = "Luis Fonsi - Despacito ft. Daddy Yankee.mp4".to_string();
                    wnd.category = "Video".to_string();
                    wnd.stage = DownloadWindowStage::Info;
                }
                "test6" => {
                    wnd.url = "https://www.youtube.com/watch?v=B402rKl4bUg".to_string();
                    wnd.filename = "Olivia Rodrigo - the cure (Official Video).mp4".to_string();
                    wnd.category = "Video".to_string();
                    wnd.stage = DownloadWindowStage::Downloading;
                    wnd.downloaded_bytes = 45_200_000;
                    wnd.total_bytes = Some(112_000_000);
                    wnd.speed_bps = 7_400_000.0;
                    wnd.eta_seconds = Some(9);
                    wnd.disp_downloaded = 45_200_000;
                    wnd.disp_total = Some(112_000_000);
                    wnd.disp_speed_bps = 7_400_000.0;
                    wnd.disp_eta_seconds = Some(9);
                }
                "cascade" | "cascade_canvas" => {
                    wnd.url = "https://www.youtube.com/watch?v=B402rKl4bUg".to_string();
                    wnd.filename = "Olivia Rodrigo - the cure (Official Video).mp4".to_string();
                    wnd.category = "Video".to_string();
                    wnd.stage = DownloadWindowStage::Info;
                    wnd.window_pos = Some(Pos2::new(20.0, 20.0));
                    app.download_windows.push(wnd);

                    let mut wnd2 = DownloadWindowState::new(
                        "https://www.youtube.com/watch?v=kJQP7kiw5Fk".to_string(),
                        "Luis Fonsi - Despacito ft. Daddy Yankee.mp4".to_string(),
                        "Video".to_string(),
                        "C:\\Users\\steam\\Downloads".to_string(),
                    );
                    wnd2.stage = DownloadWindowStage::Info;
                    wnd2.window_pos = Some(Pos2::new(44.0, 44.0)); // 24px cascade
                    app.download_windows.push(wnd2);
                    return app;
                }
                "options" => {
                    app.is_popup_mode = false;
                    app.is_main_window_open.store(true, Ordering::SeqCst);
                    app.open_options_dialog();
                    return app;
                }
                _ => {}
            }
            app.download_windows.push(wnd);
        } else if force_show_add {
            if let Some(ref url) = initial_url_arg {
                let d_req = DownloadDialogRequest {
                    url: url.clone(),
                    filename: None,
                    cookies: None,
                    referer: None,
                    user_agent: None,
                    num_segments: None,
                };
                app.show_download_dialog(d_req, "force_show_add", None);
            } else {
                app.show_download_dialog(DownloadDialogRequest::empty(), "force_show_add", None);
            }
        } else if let Some(req) = standalone_req {
            let d_req = DownloadDialogRequest {
                url: req.url,
                filename: req.filename,
                cookies: req.cookies,
                referer: req.referer,
                user_agent: req.user_agent,
                num_segments: req.num_segments,
            };
            app.show_download_dialog(d_req, "standalone_req", None);
        } else if let Some(url_arg) = initial_url_arg {
            let d_req = DownloadDialogRequest {
                url: url_arg,
                filename: None,
                cookies: None,
                referer: None,
                user_agent: None,
                num_segments: None,
            };
            app.show_download_dialog(d_req, "initial_url_arg", None);
        }

        app
    }

    /// Cancels active tasks cleanly (preserving them as `'queued'` for auto-resume),
    /// flushes SQLite WAL, removes tray icon & temp popup files, and exits cleanly.
    pub fn perform_graceful_shutdown(&mut self) -> ! {
        if let Some(ref p) = self.standalone_popup_file {
            let _ = std::fs::remove_file(p);
        }
        self.has_tray_icon = false;
        direct_graceful_shutdown();
    }

    pub fn refresh_downloads(&mut self) {
        let q = Arc::clone(&self.queue);
        let downloads = {
            let rt = tokio::runtime::Handle::current();
            rt.block_on(async {
                q.db().list_downloads(None).await.unwrap_or_default()
            })
        };
        let valid_ids: HashSet<&str> = downloads.iter().map(|d| d.id.as_str()).collect();
        self.selected_ids.retain(|id| valid_ids.contains(id.as_str()));
        self.downloads = downloads;
    }

    pub fn resolve_save_dir_for_category(&self, cat: Category) -> String {
        let key = cat.name().to_ascii_lowercase();
        if let Some(saved) = self.category_save_dirs.get(&key) {
            if !saved.trim().is_empty() {
                return saved.clone();
            }
        }
        if !self.setting_default_save_dir.trim().is_empty() {
            return self.setting_default_save_dir.clone();
        }
        cat.default_dir().to_string_lossy().to_string()
    }

    pub fn spawn_size_probe(&mut self) {
        platform::log_daemon(&format!("STEP: video info lookup started: url='{}'", self.add_url));
        self.add_file_size = None;
        self.add_is_probing = false;
        self.size_probe_rx = None;
        if self.add_url.trim().is_empty() || rustget_core::youtube::is_streaming_site(&self.add_url) {
            platform::log_daemon(&format!("STEP: video info lookup finished (streaming site bypassed): url='{}'", self.add_url));
            return;
        }
        if rustget_core::probe::validate_http_url(&self.add_url).is_err() {
            return;
        }
        if let Ok(parsed_url) = url::Url::parse(self.add_url.trim()) {
            let (tx, rx) = mpsc::channel(1);
            self.size_probe_rx = Some(rx);
            self.add_is_probing = true;
            let cookies_clone = self.add_cookies.clone();
            let referer_clone = self.add_referer.clone();
            let ua_clone = self.add_user_agent.clone();
            let proxy_clone = self.setting_proxy_url.clone();
            tokio::spawn(async move {
                let mut builder = reqwest::Client::builder().timeout(Duration::from_secs(4));
                if !proxy_clone.trim().is_empty() {
                    if let Ok(p) = reqwest::Proxy::all(proxy_clone.trim()) {
                        builder = builder.proxy(p);
                    }
                }
                if let Ok(c) = builder.build() {
                    let mut headers = reqwest::header::HeaderMap::new();
                    if let Some(ref ua) = ua_clone {
                        if let Ok(hv) = reqwest::header::HeaderValue::from_str(ua) {
                            headers.insert(reqwest::header::USER_AGENT, hv);
                        }
                    }
                    if let Some(ref r) = referer_clone {
                        if let Ok(hv) = reqwest::header::HeaderValue::from_str(r) {
                            headers.insert(reqwest::header::REFERER, hv);
                        }
                    }
                    if let Some(ref ck) = cookies_clone {
                        if let Ok(hv) = reqwest::header::HeaderValue::from_str(ck) {
                            headers.insert(reqwest::header::COOKIE, hv);
                        }
                    }
                    if let Ok(info) = rustget_core::probe::probe_url(&c, &parsed_url, &headers).await {
                        let _ = tx.send((info.content_length, Some(info.suggested_filename))).await;
                    }
                }
            });
        }
    }

    pub fn show_download_dialog(
        &mut self,
        req: DownloadDialogRequest,
        source: &str,
        ctx: Option<&egui::Context>,
    ) {
        let timestamp = chrono::Local::now().format("%Y-%m-%d %H:%M:%S%.3f");
        platform::log_daemon(&format!(
            "[{}] ShowDownloadDialog called: source='{}', url='{}'",
            timestamp, source, req.url
        ));
        tracing::info!(
            "[{}] ShowDownloadDialog called: source='{}', url='{}'",
            timestamp, source, req.url
        );

        let url = req.url.trim();
        let key = if !url.is_empty() {
            rustget_core::youtube::normalize_url_key(url)
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
                    && rustget_core::youtube::normalize_url_key(&w.url) == key
            });

            let recent_open = self
                .recent_dialog_keys
                .get(&key)
                .map(|t| now.duration_since(*t) < Duration::from_secs(2))
                .unwrap_or(false);

            if let Some((existing_idx, _)) = has_open_dialog {
                let target_id = self.download_windows[existing_idx].dialog_id.clone();
                platform::log_daemon(&format!(
                    "DEDUPE: Restoring and focusing existing dialog idx={} (dialog_id='{}') for key '{}'",
                    existing_idx, target_id, key
                ));
                tracing::info!(
                    "DEDUPE: Restoring and focusing existing dialog idx={} (dialog_id='{}') for key '{}'",
                    existing_idx, target_id, key
                );
                self.download_windows[existing_idx].is_open = true;
                self.download_windows[existing_idx].confirming_cancel = false;
                if let Some(c) = ctx {
                    if self.is_popup_mode && existing_idx == 0 {
                        c.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
                        c.send_viewport_cmd(egui::ViewportCommand::Visible(true));
                        c.send_viewport_cmd(egui::ViewportCommand::Focus);
                        c.send_viewport_cmd(egui::ViewportCommand::RequestUserAttention(egui::UserAttentionType::Critical));
                    } else {
                        let vp_id = egui::ViewportId::from_hash_of(format!("dl_window_{}", target_id));
                        c.send_viewport_cmd_to(vp_id, egui::ViewportCommand::Minimized(false));
                        c.send_viewport_cmd_to(vp_id, egui::ViewportCommand::Visible(true));
                        c.send_viewport_cmd_to(vp_id, egui::ViewportCommand::Focus);
                        c.send_viewport_cmd_to(vp_id, egui::ViewportCommand::RequestUserAttention(egui::UserAttentionType::Critical));
                    }
                    c.request_repaint();
                }
                #[cfg(windows)]
                platform::windows::show_download_popup_window();
                return;
            } else if recent_open {
                platform::log_daemon(&format!(
                    "DEDUPE: Suppressing duplicate dialog for key '{}' (opened within last 2 seconds)",
                    key
                ));
                tracing::info!(
                    "DEDUPE: Suppressing duplicate dialog for key '{}' (opened within last 2 seconds)",
                    key
                );
                if let Some((existing_idx, _)) = self.download_windows.iter().enumerate().rev().find(|(_, w)| {
                    rustget_core::youtube::normalize_url_key(&w.url) == key
                }) {
                    let target_id = self.download_windows[existing_idx].dialog_id.clone();
                    self.download_windows[existing_idx].is_open = true;
                    if let Some(c) = ctx {
                        if self.is_popup_mode && existing_idx == 0 {
                            c.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
                            c.send_viewport_cmd(egui::ViewportCommand::Visible(true));
                            c.send_viewport_cmd(egui::ViewportCommand::Focus);
                        } else {
                            let vp_id = egui::ViewportId::from_hash_of(format!("dl_window_{}", target_id));
                            c.send_viewport_cmd_to(vp_id, egui::ViewportCommand::Minimized(false));
                            c.send_viewport_cmd_to(vp_id, egui::ViewportCommand::Visible(true));
                            c.send_viewport_cmd_to(vp_id, egui::ViewportCommand::Focus);
                        }
                        c.request_repaint();
                    }
                    #[cfg(windows)]
                    platform::windows::show_download_popup_window();
                }
                return;
            }

            // STEP 2 Check 2: Download already running or started in last 5 seconds
            let download_recently_started = self
                .download_started_at
                .get(&key)
                .map(|t| now.duration_since(*t) < Duration::from_secs(5))
                .unwrap_or(false);

            let is_download_active = self.downloads.iter().any(|d| {
                (d.status == "downloading"
                    || d.status == "queued"
                    || d.status == "resumed"
                    || d.status == "paused")
                    && rustget_core::youtube::normalize_url_key(&d.url) == key
            });

            if download_recently_started || is_download_active {
                platform::log_daemon(&format!(
                    "DEDUPE: Suppressing dialog for key '{}' (download already active or started within 5s)",
                    key
                ));
                tracing::info!(
                    "DEDUPE: Suppressing dialog for key '{}' (download already active or started within 5s)",
                    key
                );
                return;
            }
        }

        // STEP 2: Up to 5 windows open at once. Extra ones wait in queue.
        self.download_windows.retain(|w| w.is_open);
        let open_count = self.download_windows.len();
        if open_count >= 5 {
            platform::log_daemon(&format!(
                "QUEUE: 5 dialogs already open, queueing request for '{}'",
                req.url
            ));
            tracing::info!(
                "QUEUE: 5 dialogs already open, queueing request for '{}'",
                req.url
            );
            self.pending_dialog_queue.push_back(req);
            return;
        }

        // STEP 3: Consistent position - centered over main window if visible, otherwise screen work area where mouse is
        let main_visible = self.is_main_window_open.load(Ordering::Relaxed) && !self.is_popup_mode;
        let (base_x, base_y) = platform::get_dialog_center_pos(
            main_visible,
            crate::dialogs::download_window::DOWNLOAD_WINDOW_WIDTH,
            crate::dialogs::download_window::DOWNLOAD_WINDOW_HEIGHT,
        );

        let cascade_idx = open_count % 5;
        let cascade_offset = (cascade_idx as f32) * 24.0;
        let win_pos = Pos2::new(base_x + cascade_offset, base_y + cascade_offset);

        // Prepare dialog attributes
        let clean_url = if !req.url.trim().is_empty() {
            rustget_core::youtube::normalize_streaming_url(req.url.trim())
        } else {
            String::new()
        };

        let mut filename = if let Some(ref f) = req.filename {
            if !f.trim().is_empty() {
                rustget_core::probe::sanitize_filename(f)
            } else {
                String::new()
            }
        } else if let Ok(parsed) = url::Url::parse(&clean_url) {
            let name = rustget_core::probe::extract_filename(&reqwest::header::HeaderMap::new(), &parsed);
            rustget_core::probe::sanitize_filename(&name)
        } else {
            String::new()
        };

        let cat = if rustget_core::youtube::is_streaming_site(&clean_url) {
            if filename.is_empty() || filename.eq_ignore_ascii_case("watch") {
                filename = "YouTube_Video.mp4".to_string();
            } else if !filename.contains('.') {
                filename.push_str(".mp4");
            }
            Category::Video
        } else if !filename.is_empty() {
            categorize(Some(&filename), None)
        } else {
            Category::General
        };

        let save_dir = self.resolve_save_dir_for_category(cat);

        let mut wnd = DownloadWindowState::new(
            clean_url.clone(),
            filename,
            cat.name().to_string(),
            save_dir,
        );
        wnd.connections = req.num_segments.unwrap_or(self.setting_default_connections).clamp(1, 32);
        wnd.cascade_index = cascade_idx;
        wnd.window_pos = Some(win_pos);
        wnd.is_open = true;

        if !key.is_empty() {
            self.recent_dialog_keys.insert(key.clone(), Instant::now());
        }

        let new_idx = self.download_windows.len();
        let _dialog_id = wnd.dialog_id.clone();
        self.download_windows.push(wnd);
        platform::log_daemon(&format!("STEP: dialog window created: idx={}, url='{}'", new_idx, clean_url));
        platform::log_daemon(&format!("STEP: dialog view/view-model created and bound: idx={}, url='{}'", new_idx, clean_url));

        // Keep internal add fields updated for size probe
        self.add_url = clean_url;
        self.add_category = cat.name().to_string();
        self.add_save_dir = self.resolve_save_dir_for_category(cat);
        self.spawn_size_probe();

        platform::log_daemon(&format!(
            "DIALOG_OPEN: Created dialog idx={} for key '{}' at ({}, {}) [cascade={}]",
            new_idx, key, win_pos.x, win_pos.y, cascade_idx
        ));
        tracing::info!(
            "DIALOG_OPEN: Created dialog idx={} for key '{}' at ({}, {}) [cascade={}]",
            new_idx, key, win_pos.x, win_pos.y, cascade_idx
        );

        let _from_extension = source == "extension" || source == "standalone_req" || source == "force_show_add" || source == "initial_url_arg";
        let main_open = self.is_main_window_open.load(Ordering::Relaxed) && !self.is_popup_mode;
        if !main_open {
            self.is_popup_mode = true;
            #[cfg(windows)]
            if open_count == 0 {
                platform::windows::show_download_popup_window();
            }
            if let Some(c) = ctx {
                if open_count == 0 {
                    c.send_viewport_cmd(egui::ViewportCommand::Title(DOWNLOAD_FILE_INFO_TITLE.to_string()));
                    c.send_viewport_cmd(egui::ViewportCommand::Visible(true));
                    c.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
                    c.send_viewport_cmd(egui::ViewportCommand::Focus);
                    c.send_viewport_cmd(egui::ViewportCommand::RequestUserAttention(egui::UserAttentionType::Critical));
                    c.send_viewport_cmd(egui::ViewportCommand::WindowLevel(egui::WindowLevel::AlwaysOnTop));
                    c.send_viewport_cmd(egui::ViewportCommand::MinInnerSize(Vec2::new(
                        crate::dialogs::download_window::DOWNLOAD_WINDOW_WIDTH,
                        crate::dialogs::download_window::DOWNLOAD_WINDOW_HEIGHT,
                    )));
                    c.send_viewport_cmd(egui::ViewportCommand::MaxInnerSize(Vec2::new(
                        crate::dialogs::download_window::DOWNLOAD_WINDOW_WIDTH,
                        crate::dialogs::download_window::DOWNLOAD_WINDOW_HEIGHT,
                    )));
                    c.send_viewport_cmd(egui::ViewportCommand::InnerSize(Vec2::new(
                        crate::dialogs::download_window::DOWNLOAD_WINDOW_WIDTH,
                        crate::dialogs::download_window::DOWNLOAD_WINDOW_HEIGHT,
                    )));
                    c.send_viewport_cmd(egui::ViewportCommand::Resizable(false));
                    let (cx, cy) = platform::get_dialog_center_pos(false, crate::dialogs::download_window::DOWNLOAD_WINDOW_WIDTH, crate::dialogs::download_window::DOWNLOAD_WINDOW_HEIGHT);
                    c.send_viewport_cmd(egui::ViewportCommand::OuterPosition(Pos2::new(cx, cy)));
                } else {
                    let vp_id = egui::ViewportId::from_hash_of(format!("dl_window_{}", _dialog_id));
                    c.send_viewport_cmd_to(vp_id, egui::ViewportCommand::Focus);
                    c.send_viewport_cmd_to(vp_id, egui::ViewportCommand::RequestUserAttention(egui::UserAttentionType::Critical));
                }
            }
        } else {
            // Main window is already open: show the "Download File Info" popup in an independent viewport window
            self.download_windows[new_idx].is_open = true;
            #[cfg(windows)]
            platform::windows::grant_foreground_permission();
            if let Some(c) = ctx {
                let vp_id = egui::ViewportId::from_hash_of(format!("dl_window_{}", _dialog_id));
                c.send_viewport_cmd_to(vp_id, egui::ViewportCommand::Focus);
                c.send_viewport_cmd_to(vp_id, egui::ViewportCommand::RequestUserAttention(egui::UserAttentionType::Critical));
            }
        }

        if let Some(c) = ctx {
            c.request_repaint();
        }
    }

    pub fn clear_dedupe_keys_for_url(&mut self, url: &str) {
        let key = rustget_core::youtube::normalize_url_key(url);
        if !key.is_empty() {
            platform::log_daemon(&format!("DEDUPE: Clearing dedupe keys for key '{}'", key));
            self.recent_dialog_keys.remove(&key);
            self.download_started_at.remove(&key);
        }
    }

    pub fn check_pending_dialog_queue(&mut self, ctx: &egui::Context) {
        let open_count = self.download_windows.iter().filter(|w| w.is_open).count();
        if open_count < 5 {
            if let Some(next_req) = self.pending_dialog_queue.pop_front() {
                platform::log_daemon(&format!(
                    "QUEUE_POP: Popping queued dialog request for '{}' (current open_count={})",
                    next_req.url, open_count
                ));
                self.show_download_dialog(next_req, "queue", Some(ctx));
            }
        }
    }

    pub fn prepare_download_dialog(
        &mut self,
        url: &str,
        filename: Option<String>,
        cookies: Option<String>,
        referer: Option<String>,
        user_agent: Option<String>,
        num_segments: Option<usize>,
    ) {
        let req = DownloadDialogRequest {
            url: url.to_string(),
            filename,
            cookies,
            referer,
            user_agent,
            num_segments,
        };
        self.show_download_dialog(req, "legacy_prepare", None);
    }

    pub fn trigger_download_for_window(&mut self, window_idx: usize, start_paused: bool) -> bool {
        if window_idx >= self.download_windows.len() {
            return false;
        }
        let wnd = &mut self.download_windows[window_idx];
        let url = wnd.url.trim().to_string();
        platform::log_daemon(&format!("STEP: download started: idx={}, url='{}'", window_idx, url));
        if let Err(err) = rustget_core::probe::validate_http_url(&url) {
            wnd.error_message = Some(err.to_string());
            return false;
        }
        wnd.error_message = None;

        let chosen_cat = Category::from_name(&wnd.category);
        if wnd.remember_category && !wnd.save_dir.trim().is_empty() {
            if let Some(cat) = chosen_cat {
                let key = cat.name().to_ascii_lowercase();
                let dir_val = wnd.save_dir.trim().to_string();
                self.category_save_dirs.insert(key.clone(), dir_val.clone());
                let db = self.queue.db().clone();
                tokio::spawn(async move {
                    let _ = db.set_setting(&format!("category_dir_{}", key), &dir_val).await;
                });
            }
        }

        let mut opt = DownloadOptions {
            url: url.clone(),
            num_segments: wnd.connections.clamp(1, 32),
            expected_sha256: if wnd.checksum.trim().is_empty() {
                None
            } else {
                Some(wnd.checksum.trim().to_string())
            },
            ..Default::default()
        };

        let mut out_path = if !wnd.save_dir.trim().is_empty() {
            PathBuf::from(wnd.save_dir.trim())
        } else {
            Category::General.default_dir()
        };
        if !wnd.filename.trim().is_empty() {
            out_path.push(rustget_core::probe::sanitize_filename(wnd.filename.trim()));
        }
        wnd.target_path = out_path.to_string_lossy().to_string();
        self.active_popup_target_path = wnd.target_path.clone();
        self.active_popup_filename = wnd.filename.clone();
        self.active_popup_url = url.clone();
        opt.output_path = Some(out_path);

        if let Some(c) = self.add_cookies.take() {
            opt.cookie = Some(c);
        }
        if let Some(r) = self.add_referer.take() {
            opt.referer = Some(r);
        }
        if let Some(ua) = self.add_user_agent.take() {
            opt.user_agent = Some(ua);
        }

        let q = Arc::clone(&self.queue);
        let speed_bps = wnd.speed_limit_kb * 1024;
        let tx = self.window_task_tx.clone();

        tokio::spawn(async move {
            let res = if start_paused {
                if let Ok(id) = q.add_download(opt, chosen_cat, speed_bps).await {
                    let _ = q.pause(&id).await;
                    Ok(id)
                } else {
                    Err(())
                }
            } else {
                q.start_download_immediately(opt, chosen_cat, speed_bps).await.map_err(|_| ())
            };

            if let Ok(id) = res {
                let _ = tx.send((window_idx, id)).await;
            }
        });

        if start_paused {
            wnd.stage = DownloadWindowStage::Paused;
            platform::set_taskbar_progress(platform::TBPF_PAUSED, 0, 100);
        } else {
            wnd.stage = DownloadWindowStage::Downloading;
            platform::set_taskbar_progress(platform::TBPF_NORMAL, 0, 100);
        }
        wnd.is_open = true;

        let fname = wnd.filename.clone();
        platform::show_desktop_notification("Cosmo Downloader", &format!("Download started: {}", fname));
        true
    }

    pub fn handle_download_window_outcome(
        &mut self,
        window_idx: usize,
        outcome: DownloadWindowOutcome,
        ctx: &egui::Context,
    ) {
        if window_idx >= self.download_windows.len() {
            return;
        }
        let dialog_id = self.download_windows[window_idx].dialog_id.clone();
        self.handle_download_window_outcome_by_id(&dialog_id, outcome, ctx);
    }

    pub fn handle_download_window_outcome_by_id(
        &mut self,
        dialog_id: &str,
        outcome: DownloadWindowOutcome,
        ctx: &egui::Context,
    ) {
        let Some(window_idx) = self.download_windows.iter().position(|w| w.dialog_id == dialog_id) else {
            return;
        };
        match outcome {
            DownloadWindowOutcome::None => {}
            DownloadWindowOutcome::Minimize => {
                let target_did = dialog_id.to_string();
                if self.is_popup_mode && self.download_windows.first().map(|w| w.dialog_id.as_str()) == Some(dialog_id) {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(true));
                } else {
                    let vp_id = egui::ViewportId::from_hash_of(format!("dl_window_{}", target_did));
                    ctx.send_viewport_cmd_to(vp_id, egui::ViewportCommand::Minimized(true));
                }
                #[cfg(windows)]
                platform::windows::minimize_dialog_window();
            }
            DownloadWindowOutcome::StartDownload => {
                platform::log_daemon(&format!("STEP: Start pressed: dialog_id='{}'", dialog_id));
                let key = rustget_core::youtube::normalize_url_key(&self.download_windows[window_idx].url);
                if !key.is_empty() {
                    self.download_started_at.insert(key, Instant::now());
                }
                self.trigger_download_for_window(window_idx, false);
                if self.setting_open_main_on_download_start {
                    self.download_windows[window_idx].is_open = false;
                    let vp_id = egui::ViewportId::from_hash_of(format!("dl_window_{}", dialog_id));
                    ctx.send_viewport_cmd_to(vp_id, egui::ViewportCommand::Close);
                    self.show_main_window(MainWindowReason::SettingOpenMainOnDownloadStart, ctx);
                }
            }
            DownloadWindowOutcome::DownloadLater => {
                self.trigger_download_for_window(window_idx, true);
                if self.setting_open_main_on_download_start {
                    self.download_windows[window_idx].is_open = false;
                    let vp_id = egui::ViewportId::from_hash_of(format!("dl_window_{}", dialog_id));
                    ctx.send_viewport_cmd_to(vp_id, egui::ViewportCommand::Close);
                    self.show_main_window(MainWindowReason::SettingOpenMainOnDownloadStart, ctx);
                }
            }
            DownloadWindowOutcome::UrlChanged => {
                let cat_opt = {
                    let wnd = &mut self.download_windows[window_idx];
                    wnd.error_message = None;
                    Category::from_name(&wnd.category)
                };
                if let Some(c) = cat_opt {
                    let dir = self.resolve_save_dir_for_category(c);
                    self.download_windows[window_idx].save_dir = dir;
                }
                self.spawn_size_probe();
            }
            DownloadWindowOutcome::CategoryChanged => {
                let cat_opt = Category::from_name(&self.download_windows[window_idx].category);
                if let Some(c) = cat_opt {
                    let dir = self.resolve_save_dir_for_category(c);
                    self.download_windows[window_idx].save_dir = dir;
                }
            }
            DownloadWindowOutcome::OpenSettings => {
                self.open_options_dialog();
            }
            DownloadWindowOutcome::Pause => {
                let wnd = &mut self.download_windows[window_idx];
                wnd.stage = DownloadWindowStage::Paused;
                let tid = wnd.task_id.clone();
                let tid_task = tid.clone();
                let q = Arc::clone(&self.queue);
                tokio::spawn(async move {
                    let _ = q.pause(&tid_task).await;
                });
                platform::set_taskbar_progress(platform::TBPF_PAUSED, wnd.disp_downloaded, wnd.disp_total.unwrap_or(0));
                if let Some(d) = self.downloads.iter_mut().find(|d| d.id == tid) {
                    d.status = "paused".to_string();
                }
            }
            DownloadWindowOutcome::Resume => {
                let wnd = &mut self.download_windows[window_idx];
                wnd.stage = DownloadWindowStage::Downloading;
                let tid = wnd.task_id.clone();
                let tid_task = tid.clone();
                let q = Arc::clone(&self.queue);
                tokio::spawn(async move {
                    let _ = q.resume(&tid_task).await;
                });
                platform::set_taskbar_progress(platform::TBPF_NORMAL, wnd.disp_downloaded, wnd.disp_total.unwrap_or(0));
                if let Some(d) = self.downloads.iter_mut().find(|d| d.id == tid) {
                    d.status = "downloading".to_string();
                }
            }
            DownloadWindowOutcome::RunInBackground => {
                let target_did = dialog_id.to_string();
                let _url = self.download_windows[window_idx].url.clone();
                platform::log_daemon(&format!("STEP: dialog closed and disposed (to background): dialog_id='{}'", target_did));
                let vp_id = egui::ViewportId::from_hash_of(format!("dl_window_{}", target_did));
                ctx.send_viewport_cmd_to(vp_id, egui::ViewportCommand::Close);
                self.download_windows.retain(|w| w.dialog_id != target_did);
                self.check_pending_dialog_queue(ctx);
                let any_open = !self.download_windows.is_empty();
                if !any_open {
                    if self.is_main_window_open.load(Ordering::Relaxed) && !self.is_popup_mode {
                        // Main window remains open
                    } else {
                        let has_active_downloads = self.downloads.iter().any(|d| d.status == "downloading" || d.status == "queued")
                            || !self.active_progress.is_empty();
                        if !has_active_downloads {
                            if self.setting_minimize_to_tray && self.has_tray_icon {
                                self.is_popup_mode = false;
                                self.is_main_window_open.store(false, Ordering::Relaxed);
                                platform::hide_main_window();
                                ctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
                            } else {
                                self.perform_graceful_shutdown();
                            }
                        } else if self.setting_minimize_to_tray && self.has_tray_icon {
                            self.is_popup_mode = false;
                            self.is_main_window_open.store(false, Ordering::Relaxed);
                            platform::hide_main_window();
                            ctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
                        }
                    }
                }
            }
            DownloadWindowOutcome::CancelClicked => {
                let wnd = &mut self.download_windows[window_idx];
                wnd.confirming_cancel = true;
            }
            DownloadWindowOutcome::DismissCancel => {
                let wnd = &mut self.download_windows[window_idx];
                wnd.confirming_cancel = false;
            }
            DownloadWindowOutcome::ConfirmCancel => {
                let target_did = dialog_id.to_string();
                let (tid, target_p, url) = {
                    let wnd = &self.download_windows[window_idx];
                    (wnd.task_id.clone(), wnd.target_path.clone(), wnd.url.clone())
                };
                platform::log_daemon(&format!("STEP: dialog closed and disposed (ConfirmCancel): dialog_id='{}'", target_did));
                self.clear_dedupe_keys_for_url(&url);
                let vp_id = egui::ViewportId::from_hash_of(format!("dl_window_{}", target_did));
                ctx.send_viewport_cmd_to(vp_id, egui::ViewportCommand::Close);
                self.download_windows.retain(|w| w.dialog_id != target_did);
                self.check_pending_dialog_queue(ctx);
                let tid_task = tid.clone();
                let q = Arc::clone(&self.queue);
                let db = self.queue.db().clone();
                tokio::spawn(async move {
                    let _ = q.remove(&tid_task, false).await;
                    let _ = db.update_status(&tid_task, "Canceled", None).await;
                    let part1 = format!("{}.cosmo.part", target_p);
                    let part2 = format!("{}.part", target_p);
                    let _ = std::fs::remove_file(&part1);
                    let _ = std::fs::remove_file(&part2);
                    let _ = std::fs::remove_file(&target_p);
                });
                if let Some(d) = self.downloads.iter_mut().find(|d| d.id == tid) {
                    d.status = "Canceled".to_string();
                }
                platform::set_taskbar_progress(platform::TBPF_NOPROGRESS, 0, 0);

                let any_open = !self.download_windows.is_empty();
                if !any_open {
                    if self.is_main_window_open.load(Ordering::Relaxed) && !self.is_popup_mode {
                        // Main window remains open
                    } else {
                        let has_active_downloads = self.downloads.iter().any(|d| d.status == "downloading" || d.status == "queued")
                            || !self.active_progress.is_empty();
                        if !has_active_downloads {
                            if self.setting_minimize_to_tray && self.has_tray_icon {
                                self.is_popup_mode = false;
                                self.is_main_window_open.store(false, Ordering::Relaxed);
                                platform::hide_main_window();
                                ctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
                            } else {
                                self.perform_graceful_shutdown();
                            }
                        } else if self.setting_minimize_to_tray && self.has_tray_icon {
                            self.is_popup_mode = false;
                            self.is_main_window_open.store(false, Ordering::Relaxed);
                            platform::hide_main_window();
                            ctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
                        }
                    }
                }
            }
            DownloadWindowOutcome::OpenFile => {
                let wnd = &self.download_windows[window_idx];
                platform::shell_open_file(&wnd.target_path);
            }
            DownloadWindowOutcome::ShowInFolder => {
                let wnd = &self.download_windows[window_idx];
                platform::shell_open_folder_and_select(&wnd.target_path);
            }
            DownloadWindowOutcome::Retry => {
                let wnd = &mut self.download_windows[window_idx];
                wnd.stage = DownloadWindowStage::Downloading;
                let tid = wnd.task_id.clone();
                let q = Arc::clone(&self.queue);
                tokio::spawn(async move {
                    let _ = q.resume(&tid).await;
                });
                platform::set_taskbar_progress(platform::TBPF_NORMAL, wnd.disp_downloaded, wnd.disp_total.unwrap_or(0));
            }
            DownloadWindowOutcome::Close => {
                let target_did = dialog_id.to_string();
                let url = self.download_windows[window_idx].url.clone();
                platform::log_daemon(&format!("STEP: dialog closed and disposed: dialog_id='{}'", target_did));
                self.clear_dedupe_keys_for_url(&url);
                let vp_id = egui::ViewportId::from_hash_of(format!("dl_window_{}", target_did));
                ctx.send_viewport_cmd_to(vp_id, egui::ViewportCommand::Close);
                self.download_windows.retain(|w| w.dialog_id != target_did);
                self.check_pending_dialog_queue(ctx);
                platform::set_taskbar_progress(platform::TBPF_NOPROGRESS, 0, 0);

                let any_open = !self.download_windows.is_empty();
                if !any_open {
                    if self.is_main_window_open.load(Ordering::Relaxed) && !self.is_popup_mode {
                        // Main window remains open
                    } else {
                        let has_active_downloads = self.downloads.iter().any(|d| d.status == "downloading" || d.status == "queued")
                            || !self.active_progress.is_empty();
                        if !has_active_downloads {
                            if self.setting_minimize_to_tray && self.has_tray_icon {
                                self.is_popup_mode = false;
                                self.is_main_window_open.store(false, Ordering::Relaxed);
                                platform::hide_main_window();
                                ctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
                            } else {
                                self.perform_graceful_shutdown();
                            }
                        } else if self.setting_minimize_to_tray && self.has_tray_icon {
                            self.is_popup_mode = false;
                            self.is_main_window_open.store(false, Ordering::Relaxed);
                            platform::hide_main_window();
                            ctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
                        }
                    }
                }
            }
        }
    }

    pub fn trigger_download(&mut self, start_paused: bool) -> bool {
        if !self.download_windows.is_empty() {
            let last_idx = self.download_windows.len() - 1;
            return self.trigger_download_for_window(last_idx, start_paused);
        }
        false
    }

    fn toggle_theme(&mut self, ctx: &egui::Context) {
        self.is_dark = !self.is_dark;
        theme::configure_visuals(ctx, self.is_dark);
        let db = self.queue.db().clone();
        let val = if self.is_dark { "dark" } else { "light" };
        tokio::spawn(async move {
            let _ = db.set_setting("ui_theme", val).await;
        });
    }

    fn restore_main_window_chrome(&self, ctx: &egui::Context) {
        platform::remove_popup_topmost();
        ctx.send_viewport_cmd(egui::ViewportCommand::WindowLevel(egui::WindowLevel::Normal));
        ctx.send_viewport_cmd(egui::ViewportCommand::Decorations(false));
        ctx.send_viewport_cmd(egui::ViewportCommand::Resizable(true));
        ctx.send_viewport_cmd(egui::ViewportCommand::MinInnerSize(Vec2::new(760.0, 480.0)));
        ctx.send_viewport_cmd(egui::ViewportCommand::MaxInnerSize(Vec2::new(10000.0, 10000.0)));
        ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(Vec2::new(1000.0, 640.0)));
        ctx.send_viewport_cmd(egui::ViewportCommand::Title(APP_TITLE.to_string()));
    }

    fn resume_selected(&self) {
        let q = Arc::clone(&self.queue);
        let ids: Vec<String> = self
            .downloads
            .iter()
            .filter(|d| self.selected_ids.contains(&d.id) && (d.status == "paused" || d.status == "failed"))
            .map(|d| d.id.clone())
            .collect();
        tokio::spawn(async move {
            for id in ids {
                let _ = q.resume(&id).await;
            }
        });
    }

    fn pause_selected(&self) {
        let q = Arc::clone(&self.queue);
        let ids: Vec<String> = self
            .downloads
            .iter()
            .filter(|d| self.selected_ids.contains(&d.id) && (d.status == "downloading" || d.status == "queued"))
            .map(|d| d.id.clone())
            .collect();
        tokio::spawn(async move {
            for id in ids {
                let _ = q.pause(&id).await;
            }
        });
    }

    fn request_delete_selected(&mut self) {
        if self.selected_ids.is_empty() {
            return;
        }
        self.pending_delete_ids = self.selected_ids.iter().cloned().collect();
        self.delete_also_files = false;
    }

    fn execute_pending_delete(&mut self) {
        let q = Arc::clone(&self.queue);
        let ids = std::mem::take(&mut self.pending_delete_ids);
        if ids.is_empty() {
            return;
        }
        let also_files = self.delete_also_files;
        for id in &ids {
            self.selected_ids.remove(id);
        }
        tokio::spawn(async move {
            for id in ids {
                let _ = q.remove(&id, also_files).await;
            }
        });
    }

    fn resume_all(&self) {
        let q = Arc::clone(&self.queue);
        let ids: Vec<String> = self
            .downloads
            .iter()
            .filter(|d| d.status == "paused" || d.status == "failed")
            .map(|d| d.id.clone())
            .collect();
        tokio::spawn(async move {
            for id in ids {
                let _ = q.resume(&id).await;
            }
        });
    }

    fn pause_all(&self) {
        let q = Arc::clone(&self.queue);
        let ids: Vec<String> = self
            .downloads
            .iter()
            .filter(|d| d.status == "downloading" || d.status == "queued")
            .map(|d| d.id.clone())
            .collect();
        tokio::spawn(async move {
            for id in ids {
                let _ = q.pause(&id).await;
            }
        });
    }

    fn handle_borderless_resize(&self, ctx: &egui::Context) {
        let is_maximized = ctx.input(|i| i.viewport().maximized.unwrap_or(false));
        if is_maximized {
            return;
        }
        let Some(rect) = ctx.input(|i| i.viewport().inner_rect) else {
            return;
        };
        let Some(pos) = ctx.input(|i| i.pointer.hover_pos()) else {
            return;
        };

        let edge = 5.0;
        let left = pos.x <= edge;
        let right = pos.x >= rect.width() - edge;
        let top = pos.y <= edge;
        let bottom = pos.y >= rect.height() - edge;

        let dir = match (left, right, top, bottom) {
            (true, false, true, false) => Some(egui::ResizeDirection::NorthWest),
            (false, true, true, false) => Some(egui::ResizeDirection::NorthEast),
            (true, false, false, true) => Some(egui::ResizeDirection::SouthWest),
            (false, true, false, true) => Some(egui::ResizeDirection::SouthEast),
            (true, false, false, false) => Some(egui::ResizeDirection::West),
            (false, true, false, false) => Some(egui::ResizeDirection::East),
            (false, false, true, false) => Some(egui::ResizeDirection::North),
            (false, false, false, true) => Some(egui::ResizeDirection::South),
            _ => None,
        };

        if let Some(d) = dir {
            let icon = match d {
                egui::ResizeDirection::North | egui::ResizeDirection::South => egui::CursorIcon::ResizeVertical,
                egui::ResizeDirection::East | egui::ResizeDirection::West => egui::CursorIcon::ResizeHorizontal,
                egui::ResizeDirection::NorthWest | egui::ResizeDirection::SouthEast => egui::CursorIcon::ResizeNwSe,
                egui::ResizeDirection::NorthEast | egui::ResizeDirection::SouthWest => egui::CursorIcon::ResizeNeSw,
            };
            ctx.set_cursor_icon(icon);
            if ctx.input(|i| i.pointer.primary_pressed()) {
                ctx.send_viewport_cmd(egui::ViewportCommand::BeginResize(d));
            }
        }
    }

    pub fn init_options_dialog_state(&mut self) {
        let speed_unit = if self.setting_global_speed_kb % 1024 == 0 && self.setting_global_speed_kb > 0 {
            crate::dialogs::settings::SpeedUnit::Mbps
        } else {
            crate::dialogs::settings::SpeedUnit::Kbps
        };
        let speed_val = if speed_unit == crate::dialogs::settings::SpeedUnit::Mbps {
            self.setting_global_speed_kb / 1024
        } else {
            self.setting_global_speed_kb
        };

        self.options_dialog_state = Some(OptionsDialogState {
            active_tab: OptionsTab::General,
            startup: self.setting_autostart,
            minstart: self.setting_start_minimized,
            tray: self.setting_show_tray_icon,
            mintray: self.setting_minimize_to_tray,
            threads: self.setting_default_connections,
            speed: speed_val,
            unit: speed_unit,
            folder: self.setting_default_save_dir.clone(),
            clip: self.clipboard_enabled.load(Ordering::Relaxed),
            notif: self.setting_notifications_enabled,
            cwin: !self.setting_dont_show_complete_dialog,
            openmain: self.setting_open_main_on_download_start,
            proxy: self.setting_proxy_url.clone(),
        });
    }

    pub fn open_options_dialog(&mut self) {
        self.init_options_dialog_state();
        self.show_settings_dialog = true;
    }

    pub fn show_main_window(&mut self, reason: MainWindowReason, ctx: &egui::Context) -> bool {
        platform::logger::log_event(Some("WINDOW"), &format!("show_main_window requested: reason={:?}", reason));
        match reason {
            MainWindowReason::UserLaunchedManually
            | MainWindowReason::TrayLeftClick
            | MainWindowReason::TrayMenuOpen
            | MainWindowReason::SecondInstanceLaunchedByUser
            | MainWindowReason::UserClickedOpenMainWindow => {
                // Allowed
            }
            MainWindowReason::SettingOpenMainOnDownloadStart => {
                if !self.setting_open_main_on_download_start {
                    platform::logger::log_event(Some("WINDOW"), "show_main_window REJECTED: setting_open_main_on_download_start is false");
                    return false;
                }
            }
            MainWindowReason::Forbidden(r) => {
                platform::logger::log_event(Some("WINDOW"), &format!("show_main_window REJECTED: forbidden reason {}", r));
                #[cfg(debug_assertions)]
                {
                    panic!("Forbidden attempt to show main window: reason={}", r);
                }
                #[cfg(not(debug_assertions))]
                {
                    return false;
                }
            }
        }
        self.is_main_window_open.store(true, Ordering::SeqCst);
        self.is_popup_mode = false;
        self.restore_main_window_chrome(ctx);

        // When main window is opened, dismiss any popups so dashboard is completely unobstructed
        for w in &mut self.download_windows {
            w.is_open = false;
        }

        // Send to root viewport
        ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
        ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
        ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
        ctx.send_viewport_cmd(egui::ViewportCommand::RequestUserAttention(egui::UserAttentionType::Critical));

        let main_vp_id = egui::ViewportId::from_hash_of(MAIN_VIEWPORT_ID);
        ctx.send_viewport_cmd_to(main_vp_id, egui::ViewportCommand::Minimized(false));
        ctx.send_viewport_cmd_to(main_vp_id, egui::ViewportCommand::Visible(true));
        ctx.send_viewport_cmd_to(main_vp_id, egui::ViewportCommand::Focus);
        #[cfg(windows)]
        {
            platform::windows::grant_foreground_permission();
            platform::show_main_window();
        }
        true
    }

    pub fn build_diagnostic_info(&self, ctx: &egui::Context) -> String {
        let os_name = std::env::consts::OS;
        let os_arch = std::env::consts::ARCH;
        let version = env!("CARGO_PKG_VERSION");
        let ppp = ctx.pixels_per_point();
        let screen_rect = ctx.screen_rect();

        let mut out = String::new();
        out.push_str("=====================================================\n");
        out.push_str("COSMO DOWNLOADER DIAGNOSTIC REPORT\n");
        out.push_str("=====================================================\n");
        out.push_str(&format!("App Version: {}\n", version));
        out.push_str(&format!("Platform: {} {}\n", os_name, os_arch));
        out.push_str(&format!("Display Scaling: {:.2} (Screen: {:.0}x{:.0})\n", ppp, screen_rect.width(), screen_rect.height()));
        out.push_str("\n--- SETTINGS (Safe / Sanitized) ---\n");
        out.push_str(&format!("Theme: {}\n", if self.is_dark { "dark" } else { "light" }));
        out.push_str(&format!("Clipboard Monitor: {}\n", self.clipboard_enabled.load(Ordering::Relaxed)));
        out.push_str(&format!("Minimize to Tray: {}\n", self.setting_minimize_to_tray));
        out.push_str(&format!("Start Minimized: {}\n", self.setting_start_minimized));
        out.push_str(&format!("Open Main on Download Start: {}\n", self.setting_open_main_on_download_start));
        out.push_str(&format!("Don't Show Complete Dialog: {}\n", self.setting_dont_show_complete_dialog));
        out.push_str(&format!("Default Num Segments: {}\n", self.setting_default_connections));
        out.push_str(&format!("Proxy Configured: {}\n", if self.setting_proxy_url.is_empty() { "None" } else { "Yes (Redacted)" }));

        out.push_str("\n--- OPEN WINDOWS ---\n");
        out.push_str(&format!("Main Dashboard Window Open: {}\n", self.is_main_window_open.load(Ordering::Relaxed)));
        out.push_str(&format!("Download Windows Count: {}\n", self.download_windows.len()));
        for (i, wnd) in self.download_windows.iter().enumerate() {
            let stage_str = match &wnd.stage {
                crate::dialogs::download_window::DownloadWindowStage::Info => "Info".to_string(),
                crate::dialogs::download_window::DownloadWindowStage::Connecting => "Connecting".to_string(),
                crate::dialogs::download_window::DownloadWindowStage::Downloading => "Downloading".to_string(),
                crate::dialogs::download_window::DownloadWindowStage::Paused => "Paused".to_string(),
                crate::dialogs::download_window::DownloadWindowStage::Processing => "Processing".to_string(),
                crate::dialogs::download_window::DownloadWindowStage::Complete => "Complete".to_string(),
                crate::dialogs::download_window::DownloadWindowStage::Failed { reason } => format!("Failed ({})", reason),
            };
            out.push_str(&format!(
                "  [{}] dialog_id='{}' is_open={} stage={} filename='{}' downloaded={}/{} pos=({:.0},{:.0})\n",
                i,
                wnd.dialog_id,
                wnd.is_open,
                stage_str,
                wnd.filename,
                wnd.disp_downloaded,
                wnd.disp_total.map(|t| t.to_string()).unwrap_or_else(|| "unknown".to_string()),
                wnd.window_pos.map(|p| p.x).unwrap_or(0.0),
                wnd.window_pos.map(|p| p.y).unwrap_or(0.0),
            ));
        }

        out.push_str("\n=====================================================\n");
        out.push_str("RECENT EVENT LOGS (Last 200 lines):\n");
        out.push_str("=====================================================\n");
        let recent = platform::logger::get_recent_logs(200);
        out.push_str(&recent.concat());

        out
    }
}

impl eframe::App for CosmoDownloaderApp {
    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        let pal = crate::theme::palette(self.is_dark);
        [
            pal.surface.r() as f32 / 255.0,
            pal.surface.g() as f32 / 255.0,
            pal.surface.b() as f32 / 255.0,
            1.0,
        ]
    }

    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if let Some(scale) = self.custom_scale {
            ctx.set_pixels_per_point(scale);
        }

        ctx.input(|i| {
            for event in &i.raw.events {
                if let egui::Event::Screenshot { image, .. } = event {
                    if let Some(ref path) = self.screenshot_path {
                        let w = image.width() as u32;
                        let h = image.height() as u32;
                        let mut pixels = Vec::with_capacity(image.pixels.len() * 4);
                        for p in &image.pixels {
                            pixels.push(p.r());
                            pixels.push(p.g());
                            pixels.push(p.b());
                            pixels.push(p.a());
                        }
                        if let Err(e) = image::save_buffer(
                            path,
                            &pixels,
                            w,
                            h,
                            image::ExtendedColorType::Rgba8,
                        ) {
                            eprintln!("Failed to save screenshot to '{}': {}", path, e);
                        } else {
                            println!("Successfully saved screenshot to '{}'", path);
                        }
                        std::process::exit(0);
                    }
                }
            }
        });

        if self.screenshot_path.is_some() {
            let wait_for_popup = std::env::args().any(|a| {
                let clean = a.trim_matches('"').trim_matches('\'');
                clean == "--wait-popup"
            });
            if !wait_for_popup || self.is_popup_mode {
                self.frame_counter += 1;
                if self.frame_counter >= 5 {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot);
                }
            }
            ctx.request_repaint();
        }

        #[cfg(windows)]
        {
            use raw_window_handle::HasWindowHandle;
            if let Ok(handle) = _frame.window_handle() {
                if let raw_window_handle::RawWindowHandle::Win32(win32) = handle.as_raw() {
                    let hwnd = win32.hwnd.get() as *mut std::ffi::c_void;
                    platform::windows::set_main_dashboard_window(hwnd);
                }
            }
            platform::windows::init_shutdown_listener(ctx);
        }

        let external_quit = self.quit_rx.try_recv().is_ok()
            || {
                #[cfg(windows)]
                { platform::windows::is_external_shutdown_requested() }
                #[cfg(not(windows))]
                { false }
            };
        if external_quit {
            self.perform_graceful_shutdown();
        }

        if !self.initial_minimized_handled {
            self.initial_minimized_handled = true;
            let start_minimized = std::env::args().any(|a| a == "--minimized" || a == "--daemon");
            if start_minimized {
                ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(true));
                if self.setting_minimize_to_tray && self.has_tray_icon {
                    platform::hide_main_window();
                    ctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
                    self.is_main_window_open.store(false, Ordering::Relaxed);
                }
            }
        }

        let has_active_work = !self.active_progress.is_empty()
            || self.add_is_probing
            || self.downloads.iter().any(|d| d.status == "downloading" || d.status == "queued");
        if has_active_work {
            ctx.request_repaint_after(Duration::from_millis(100));
        } else {
            ctx.request_repaint_after(Duration::from_millis(1000));
        }

        let pal = theme::palette(self.is_dark);

        // Window close handler (honor OS/installer quit first, otherwise tray only if tray icon exists)
        if ctx.input(|i| i.viewport().close_requested()) {
            #[cfg(windows)]
            if platform::windows::is_external_shutdown_requested() {
                self.perform_graceful_shutdown();
            }

            if self.is_popup_mode && !self.download_windows.is_empty() {
                ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
                let wnd = &mut self.download_windows[0];
                match wnd.stage {
                    DownloadWindowStage::Info => {
                        self.handle_download_window_outcome(0, DownloadWindowOutcome::Close, ctx);
                    }
                    DownloadWindowStage::Connecting | DownloadWindowStage::Downloading | DownloadWindowStage::Paused | DownloadWindowStage::Processing => {
                        self.handle_download_window_outcome(0, DownloadWindowOutcome::RunInBackground, ctx);
                    }
                    DownloadWindowStage::Complete | DownloadWindowStage::Failed { .. } => {
                        self.handle_download_window_outcome(0, DownloadWindowOutcome::Close, ctx);
                    }
                }
                return;
            }

            if self.setting_minimize_to_tray && self.has_tray_icon {
                platform::log_daemon("Main window close requested -> minimizing to tray");
                ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
                platform::remove_popup_topmost();
                ctx.send_viewport_cmd(egui::ViewportCommand::WindowLevel(egui::WindowLevel::Normal));
                platform::hide_main_window();
                ctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
                self.is_main_window_open.store(false, Ordering::Relaxed);
                return;
            } else {
                self.perform_graceful_shutdown();
            }
        }

        // Receive task ID from window launcher
        while let Ok((idx, id)) = self.window_task_rx.try_recv() {
            if idx < self.download_windows.len() {
                self.download_windows[idx].task_id = id.clone();
            }
            self.active_popup_task_id = Some(id);
        }

        // Receive task ID
        if let Some(ref mut rx) = self.pending_task_rx {
            if let Ok(id) = rx.try_recv() {
                self.active_popup_task_id = Some(id);
            }
        }

        // Size probe receiver
        let mut clear_probe = false;
        if let Some(ref mut rx) = self.size_probe_rx {
            match rx.try_recv() {
                Ok((sz, suggested_name)) => {
                    self.add_is_probing = false;
                    self.add_file_size = sz;
                    if let Some(name) = suggested_name {
                        let cur_name = self.add_filename.trim().to_ascii_lowercase();
                        if cur_name == "download"
                            || cur_name == "download.bin"
                            || !self.add_filename.contains('.')
                            || (name.contains('.') && !cur_name.contains('.'))
                        {
                            self.add_filename = name.clone();
                            let cat = categorize(Some(&name), None);
                            self.add_category = cat.name().to_string();
                            self.add_save_dir = self.resolve_save_dir_for_category(cat);
                        }
                    }
                    if let Some(last_wnd) = self.download_windows.iter_mut().rev().find(|w| w.stage == DownloadWindowStage::Info) {
                        last_wnd.file_size = sz;
                        last_wnd.is_probing = false;
                        if !self.add_filename.is_empty() {
                            last_wnd.filename = self.add_filename.clone();
                            last_wnd.category = self.add_category.clone();
                            last_wnd.save_dir = self.add_save_dir.clone();
                        }
                    }
                    clear_probe = true;
                }
                Err(tokio::sync::mpsc::error::TryRecvError::Disconnected) => {
                    self.add_is_probing = false;
                    if let Some(last_wnd) = self.download_windows.iter_mut().rev().find(|w| w.stage == DownloadWindowStage::Info) {
                        last_wnd.is_probing = false;
                    }
                    clear_probe = true;
                }
                Err(tokio::sync::mpsc::error::TryRecvError::Empty) => {}
            }
        }
        if clear_probe {
            self.size_probe_rx = None;
        }

        // Event processing
        while let Ok(event) = self.event_rx.try_recv() {
            match event {
                QueueEvent::TaskCompleted(id) => {
                    self.refresh_downloads();
                    let completed_url = self.downloads.iter().find(|d| d.id == id).map(|d| d.url.clone());
                    if let Some(ref u) = completed_url {
                        self.clear_dedupe_keys_for_url(u);
                    }
                    if self.setting_notifications_enabled {
                        let fname = self
                            .downloads
                            .iter()
                            .find(|d| d.id == id)
                            .map(|d| d.filename.clone())
                            .unwrap_or_else(|| self.active_popup_filename.clone());
                        platform::show_desktop_notification(
                            "Download complete",
                            &format!("{} finished downloading.", fname),
                        );
                    }
                    let mut matched = false;
                    for wnd in &mut self.download_windows {
                        if wnd.task_id == id || (wnd.task_id.is_empty() && (wnd.stage == DownloadWindowStage::Downloading || wnd.stage == DownloadWindowStage::Paused)) {
                            matched = true;
                            wnd.task_id = id.clone();
                            wnd.stage = DownloadWindowStage::Complete;
                            if let Some(tot) = wnd.total_bytes {
                                wnd.downloaded_bytes = tot;
                                wnd.disp_downloaded = tot;
                            }
                            platform::set_taskbar_progress(platform::TBPF_NOPROGRESS, 0, 0);
                            if !self.setting_dont_show_complete_dialog {
                                wnd.is_open = true;
                                let vp_id = egui::ViewportId::from_hash_of(format!("dl_window_{}", wnd.dialog_id));
                                ctx.send_viewport_cmd_to(vp_id, egui::ViewportCommand::Visible(true));
                                ctx.send_viewport_cmd_to(vp_id, egui::ViewportCommand::Minimized(false));
                                ctx.send_viewport_cmd_to(vp_id, egui::ViewportCommand::Focus);
                                if self.is_popup_mode {
                                    ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
                                    ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
                                    ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
                                }
                            } else {
                                wnd.is_open = false;
                                let vp_id = egui::ViewportId::from_hash_of(format!("dl_window_{}", wnd.dialog_id));
                                ctx.send_viewport_cmd_to(vp_id, egui::ViewportCommand::Close);
                            }
                            break;
                        }
                    }
                    if !matched && !self.setting_dont_show_complete_dialog && self.is_popup_mode {
                        if let Some(rec) = self.downloads.iter().find(|d| d.id == id).cloned() {
                            let mut wnd = DownloadWindowState::new(rec.url, rec.filename, rec.category, rec.target_path.clone());
                            wnd.task_id = id.clone();
                            wnd.target_path = rec.target_path;
                            wnd.downloaded_bytes = rec.downloaded_bytes;
                            wnd.total_bytes = rec.total_size;
                            wnd.disp_downloaded = rec.downloaded_bytes;
                            wnd.disp_total = rec.total_size;
                            wnd.stage = DownloadWindowStage::Complete;
                            wnd.is_open = true;
                            self.download_windows.push(wnd);
                            ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
                            ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
                            ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
                        }
                    }
                }
                QueueEvent::TaskAdded(_) => {
                    self.refresh_downloads();
                }
                QueueEvent::TaskFailed { id, error } => {
                    self.refresh_downloads();
                    let failed_url = self.downloads.iter().find(|d| d.id == id).map(|d| d.url.clone());
                    if let Some(ref u) = failed_url {
                        self.clear_dedupe_keys_for_url(u);
                    }
                    if self.setting_notifications_enabled {
                        let fname = self
                            .downloads
                            .iter()
                            .find(|d| d.id == id)
                            .map(|d| d.filename.clone())
                            .unwrap_or_else(|| "Download".to_string());
                        platform::show_desktop_notification(
                            "Download failed",
                            &format!("{}: {}", fname, error),
                        );
                    }
                    for wnd in &mut self.download_windows {
                        if wnd.task_id == id || (wnd.task_id.is_empty() && wnd.stage == DownloadWindowStage::Downloading) {
                            wnd.task_id = id.clone();
                            wnd.stage = DownloadWindowStage::Failed { reason: error.clone() };
                            wnd.is_open = true;
                            platform::set_taskbar_progress(platform::TBPF_ERROR, wnd.disp_downloaded, wnd.disp_total.unwrap_or(0));
                            ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
                            ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
                            break;
                        }
                    }
                }
                QueueEvent::StatusChanged { id, status } => {
                    if status == "completed" || status == "paused" || status == "failed" {
                        self.active_progress.remove(&id);
                        self.ema_speeds.remove(&id);
                    }
                    for wnd in &mut self.download_windows {
                        if wnd.task_id == id {
                            if status == "paused" && wnd.stage == DownloadWindowStage::Downloading {
                                wnd.stage = DownloadWindowStage::Paused;
                                platform::set_taskbar_progress(platform::TBPF_PAUSED, wnd.disp_downloaded, wnd.disp_total.unwrap_or(0));
                            } else if (status == "downloading" || status == "resumed") && wnd.stage == DownloadWindowStage::Paused {
                                wnd.stage = DownloadWindowStage::Downloading;
                                platform::set_taskbar_progress(platform::TBPF_NORMAL, wnd.disp_downloaded, wnd.disp_total.unwrap_or(0));
                            } else if status == "completed" {
                                wnd.stage = DownloadWindowStage::Complete;
                                wnd.is_open = true;
                                platform::set_taskbar_progress(platform::TBPF_NOPROGRESS, 0, 0);
                            }
                        }
                    }
                    self.refresh_downloads();
                }
                QueueEvent::Progress { id, progress } => {
                    let current_speed = progress.speed_bps;
                    let smoothed = if let Some(&prev) = self.ema_speeds.get(&id) {
                        0.3 * current_speed + 0.7 * prev
                    } else {
                        current_speed
                    };
                    self.ema_speeds.insert(id.clone(), smoothed);
                    self.active_progress.insert(id.clone(), progress.clone());

                    for wnd in &mut self.download_windows {
                        if wnd.task_id == id || (wnd.task_id.is_empty() && wnd.stage == DownloadWindowStage::Downloading) {
                            if wnd.task_id.is_empty() {
                                wnd.task_id = id.clone();
                            }
                            wnd.update_progress(progress.downloaded_bytes, progress.total_bytes, std::time::Instant::now());
                            let flags = if wnd.stage == DownloadWindowStage::Paused {
                                platform::TBPF_PAUSED
                            } else {
                                platform::TBPF_NORMAL
                            };
                            platform::set_taskbar_progress(flags, wnd.disp_downloaded, wnd.disp_total.unwrap_or(0));
                            break;
                        }
                    }
                }
            }
        }

        // External show requests
        while self.show_rx.try_recv().is_ok() {
            platform::log_daemon("Received show request via bridge (/show)");
            self.show_main_window(MainWindowReason::SecondInstanceLaunchedByUser, ctx);
        }

        // Browser extension download requests
        while let Ok(req) = self.browser_dl_rx.try_recv() {
            platform::log_daemon(&format!("STEP: extension request received on UI thread: url='{}'", req.url));
            let dialog_req = DownloadDialogRequest {
                url: req.url,
                filename: req.filename,
                cookies: req.cookies,
                referer: req.referer,
                user_agent: req.user_agent,
                num_segments: req.num_segments,
            };
            self.show_download_dialog(dialog_req, "extension", Some(ctx));
        }

        // Clipboard events
        while let Ok(url) = self.clipboard_rx.try_recv() {
            if self.setting_clipboard_monitor {
                platform::log_daemon(&format!("Clipboard monitor detected URL: {}", url));
                let dialog_req = DownloadDialogRequest {
                    url,
                    filename: None,
                    cookies: None,
                    referer: None,
                    user_agent: None,
                    num_segments: None,
                };
                self.show_download_dialog(dialog_req, "clipboard", Some(ctx));
            }
        }

        // Tray actions
        let tray_actions: Vec<String> = if let Some(ref rx) = self.tray_event_receiver {
            let mut acts = Vec::new();
            while let Ok(action) = rx.try_recv() {
                acts.push(action);
            }
            acts
        } else {
            Vec::new()
        };
        for action in tray_actions {
            match action.as_str() {
                "tray_click" => {
                    platform::log_daemon("Tray action: tray click -> show main window");
                    self.show_main_window(MainWindowReason::TrayLeftClick, ctx);
                }
                "tray_menu_open" | "show" => {
                    platform::log_daemon("Tray action: tray menu open -> show main window");
                    self.show_main_window(MainWindowReason::TrayMenuOpen, ctx);
                }
                "pause_all" => {
                    platform::log_daemon("Tray action: pause all");
                    self.pause_all();
                }
                "resume_all" => {
                    platform::log_daemon("Tray action: resume all");
                    self.resume_all();
                }
                "add" => {
                    platform::log_daemon("Tray action: add URL");
                    self.show_download_dialog(DownloadDialogRequest::empty(), "tray", Some(ctx));
                }
                "exit" => {
                    self.perform_graceful_shutdown();
                }
                "options" => {
                    self.show_main_window(MainWindowReason::TrayMenuOpen, ctx);
                    self.open_options_dialog();
                }
                _ => {}
            }
        }

        // Standalone Popup File Info Mode (IDM-style unified window)
        if self.is_popup_mode {
            if std::env::args().any(|a| {
                let clean = a.trim_matches('"').trim_matches('\'');
                clean == "--view=cascade" || clean == "--view=cascade_canvas"
            }) {
                egui::CentralPanel::default()
                    .frame(egui::Frame::none().fill(pal.bg))
                    .show(ctx, |ui| {
                        if self.download_windows.len() >= 2 {
                            ui.push_id("cascade_card_0", |ui| {
                                crate::dialogs::download_window::render_download_window_card(
                                    ui,
                                    ctx,
                                    pal,
                                    self.logo_texture.as_ref(),
                                    &mut self.download_windows[0],
                                    Pos2::new(20.0, 20.0),
                                );
                            });
                            ui.push_id("cascade_card_1", |ui| {
                                crate::dialogs::download_window::render_download_window_card(
                                    ui,
                                    ctx,
                                    pal,
                                    self.logo_texture.as_ref(),
                                    &mut self.download_windows[1],
                                    Pos2::new(44.0, 44.0),
                                );
                            });
                        }
                    });
                return;
            }

            if self.download_windows.is_empty() {
                return;
            }

            let mut outcomes: Vec<(String, DownloadWindowOutcome)> = Vec::new();

            // The first active dialog renders in the root window
            let root_dialog_id = self.download_windows[0].dialog_id.clone();
            platform::log_daemon(&format!("STEP: dialog shown on main window: dialog_id='{}'", root_dialog_id));
            let outcome = {
                let render_result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    render_download_window_content(
                        ctx,
                        pal,
                        self.logo_texture.as_ref(),
                        &mut self.download_windows[0],
                    )
                }));
                match render_result {
                    Ok(out) => out,
                    Err(err) => {
                        platform::log_daemon(&format!("ERROR: Panic rendering main window dialog_id='{}': {:?}", root_dialog_id, err));
                        DownloadWindowOutcome::Close
                    }
                }
            };
            outcomes.push((root_dialog_id, outcome));

            // Render any additional open download windows in child viewports
            for idx in 1..self.download_windows.len() {
                if self.download_windows[idx].is_open {
                    let dialog_id = self.download_windows[idx].dialog_id.clone();
                    let vp_id = egui::ViewportId::from_hash_of(format!("dl_window_{}", dialog_id));
                    let mut vp_builder = egui::ViewportBuilder::default()
                        .with_title(DOWNLOAD_FILE_INFO_TITLE)
                        .with_inner_size(Vec2::new(
                            crate::dialogs::download_window::DOWNLOAD_WINDOW_WIDTH,
                            crate::dialogs::download_window::DOWNLOAD_WINDOW_HEIGHT,
                        ))
                        .with_min_inner_size(Vec2::new(
                            crate::dialogs::download_window::DOWNLOAD_WINDOW_WIDTH,
                            crate::dialogs::download_window::DOWNLOAD_WINDOW_HEIGHT,
                        ))
                        .with_max_inner_size(Vec2::new(
                            crate::dialogs::download_window::DOWNLOAD_WINDOW_WIDTH,
                            crate::dialogs::download_window::DOWNLOAD_WINDOW_HEIGHT,
                        ))
                        .with_transparent(false)
                        .with_resizable(false)
                        .with_decorations(false)
                        .with_minimize_button(true)
                        .with_close_button(true)
                        .with_taskbar(true)
                        .with_always_on_top();

                    if let Some(pos) = self.download_windows[idx].window_pos {
                        vp_builder = vp_builder.with_position(pos);
                    }

                    let mut child_outcome = DownloadWindowOutcome::None;
                    platform::log_daemon(&format!("STEP: dialog shown on child viewport: idx={}, dialog_id='{}'", idx, dialog_id));
                    ctx.show_viewport_immediate(vp_id, vp_builder, |child_ctx, _class| {
                        let render_result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                            render_download_window_content(
                                child_ctx,
                                pal,
                                self.logo_texture.as_ref(),
                                &mut self.download_windows[idx],
                            )
                        }));
                        match render_result {
                            Ok(out) => {
                                child_outcome = out;
                            }
                            Err(err) => {
                                platform::log_daemon(&format!("ERROR: Panic rendering child viewport idx={}: {:?}", idx, err));
                                egui::CentralPanel::default()
                                    .frame(egui::Frame::none().fill(pal.surface).inner_margin(Margin::same(24.0)))
                                    .show(child_ctx, |ui| {
                                        ui.vertical_centered(|ui| {
                                            ui.add_space(50.0);
                                            ui.heading("Couldn't open the download window");
                                            ui.add_space(16.0);
                                            ui.label("An error occurred while preparing this download dialog.");
                                            ui.add_space(24.0);
                                            if ui.button("Close").clicked() {
                                                child_outcome = DownloadWindowOutcome::Close;
                                            }
                                        });
                                    });
                            }
                        }
                    });
                    outcomes.push((dialog_id, child_outcome));
                }
            }

            for (did, out) in outcomes {
                self.handle_download_window_outcome_by_id(&did, out, ctx);
            }
            return;
        }

        // ====================================================================
        // Full Dashboard View (Matches User's HTML UI)
        // ====================================================================
        let any_dialog_open = self.download_windows.iter().any(|w| w.is_open && w.stage == DownloadWindowStage::Info);
        if !self.show_add_dialog && !self.show_settings_dialog && !any_dialog_open {
            self.handle_borderless_resize(ctx);
        }

        let prev_cat = self.selected_category;
        let status_counts = compute_status_counts(&self.downloads);
        let cat_counts = compute_category_counts(&self.downloads);

        let needle = self.search_query.trim().to_ascii_lowercase();
        let filtered_ids: Vec<String> = self
            .downloads
            .iter()
            .filter(|d| match self.selected_status {
                nav_rail::StatusFilter::All => true,
                nav_rail::StatusFilter::Downloading => d.status == "downloading" || d.status == "queued",
                nav_rail::StatusFilter::Completed => d.status == "completed",
                nav_rail::StatusFilter::Paused => d.status == "paused" || d.status == "failed",
            })
            .filter(|d| self.selected_category.matches(&d.category))
            .filter(|d| {
                needle.is_empty()
                    || d.filename.to_ascii_lowercase().contains(&needle)
                    || d.url.to_ascii_lowercase().contains(&needle)
            })
            .map(|d| d.id.clone())
            .collect();
        let filtered_count = filtered_ids.len();

        // Keyboard shortcuts for desktop ergonomics
        let modal_open = self.show_add_dialog
            || self.show_settings_dialog
            || self.show_about_dialog
            || !self.pending_delete_ids.is_empty()
            || self.download_windows.iter().any(|w| w.is_open && w.stage == DownloadWindowStage::Info);
        if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            if !self.pending_delete_ids.is_empty() {
                self.pending_delete_ids.clear();
            } else if let Some(last_open_idx) = self.download_windows.iter().rposition(|w| w.is_open && w.stage == DownloadWindowStage::Info) {
                let did = self.download_windows[last_open_idx].dialog_id.clone();
                let outcome = DownloadWindowOutcome::Close;
                self.handle_download_window_outcome_by_id(&did, outcome, ctx);
            } else if self.show_add_dialog {
                self.show_add_dialog = false;
                self.add_error_message = None;
            } else if self.show_settings_dialog {
                self.show_settings_dialog = false;
            } else if self.show_about_dialog {
                self.show_about_dialog = false;
            } else {
                self.selected_ids.clear();
            }
        } else if !modal_open && !ctx.wants_keyboard_input() {
            if ctx.input(|i| i.modifiers.ctrl && i.key_pressed(egui::Key::N)) {
                self.show_download_dialog(DownloadDialogRequest::empty(), "shortcut_ctrl_n", Some(ctx));
            } else if ctx.input(|i| i.modifiers.ctrl && i.key_pressed(egui::Key::A)) {
                for id in &filtered_ids {
                    self.selected_ids.insert(id.clone());
                }
            } else if ctx.input(|i| i.key_pressed(egui::Key::Delete)) {
                self.request_delete_selected();
            } else if ctx.input(|i| i.key_pressed(egui::Key::Space)) {
                let any_active = self.downloads.iter().any(|d| {
                    self.selected_ids.contains(&d.id)
                        && (d.status == "downloading" || d.status == "queued")
                });
                if any_active {
                    self.pause_selected();
                } else if !self.selected_ids.is_empty() {
                    self.resume_selected();
                }
            }
        }

        let total_speed: f64 = self.active_progress.values().map(|p| p.speed_bps).sum();

        // 1. Top Title Bar (.title)
        egui::TopBottomPanel::top("cosmo_title_bar")
            .exact_height(36.0)
            .frame(egui::Frame::none().fill(pal.raise).inner_margin(Margin::same(0.0)))
            .show(ctx, |ui| {
                let act = title_bar::render_title_bar(ui, ctx, pal, self.logo_texture.as_ref(), APP_TITLE);
                if act.minimize {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(true));
                }
                if act.toggle_maximize {
                    let is_max = ctx.input(|i| i.viewport().maximized.unwrap_or(false));
                    ctx.send_viewport_cmd(egui::ViewportCommand::Maximized(!is_max));
                }
                if act.close {
                    if self.setting_minimize_to_tray && self.has_tray_icon {
                        platform::log_daemon("Main window title bar close clicked -> minimizing to tray");
                        platform::hide_main_window();
                        ctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
                        self.is_main_window_open.store(false, Ordering::Relaxed);
                    } else {
                        self.perform_graceful_shutdown();
                    }
                }
            });

        // 2. Menu Bar (.menubar)
        egui::TopBottomPanel::top("cosmo_menu_bar")
            .exact_height(36.0)
            .frame(egui::Frame::none().fill(pal.surface).inner_margin(Margin::same(0.0)))
            .show(ctx, |ui| {
                let m_act = menu_bar::render_menu_bar(
                    ui,
                    pal,
                    !self.selected_ids.is_empty(),
                    self.setting_clipboard_monitor,
                    self.selected_category,
                    self.hover_menu_name.as_deref(),
                );
                if m_act.add_download {
                    self.show_download_dialog(DownloadDialogRequest::empty(), "menu_bar", Some(ctx));
                }
                if m_act.exit_app {
                    self.perform_graceful_shutdown();
                }
                if m_act.resume_selected {
                    self.resume_selected();
                }
                if m_act.pause_selected {
                    self.pause_selected();
                }
                if m_act.delete_selected {
                    self.request_delete_selected();
                }
                if m_act.resume_all {
                    self.resume_all();
                }
                if m_act.pause_all {
                    self.pause_all();
                }
                if m_act.select_all {
                    for id in &filtered_ids {
                        self.selected_ids.insert(id.clone());
                    }
                }
                if m_act.clear_selection {
                    self.selected_ids.clear();
                }
                if let Some(cat) = m_act.set_category {
                    self.selected_category = cat;
                }
                if m_act.toggle_theme {
                    self.toggle_theme(ctx);
                }
                if m_act.open_options {
                    self.open_options_dialog();
                }
                if m_act.toggle_clipboard {
                    self.setting_clipboard_monitor = !self.setting_clipboard_monitor;
                    self.clipboard_enabled.store(self.setting_clipboard_monitor, Ordering::Relaxed);
                    let db = self.queue.db().clone();
                    let val = if self.setting_clipboard_monitor { "true" } else { "false" };
                    tokio::spawn(async move {
                        let _ = db.set_setting("clipboard_monitor_enabled", val).await;
                    });
                }
                if m_act.open_downloads_dir {
                    let dir = Category::General.default_dir().to_string_lossy().to_string();
                    platform::shell_open_folder_and_select(&dir);
                }
                if m_act.open_about {
                    self.show_about_dialog = true;
                }
            });

        // 3. Toolbar (.toolbar)
        egui::TopBottomPanel::top("cosmo_toolbar")
            .exact_height(60.0)
            .frame(egui::Frame::none().fill(pal.surface).inner_margin(Margin::same(0.0)))
            .show(ctx, |ui| {
                let tb = toolbar::render_toolbar(
                    ui,
                    pal,
                    !self.selected_ids.is_empty(),
                    &mut self.search_query,
                );
                if tb.add_clicked {
                    self.show_download_dialog(DownloadDialogRequest::empty(), "toolbar", Some(ctx));
                }
                if tb.resume_clicked {
                    if self.selected_ids.is_empty() {
                        self.resume_all();
                    } else {
                        self.resume_selected();
                    }
                }
                if tb.pause_clicked {
                    if self.selected_ids.is_empty() {
                        self.pause_all();
                    } else {
                        self.pause_selected();
                    }
                }
                if tb.delete_clicked {
                    self.request_delete_selected();
                }
                if tb.options_clicked {
                    self.open_options_dialog();
                }
            });

        // 4. Bottom Status Bar (.status)
        egui::TopBottomPanel::bottom("cosmo_status_bar")
            .exact_height(28.0)
            .frame(egui::Frame::none().fill(pal.raise).inner_margin(Margin::same(0.0)))
            .show(ctx, |ui| {
                status_bar::render_status_bar(
                    ui,
                    pal,
                    filtered_count,
                    total_speed,
                    env!("CARGO_PKG_VERSION"),
                );
            });

        // 5. Left Sidebar (.side)
        let pal_refresh = theme::palette(self.is_dark);
        egui::SidePanel::left("cosmo_sidebar")
            .resizable(false)
            .exact_width(208.0)
            .frame(
                egui::Frame::none()
                    .fill(pal_refresh.bg)
                    .inner_margin(Margin::same(12.0)),
            )
            .show(ctx, |ui| {
                // Right 1px divider line
                let r = ui.max_rect();
                ui.painter().line_segment(
                    [egui::pos2(r.right() + 12.0, r.top() - 12.0), egui::pos2(r.right() + 12.0, r.bottom() + 12.0)],
                    Stroke::new(1.0_f32, pal_refresh.line),
                );
                ui.spacing_mut().scroll.bar_width = 4.0_f32;
                egui::ScrollArea::vertical()
                    .id_salt("sidebar_scroll")
                    .auto_shrink([false, false])
                    .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::VisibleWhenNeeded)
                    .show(ui, |ui| {
                        nav_rail::render_category_sidebar(
                            ui,
                            pal_refresh,
                            &mut self.selected_status,
                            &status_counts,
                            &mut self.selected_category,
                            &cat_counts,
                        );
                    });
            });

        if self.selected_category != prev_cat {
            let cat_str = match self.selected_category {
                CategoryFilter::All => "all",
                CategoryFilter::Video => "video",
                CategoryFilter::Music => "music",
                CategoryFilter::Documents => "documents",
                CategoryFilter::Compressed => "compressed",
                CategoryFilter::Programs => "programs",
                CategoryFilter::Other => "other",
            };
            let q = Arc::clone(&self.queue);
            tokio::spawn(async move {
                let _ = q.db().set_setting("ui_selected_category", cat_str).await;
            });
        }

        // 6. Central Downloads Table (.tbl)
        egui::CentralPanel::default()
            .frame(egui::Frame::none().fill(pal_refresh.surface).inner_margin(Margin::same(0.0)))
            .show(ctx, |ui| {
                let mut filtered_downloads: Vec<&DownloadRecord> = self
                    .downloads
                    .iter()
                    .filter(|d| match self.selected_status {
                        nav_rail::StatusFilter::All => true,
                        nav_rail::StatusFilter::Downloading => d.status == "downloading" || d.status == "queued",
                        nav_rail::StatusFilter::Completed => d.status == "completed",
                        nav_rail::StatusFilter::Paused => d.status == "paused" || d.status == "failed",
                    })
                    .filter(|d| self.selected_category.matches(&d.category))
                    .filter(|d| {
                        needle.is_empty()
                            || d.filename.to_ascii_lowercase().contains(&needle)
                            || d.url.to_ascii_lowercase().contains(&needle)
                    })
                    .collect();

                if self.sort_col != download_table::SortColumn::Added {
                    let sort_col = self.sort_col;
                    let asc = self.sort_asc;
                    let prog_map = &self.active_progress;
                    let ema_map = &self.ema_speeds;
                    filtered_downloads.sort_by(|a, b| {
                        let ord = match sort_col {
                            download_table::SortColumn::Added => a.created_at.cmp(&b.created_at),
                            download_table::SortColumn::Name => {
                                a.filename.to_ascii_lowercase().cmp(&b.filename.to_ascii_lowercase())
                            }
                            download_table::SortColumn::Size => {
                                let sa = a.total_size.unwrap_or(a.downloaded_bytes);
                                let sb = b.total_size.unwrap_or(b.downloaded_bytes);
                                sa.cmp(&sb)
                            }
                            download_table::SortColumn::Progress => {
                                let pct = |r: &DownloadRecord| -> u64 {
                                    let dl = prog_map.get(&r.id).map(|p| p.downloaded_bytes).unwrap_or(r.downloaded_bytes);
                                    let tot = prog_map.get(&r.id).and_then(|p| p.total_bytes).or(r.total_size);
                                    match tot {
                                        Some(t) if t > 0 => (dl.saturating_mul(10_000)) / t,
                                        _ if r.status == "completed" => 10_000,
                                        _ => 0,
                                    }
                                };
                                pct(a).cmp(&pct(b))
                            }
                            download_table::SortColumn::Speed => {
                                let sp = |r: &DownloadRecord| -> f64 {
                                    if r.status == "downloading" {
                                        prog_map.get(&r.id).map(|p| ema_map.get(&r.id).copied().unwrap_or(p.speed_bps)).unwrap_or(0.0)
                                    } else {
                                        0.0
                                    }
                                };
                                sp(a).partial_cmp(&sp(b)).unwrap_or(std::cmp::Ordering::Equal)
                            }
                            download_table::SortColumn::Status => a.status.cmp(&b.status),
                        };
                        if asc { ord } else { ord.reverse() }
                    });
                }

                let tbl_action = download_table::render_download_table(
                    ui,
                    pal_refresh,
                    &filtered_downloads,
                    &self.active_progress,
                    &self.ema_speeds,
                    &mut self.selected_ids,
                    &self.queue,
                    &mut self.sort_col,
                    &mut self.sort_asc,
                );
                if tbl_action.add_download_clicked {
                    self.show_download_dialog(DownloadDialogRequest::empty(), "empty_state", Some(ctx));
                }
                if !tbl_action.request_delete_ids.is_empty() {
                    self.pending_delete_ids = tbl_action.request_delete_ids;
                    self.delete_also_files = false;
                }
            });

        // Confirm Delete Modal Dialog
        if !self.pending_delete_ids.is_empty() {
            let mut confirm_delete = false;
            let mut cancel_delete = false;
            let del_count = self.pending_delete_ids.len();
            egui::Window::new("Delete Download")
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, Vec2::ZERO)
                .frame(
                    egui::Frame::none()
                        .fill(pal_refresh.surface)
                        .stroke(Stroke::new(1.0_f32, pal_refresh.line))
                        .inner_margin(Margin::same(20.0))
                        .rounding(shapes::R),
                )
                .show(ctx, |ui| {
                    ui.set_width(360.0);
                    let prompt = if del_count == 1 {
                        "Remove the selected download from the queue?".to_string()
                    } else {
                        format!("Remove {} selected downloads from the queue?", del_count)
                    };
                    ui.label(egui::RichText::new(prompt).size(13.5).color(pal_refresh.text));
                    ui.add_space(10.0);
                    ui.checkbox(
                        &mut self.delete_also_files,
                        egui::RichText::new("Also delete downloaded file(s) from disk")
                            .size(12.5)
                            .color(pal_refresh.muted),
                    );
                    ui.add_space(16.0);
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let del_btn = egui::Button::new(
                            egui::RichText::new("Delete").size(13.0).color(pal_refresh.on_accent),
                        )
                        .fill(pal_refresh.danger)
                        .min_size(Vec2::new(84.0, shapes::CONTROL_HEIGHT))
                        .rounding(shapes::R_SM);
                        if ui.add(del_btn).clicked() {
                            confirm_delete = true;
                        }
                        ui.add_space(8.0);
                        if ui.add(
                            egui::Button::new(egui::RichText::new("Cancel").size(13.0).color(pal_refresh.text))
                                .min_size(Vec2::new(76.0, shapes::CONTROL_HEIGHT))
                                .rounding(shapes::R_SM),
                        ).clicked() {
                            cancel_delete = true;
                        }
                    });
                });
            if confirm_delete {
                self.execute_pending_delete();
            } else if cancel_delete {
                self.pending_delete_ids.clear();
            }
        }

        // Options / Settings Modal Dialog (Rendered below download dialogs)
        if self.show_settings_dialog {
            let screen_rect = ctx.screen_rect();
            ctx.layer_painter(egui::LayerId::new(egui::Order::Middle, egui::Id::new("options_modal_backdrop")))
                .rect_filled(screen_rect, 0.0, egui::Color32::from_black_alpha(110));

            if self.options_dialog_state.is_none() {
                self.init_options_dialog_state();
            }

            let mut close_dialog = false;
            let mut save_dialog = false;

            if let Some(opt_state) = &mut self.options_dialog_state {
                egui::Window::new("Options###options_modal_window")
                    .order(egui::Order::Foreground)
                    .title_bar(false)
                    .collapsible(false)
                    .resizable(false)
                    .fixed_size(Vec2::new(620.0, 490.0))
                    .anchor(egui::Align2::CENTER_CENTER, Vec2::ZERO)
                    .frame(
                        egui::Frame::none()
                            .fill(pal_refresh.surface)
                            .stroke(Stroke::new(1.0_f32, pal_refresh.line))
                            .inner_margin(Margin::ZERO)
                            .rounding(shapes::R)
                            .shadow(egui::epaint::Shadow {
                                offset: Vec2::new(0.0, 8.0),
                                blur: 24.0,
                                spread: 0.0,
                                color: egui::Color32::from_black_alpha(120),
                            }),
                    )
                    .show(ctx, |ui| {
                        ui.spacing_mut().item_spacing = Vec2::ZERO;
                        let outcome = render_options_window_content(
                            ui,
                            pal_refresh,
                            self.logo_texture.as_ref(),
                            opt_state,
                        );
                        match outcome {
                            OptionsDialogOutcome::Save => {
                                save_dialog = true;
                                close_dialog = true;
                            }
                            OptionsDialogOutcome::Cancel => {
                                close_dialog = true;
                            }
                            OptionsDialogOutcome::None => {}
                        }
                    });
            }

            if save_dialog {
                if let Some(opt_state) = &self.options_dialog_state {
                    let prev_tray = self.setting_show_tray_icon;
                    self.setting_show_tray_icon = opt_state.tray;
                    self.setting_autostart = opt_state.startup;
                    self.setting_start_minimized = opt_state.minstart;
                    self.setting_minimize_to_tray = opt_state.mintray;
                    self.setting_default_connections = opt_state.threads;
                    self.add_connections = opt_state.threads;
                    self.setting_global_speed_kb = match opt_state.unit {
                        SpeedUnit::Mbps => opt_state.speed * 1024,
                        SpeedUnit::Kbps => opt_state.speed,
                    };
                    self.setting_default_save_dir = opt_state.folder.clone();
                    self.clipboard_enabled.store(opt_state.clip, Ordering::Relaxed);
                    self.setting_clipboard_monitor = opt_state.clip;
                    self.setting_notifications_enabled = opt_state.notif;
                    self.setting_dont_show_complete_dialog = !opt_state.cwin;
                    self.setting_open_main_on_download_start = opt_state.openmain;
                    self.setting_proxy_url = opt_state.proxy.clone();

                    if prev_tray != self.setting_show_tray_icon {
                        if self.setting_show_tray_icon {
                            self.has_tray_icon = platform::build_system_tray(&self.tray_tx, ctx.clone(), ICON_RGBA, APP_TITLE);
                        } else {
                            platform::destroy_system_tray();
                            self.has_tray_icon = false;
                        }
                    }

                    let q = Arc::clone(&self.queue);
                    let db = self.queue.db().clone();
                    let max_c = self.setting_max_concurrent;
                    let speed_bps = self.setting_global_speed_kb * 1024;
                    let conns = self.setting_default_connections;
                    let save_dir = self.setting_default_save_dir.clone();
                    let proxy = self.setting_proxy_url.clone();
                    let clip = self.setting_clipboard_monitor;
                    let notify = self.setting_notifications_enabled;
                    let open_main = self.setting_open_main_on_download_start;
                    let dont_complete = self.setting_dont_show_complete_dialog;
                    let min_tray = self.setting_minimize_to_tray;
                    let show_tray = self.setting_show_tray_icon;
                    let autostart = self.setting_autostart;
                    let start_min = self.setting_start_minimized;

                    tokio::spawn(async move {
                        q.set_max_concurrent(max_c).await;
                        q.set_global_speed_limit(speed_bps).await;
                        q.set_proxy(if proxy.trim().is_empty() { None } else { Some(proxy.clone()) }).await;
                        let _ = db.set_setting("max_concurrent_downloads", &max_c.to_string()).await;
                        let _ = db.set_setting("global_speed_limit_bps", &speed_bps.to_string()).await;
                        let _ = db.set_setting("default_num_segments", &conns.to_string()).await;
                        let _ = db.set_setting("default_save_dir", &save_dir).await;
                        let _ = db.set_setting("proxy_url", &proxy).await;
                        let _ = db.set_setting("clipboard_monitor_enabled", if clip { "true" } else { "false" }).await;
                        let _ = db.set_setting("notifications_enabled", if notify { "true" } else { "false" }).await;
                        let _ = db.set_setting("open_main_on_download_start", if open_main { "true" } else { "false" }).await;
                        let _ = db.set_setting("dont_show_complete_dialog", if dont_complete { "true" } else { "false" }).await;
                        let _ = db.set_setting("minimize_to_tray_enabled", if min_tray { "true" } else { "false" }).await;
                        let _ = db.set_setting("show_tray_icon_enabled", if show_tray { "true" } else { "false" }).await;
                        let _ = db.set_setting("start_minimized_enabled", if start_min { "true" } else { "false" }).await;
                    });
                    platform::set_autostart_enabled(autostart, start_min);
                }
            }

            if close_dialog {
                self.show_settings_dialog = false;
                self.options_dialog_state = None;
            }
        }

        // Render open download windows (IDM-style unified detached window, Foreground order, always on top)
        // Always render as a real, detached viewport window (with minimize & close buttons)
        // whether the main dashboard is open or minimized.
        let mut child_outcomes: Vec<(String, DownloadWindowOutcome)> = Vec::new();
        for idx in 0..self.download_windows.len() {
            if self.download_windows[idx].is_open {
                let dialog_id = self.download_windows[idx].dialog_id.clone();
                let vp_id = egui::ViewportId::from_hash_of(format!("dl_window_{}", dialog_id));
                let mut vp_builder = egui::ViewportBuilder::default()
                    .with_title(DOWNLOAD_FILE_INFO_TITLE)
                    .with_inner_size(Vec2::new(
                        crate::dialogs::download_window::DOWNLOAD_WINDOW_WIDTH,
                        crate::dialogs::download_window::DOWNLOAD_WINDOW_HEIGHT,
                    ))
                    .with_min_inner_size(Vec2::new(
                        crate::dialogs::download_window::DOWNLOAD_WINDOW_WIDTH,
                        crate::dialogs::download_window::DOWNLOAD_WINDOW_HEIGHT,
                    ))
                    .with_max_inner_size(Vec2::new(
                        crate::dialogs::download_window::DOWNLOAD_WINDOW_WIDTH,
                        crate::dialogs::download_window::DOWNLOAD_WINDOW_HEIGHT,
                    ))
                    .with_transparent(false)
                    .with_resizable(false)
                    .with_decorations(false)
                    .with_minimize_button(true)
                    .with_close_button(true)
                    .with_taskbar(true)
                    .with_always_on_top();

                if let Some(pos) = self.download_windows[idx].window_pos {
                    vp_builder = vp_builder.with_position(pos);
                }

                let mut child_outcome = DownloadWindowOutcome::None;
                platform::log_daemon(&format!("STEP: dialog shown on child viewport (main open): idx={}, dialog_id='{}'", idx, dialog_id));
                ctx.show_viewport_immediate(vp_id, vp_builder, |child_ctx, _class| {
                    let render_result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        render_download_window_content(
                            child_ctx,
                            &pal_refresh,
                            self.logo_texture.as_ref(),
                            &mut self.download_windows[idx],
                        )
                    }));
                    match render_result {
                        Ok(out) => {
                            child_outcome = out;
                        }
                        Err(err) => {
                            platform::log_daemon(&format!("ERROR: Panic rendering child viewport idx={}: {:?}", idx, err));
                            egui::CentralPanel::default()
                                .frame(egui::Frame::none().fill(pal_refresh.surface).inner_margin(Margin::same(24.0)))
                                .show(child_ctx, |ui| {
                                    ui.vertical_centered(|ui| {
                                        ui.add_space(50.0);
                                        ui.heading("Couldn't open the download window");
                                        ui.add_space(16.0);
                                        ui.label("An error occurred while preparing this download dialog.");
                                        ui.add_space(24.0);
                                        if ui.button("Close").clicked() {
                                            child_outcome = DownloadWindowOutcome::Close;
                                        }
                                    });
                                });
                        }
                    }
                });
                child_outcomes.push((dialog_id, child_outcome));
            }
        }
        for (did, out) in child_outcomes {
            self.handle_download_window_outcome_by_id(&did, out, ctx);
        }

        // About Modal Dialog
        if self.show_about_dialog {
            egui::Window::new("About Cosmo Downloader")
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, Vec2::ZERO)
                .frame(
                    egui::Frame::none()
                        .fill(pal_refresh.surface)
                        .stroke(Stroke::new(1.0_f32, pal_refresh.line))
                        .inner_margin(Margin::same(20.0))
                        .rounding(shapes::R),
                )
                .show(ctx, |ui| {
                    ui.set_width(340.0);
                    ui.horizontal(|ui| {
                        if let Some(tex) = &self.logo_texture {
                            ui.image((tex.id(), Vec2::splat(28.0)));
                            ui.add_space(6.0);
                        }
                        ui.vertical(|ui| {
                            ui.label(
                                egui::RichText::new("Cosmo Downloader")
                                    .size(16.0)
                                    .strong()
                                    .color(pal_refresh.text),
                            );
                            ui.label(
                                egui::RichText::new(format!("Version {} · © 2026 Mahedi", env!("CARGO_PKG_VERSION")))
                                    .size(12.5)
                                    .color(pal_refresh.muted),
                            );
                        });
                    });
                    ui.add_space(10.0);
                    ui.label(
                        egui::RichText::new("Fast multi-segment download manager for Windows with seamless browser integration.")
                            .size(13.0)
                            .color(pal_refresh.text),
                    );
                    ui.add_space(16.0);
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let ok_btn = egui::Button::new(
                            egui::RichText::new("Close").size(13.0).color(pal_refresh.on_accent),
                        )
                        .fill(pal_refresh.accent)
                        .min_size(Vec2::new(80.0, shapes::CONTROL_HEIGHT))
                        .rounding(shapes::R_SM);
                        if ui.add(ok_btn).clicked() {
                            self.show_about_dialog = false;
                        }
                    });
                });
        }
    }
}
