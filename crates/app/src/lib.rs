
pub mod app;
pub mod components;
pub mod dialogs;
pub mod icons;
pub mod platform;
pub mod theme;

use std::sync::Arc;
use eframe::egui;

pub use app::{
    determine_launch_origin, direct_graceful_shutdown, set_global_popup_file, set_global_queue,
    set_tokio_handle, CosmoDownloaderApp, LaunchOrigin, MainWindowReason, APP_TITLE,
    DOWNLOAD_FILE_INFO_TITLE, ICON_RGBA, MAIN_VIEWPORT_ID,
};

#[cfg(windows)]
fn send_http_to_bridge(req: &str) -> bool {
    platform::grant_foreground_permission();
    for port in [42424, 24242, 18765] {
        if let Ok(mut stream) = std::net::TcpStream::connect(format!("127.0.0.1:{}", port)) {
            let _ = std::io::Write::write_all(&mut stream, req.as_bytes());
            let _ = std::io::Write::flush(&mut stream);
            let _ = stream.set_read_timeout(Some(std::time::Duration::from_millis(50)));
            let mut resp_buf = [0u8; 128];
            let _ = std::io::Read::read(&mut stream, &mut resp_buf);
            platform::grant_foreground_permission();
            return true;
        }
    }
    false
}

fn handle_quit_command() -> ! {
    #[cfg(windows)]
    {
        let signaled = platform::windows::signal_existing_instance_shutdown();
        let token = rustget_core::server::get_or_create_bridge_token();
        let quit_req = format!(
            "POST /quit HTTP/1.1\r\nHost: 127.0.0.1\r\nX-Cosmo-Token: {}\r\nConnection: close\r\n\r\n",
            token
        );
        let bridged = send_http_to_bridge(&quit_req);
        if signaled || bridged {
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
            while std::time::Instant::now() < deadline {
                if std::net::TcpStream::connect("127.0.0.1:42424").is_err() {
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(100));
            }
        }
    }
    std::process::exit(0);
}

pub fn run() -> eframe::Result<()> {
    if std::env::args().any(|a| a == "--quit") {
        handle_quit_command();
    }

    if std::env::args().any(|a| a == "--leak-test") {
        run_leak_test_cli();
        return Ok(());
    }

    platform::logger::install_panic_hook();
    platform::watchdog::init_watchdog();
    rustget_core::set_log_callback(|msg| {
        platform::logger::log_event(None, msg);
    });
    let _ = tracing_subscriber::fmt().try_init();

    let raw_args: Vec<String> = std::env::args().collect();
    if raw_args.iter().any(|a| a == "--test-panic") {
        platform::logger::log_event(Some("TEST"), "Triggering deliberate test panic via --test-panic");
        panic!("Deliberate test panic for diagnostic verification");
    }

    let origin = determine_launch_origin(&raw_args);
    let is_screenshot_mode = raw_args.iter().any(|a| {
        let clean = a.trim_matches('"').trim_matches('\'');
        clean.starts_with("--screenshot") || clean.starts_with("--view")
    });

    platform::logger::log_event(
        Some("APP"),
        &format!(
            "Cosmo Downloader v{} starting. OS: {} {}. Origin: {:?}. ScreenshotMode: {}. Args: {:?}",
            env!("CARGO_PKG_VERSION"),
            std::env::consts::OS,
            std::env::consts::ARCH,
            origin,
            is_screenshot_mode,
            raw_args
        ),
    );

    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .max_blocking_threads(16)
        .enable_all()
        .build()
        .expect("Failed to create Tokio runtime");
    let _guard = rt.enter();
    app::set_tokio_handle(rt.handle().clone());

    // Parse optional CLI URL or --popup-file argument (Windows single-instance handover)
    #[cfg(windows)]
    let mut cli_popup_file: Option<String> = None;
    #[cfg(windows)]
    let mut cli_url: Option<String> = None;
    #[cfg(windows)]
    {
        let mut args = std::env::args().skip(1);
        while let Some(a) = args.next() {
            let clean = a.trim_matches('"').trim_matches('\'');
            if clean == "--popup-file" {
                cli_popup_file = args.next().map(|s| s.trim_matches('"').trim_matches('\'').to_string());
            } else if clean.starts_with("--popup-file=") {
                cli_popup_file = Some(clean.trim_start_matches("--popup-file=").trim_matches('"').trim_matches('\'').to_string());
            } else if clean == "--add-url" {
                cli_url = args.next().map(|s| s.trim_matches('"').trim_matches('\'').to_string());
            } else if clean.starts_with("--add-url=") {
                cli_url = Some(clean.trim_start_matches("--add-url=").trim_matches('"').trim_matches('\'').to_string());
            } else if clean.starts_with("cosmodownloader://") {
                cli_url = Some(clean.trim_start_matches("cosmodownloader://").to_string());
            } else if clean.starts_with("rustget://") {
                cli_url = Some(clean.trim_start_matches("rustget://").to_string());
            } else if clean.starts_with("http://") || clean.starts_with("https://") || clean.starts_with("magnet:") || clean.ends_with(".torrent") || clean.ends_with(".TORRENT") {
                cli_url = Some(clean.to_string());
            }
        }
    }

    #[cfg(windows)]
    let _instance_mutex_guard: *mut std::ffi::c_void;
    #[cfg(windows)]
    unsafe {
        let mutex_handle = platform::windows::CreateMutexA(
            std::ptr::null_mut(),
            1,
            c"CosmoDownloader_v1_0_SingleInstance_Mutex".as_ptr().cast(),
        );
        let last_err = platform::windows::GetLastError();
        platform::log_daemon(&format!("Mutex check: last_error={}", last_err));
        if !is_screenshot_mode && last_err == 183 {
            platform::grant_foreground_permission();
            let ipc_dir = directories::BaseDirs::new()
                .map(|d| d.data_local_dir().to_path_buf())
                .unwrap_or_else(|| std::path::PathBuf::from("."))
                .join("CosmoDownloader")
                .join("ipc_requests");
            let _ = std::fs::create_dir_all(&ipc_dir);

            if let Some(ref file_path) = cli_popup_file {
                if let Ok(content) = std::fs::read_to_string(file_path) {
                    let clean = content.trim_start_matches('\u{feff}');
                    let req = format!(
                        "POST /download HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                        clean.len(),
                        clean
                    );
                    if !send_http_to_bridge(&req) {
                        let target = ipc_dir.join(format!("req_{}.json", uuid::Uuid::new_v4()));
                        let _ = std::fs::write(&target, clean);
                    }
                    let _ = std::fs::remove_file(file_path);
                }
            } else if let Some(ref url_arg) = cli_url {
                let body = serde_json::json!({ "url": url_arg }).to_string();
                let req = format!(
                    "POST /download HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                );
                if !send_http_to_bridge(&req) {
                    let target = ipc_dir.join(format!("req_{}.json", uuid::Uuid::new_v4()));
                    let _ = std::fs::write(&target, &body);
                }
            } else if origin == LaunchOrigin::User {
                // User double-clicked app while running -> request main window show
                let token = rustget_core::server::get_or_create_bridge_token();
                let show_req = format!(
                    "POST /show HTTP/1.1\r\nHost: 127.0.0.1\r\nX-Cosmo-Token: {}\r\nConnection: close\r\n\r\n",
                    token
                );
                if !send_http_to_bridge(&show_req) {
                    let target = ipc_dir.join(format!("show_{}.json", uuid::Uuid::new_v4()));
                    let _ = std::fs::write(&target, "{}");
                }
            }

            platform::windows::show_download_popup_window();
            std::process::exit(0);
        }
        _instance_mutex_guard = mutex_handle;
    }

    #[cfg(not(windows))]
    let _unix_instance_guard = {
        if !is_screenshot_mode && origin == LaunchOrigin::User {
            if let Ok(mut stream) = std::net::TcpStream::connect("127.0.0.1:42424") {
                let token = rustget_core::server::get_or_create_bridge_token();
                let show_req = format!(
                    "POST /show HTTP/1.1\r\nHost: 127.0.0.1\r\nX-Cosmo-Token: {}\r\nConnection: close\r\n\r\n",
                    token
                );
                let _ = std::io::Write::write_all(&mut stream, show_req.as_bytes());
                std::process::exit(0);
            }
        }
    };

    let window_icon = egui::IconData {
        rgba: ICON_RGBA.to_vec(),
        width: 32,
        height: 32,
    };

    let custom_scale: Option<f32> = std::env::args().find_map(|a| {
        let clean = a.trim_matches('"').trim_matches('\'');
        if clean.starts_with("--scale=") {
            clean.trim_start_matches("--scale=").parse().ok()
        } else {
            None
        }
    });

    let custom_size: Option<(f32, f32)> = std::env::args().find_map(|a| {
        let clean = a.trim_matches('"').trim_matches('\'');
        if clean.starts_with("--size=") {
            let part = clean.trim_start_matches("--size=");
            let mut parts = part.split('x');
            let w = parts.next()?.parse().ok()?;
            let h = parts.next()?.parse().ok()?;
            Some((w, h))
        } else {
            None
        }
    }).or_else(|| {
        if is_screenshot_mode && std::env::args().any(|a| {
            let clean = a.trim_matches('"').trim_matches('\'');
            clean.starts_with("--view=") && clean != "--view=options" && !clean.starts_with("--view=options-")
        }) {
            Some((crate::dialogs::download_window::DOWNLOAD_WINDOW_WIDTH, crate::dialogs::download_window::DOWNLOAD_WINDOW_HEIGHT))
        } else {
            None
        }
    });

    let is_popup_arg = raw_args.iter().skip(1).any(|a| {
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
            || clean == "--from-extension"
    });
    let start_minimized = origin == LaunchOrigin::Autostart || raw_args.iter().any(|a| a == "--minimized" || a == "--daemon");

    let mut viewport = egui::ViewportBuilder::default()
        .with_icon(Arc::new(window_icon));

    if is_popup_arg {
        let (w, h) = if raw_args.iter().any(|a| {
            let clean = a.trim_matches('"').trim_matches('\'');
            clean == "--view=cascade" || clean == "--view=cascade_canvas"
        }) {
            (640.0, 430.0)
        } else if let Some(s) = custom_scale {
            (560.0 * s, 350.0 * s)
        } else {
            (560.0, 350.0)
        };
        let (pos_x, pos_y) = platform::get_dialog_center_pos(false, w, h);
        viewport = viewport
            .with_app_id("cosmo-dialog")
            .with_title(DOWNLOAD_FILE_INFO_TITLE)
            .with_position([pos_x, pos_y])
            .with_inner_size([w, h])
            .with_min_inner_size([w, h])
            .with_max_inner_size([w, h])
            .with_resizable(false)
            .with_maximize_button(false)
            .with_minimize_button(true)
            .with_close_button(true)
            .with_decorations(false)
            .with_visible(true)
            .with_active(true)
            .with_always_on_top();
    } else if start_minimized && !is_screenshot_mode {
        viewport = viewport
            .with_app_id("cosmo-downloader")
            .with_title(APP_TITLE)
            .with_decorations(false)
            .with_inner_size([1000.0, 640.0])
            .with_min_inner_size([760.0, 480.0])
            .with_resizable(true)
            .with_visible(false)
            .with_active(false);
    } else {
        let (win_w, win_h) = custom_size.unwrap_or((1000.0, 640.0));
        viewport = viewport
            .with_app_id("cosmo-downloader")
            .with_title(APP_TITLE)
            .with_decorations(false)
            .with_inner_size([win_w, win_h])
            .with_min_inner_size([760.0, 480.0])
            .with_resizable(true)
            .with_visible(true)
            .with_active(true);
    }

    let native_options = eframe::NativeOptions {
        viewport,
        ..Default::default()
    };

    platform::log_daemon("Calling eframe::run_native...");
    let res = eframe::run_native(
        "Cosmo Downloader",
        native_options,
        Box::new(move |cc| {
            if let Some(s) = custom_scale {
                cc.egui_ctx.set_pixels_per_point(s);
            }
            let ppp = cc.egui_ctx.pixels_per_point();
            platform::logger::log_event(
                Some("DISPLAY"),
                &format!("Display initialized: pixels_per_point={:.2}", ppp),
            );
            platform::log_daemon("Inside eframe creator closure...");
            Ok(Box::new(CosmoDownloaderApp::new(cc)))
        }),
    );
    if let Err(ref e) = res {
        platform::log_daemon(&format!("eframe::run_native error: {:?}", e));
    } else {
        platform::log_daemon("eframe::run_native exited cleanly");
    }
    res
}

fn run_leak_test_cli() {
    println!("=== Cosmo Downloader Leak Test ===");
    println!("Testing 50 window open/close cycles and 100 queue item add/remove cycles...");

    let ctx = eframe::egui::Context::default();
    crate::theme::configure_fonts(&ctx);

    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .max_blocking_threads(8)
        .enable_all()
        .build()
        .expect("Tokio runtime creation");
    let _guard = rt.enter();

    let db = rt.block_on(async {
        rustget_core::db::Database::open_in_memory().expect("open DB")
    });

    // Helper to get memory bytes (working set and commit)
    fn get_mem() -> (usize, usize) {
        #[cfg(windows)]
        {
            #[repr(C)]
            struct PROCESS_MEMORY_COUNTERS {
                cb: u32,
                page_fault_count: u32,
                peak_working_set_size: usize,
                working_set_size: usize,
                quota_peak_paged_pool_usage: usize,
                quota_paged_pool_usage: usize,
                quota_peak_non_paged_pool_usage: usize,
                quota_non_paged_pool_usage: usize,
                pagefile_usage: usize,
                peak_pagefile_usage: usize,
            }
            #[link(name = "psapi")]
            extern "system" {
                fn GetCurrentProcess() -> *mut std::ffi::c_void;
                fn GetProcessMemoryInfo(
                    process: *mut std::ffi::c_void,
                    counters: *mut PROCESS_MEMORY_COUNTERS,
                    cb: u32,
                ) -> i32;
            }
            unsafe {
                let mut counters = std::mem::zeroed::<PROCESS_MEMORY_COUNTERS>();
                counters.cb = std::mem::size_of::<PROCESS_MEMORY_COUNTERS>() as u32;
                GetProcessMemoryInfo(GetCurrentProcess(), &mut counters, counters.cb);
                (counters.working_set_size, counters.pagefile_usage)
            }
        }
        #[cfg(not(windows))]
        {
            if let Ok(statm) = std::fs::read_to_string("/proc/self/statm") {
                let parts: Vec<&str> = statm.split_whitespace().collect();
                if parts.len() >= 2 {
                    if let Ok(pages) = parts[1].parse::<usize>() {
                        return (pages * 4096, pages * 4096);
                    }
                }
            }
            (0, 0)
        }
    }

    // Warm-up iteration (both windows and DB operations)
    for i in 0..5 {
        let mut wnd = crate::dialogs::download_window::DownloadWindowState::new(
            format!("https://example.com/warmup_{}.zip", i),
            format!("warmup_{}.zip", i),
            "General".to_string(),
            "C:\\Downloads".to_string(),
        );
        wnd.stage = crate::dialogs::download_window::DownloadWindowStage::Downloading;
        wnd.update_progress(1024, Some(2048), std::time::Instant::now());
        drop(wnd);
    }
    rt.block_on(async {
        let rec = rustget_core::db::DownloadRecord {
            id: "warmup".to_string(),
            url: "https://example.com/warmup.bin".to_string(),
            filename: "warmup.bin".to_string(),
            target_path: "C:\\Downloads\\warmup.bin".to_string(),
            part_path: "C:\\Downloads\\warmup.bin.part".to_string(),
            category: "General".to_string(),
            total_size: Some(1024),
            downloaded_bytes: 0,
            status: "queued".to_string(),
            num_segments: 1,
            expected_sha256: None,
            custom_headers: None,
            user_agent: None,
            referer: None,
            cookie: None,
            max_speed_bps: 0,
            retry_count: 0,
            max_retries: 3,
            error_message: None,
            created_at: chrono::Utc::now().to_rfc3339(),
            updated_at: chrono::Utc::now().to_rfc3339(),
        };
        let _ = db.insert_download(&rec).await;
        let _ = db.delete_download("warmup").await;
    });

    std::thread::sleep(std::time::Duration::from_millis(100));
    #[cfg(windows)]
    crate::platform::trim_working_set();

    let (baseline_ws, baseline_commit) = get_mem();
    println!("Baseline memory: WS {:.2} MB, Commit {:.2} MB", 
        baseline_ws as f64 / (1024.0 * 1024.0),
        baseline_commit as f64 / (1024.0 * 1024.0)
    );

    // 1. Open and close 50 download windows
    for i in 0..50 {
        let mut wnd = crate::dialogs::download_window::DownloadWindowState::new(
            format!("https://example.com/leak_{}.mp4", i),
            format!("video_{}.mp4", i),
            "Video".to_string(),
            "C:\\Downloads".to_string(),
        );
        wnd.stage = crate::dialogs::download_window::DownloadWindowStage::Connecting;
        wnd.stage = crate::dialogs::download_window::DownloadWindowStage::Downloading;
        wnd.update_progress(100_000 * (i as u64 + 1), Some(10_000_000), std::time::Instant::now());
        wnd.is_open = false;
        drop(wnd);
    }

    // 2. Add and remove 100 items
    rt.block_on(async {
        let mut ids = Vec::new();
        for i in 0..100 {
            let id = format!("task-{}", i);
            let rec = rustget_core::db::DownloadRecord {
                id: id.clone(),
                url: format!("https://example.com/item_{}.bin", i),
                filename: format!("item_{}.bin", i),
                target_path: format!("C:\\Downloads\\item_{}.bin", i),
                part_path: format!("C:\\Downloads\\item_{}.bin.part", i),
                category: "General".to_string(),
                total_size: Some(1024 * 1024),
                downloaded_bytes: 0,
                status: "queued".to_string(),
                num_segments: 8,
                expected_sha256: None,
                custom_headers: None,
                user_agent: None,
                referer: None,
                cookie: None,
                max_speed_bps: 0,
                retry_count: 0,
                max_retries: 3,
                error_message: None,
                created_at: chrono::Utc::now().to_rfc3339(),
                updated_at: chrono::Utc::now().to_rfc3339(),
            };
            db.insert_download(&rec).await.unwrap();
            ids.push(id);
        }
        for id in &ids {
            db.delete_download(id).await.unwrap();
        }
    });

    std::thread::sleep(std::time::Duration::from_millis(200));
    #[cfg(windows)]
    crate::platform::trim_working_set();

    let (final_ws, final_commit) = get_mem();
    println!("Final memory: WS {:.2} MB, Commit {:.2} MB",
        final_ws as f64 / (1024.0 * 1024.0),
        final_commit as f64 / (1024.0 * 1024.0)
    );

    let baseline_measure = if baseline_commit > 0 { baseline_commit } else { baseline_ws };
    let final_measure = if baseline_commit > 0 { final_commit } else { final_ws };

    if baseline_measure > 0 {
        let max_allowed = (baseline_measure as f64 * 1.10) as usize;
        let delta = if final_measure > baseline_measure { final_measure - baseline_measure } else { 0 };
        println!("Memory increase: {:.2} MB (Ceiling +10%: {:.2} MB)",
            delta as f64 / (1024.0 * 1024.0),
            max_allowed as f64 / (1024.0 * 1024.0)
        );
        if final_measure <= max_allowed || delta < 3 * 1024 * 1024 {
            println!("PASS: Leak test passed! Memory returned to within 10% of baseline.");
        } else {
            eprintln!("FAIL: Memory exceeded baseline + 10%!");
            std::process::exit(1);
        }
    }
}
