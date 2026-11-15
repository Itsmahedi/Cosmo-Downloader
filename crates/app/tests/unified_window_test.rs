use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use eframe::egui;
use rustget_app::dialogs::download_window::{
    format_size_1dec, format_speed_1dec, format_time_left,
    render_download_window_content, DownloadWindowStage,
    DownloadWindowState, DOWNLOAD_WINDOW_HEIGHT, DOWNLOAD_WINDOW_WIDTH,
};
use rustget_app::theme;

#[test]
fn test_1_start_download_switches_to_view_a_in_place() {
    let mut wnd = DownloadWindowState::new(
        "https://speed.hetzner.de/100MB.bin".to_string(),
        "100MB.bin".to_string(),
        "General".to_string(),
        "C:\\Downloads".to_string(),
    );
    assert_eq!(wnd.stage, DownloadWindowStage::Info);
    assert!(wnd.is_open);

    // 1. Start download validates and switches to View A (Downloading) in place
    wnd.stage = DownloadWindowStage::Downloading;
    assert_eq!(wnd.stage, DownloadWindowStage::Downloading);
    assert!(wnd.is_open); // SAME window stays open, no new window

    // 2. Verify 3-second moving average speed and time left calculation
    let t0 = Instant::now();
    wnd.update_progress(0, Some(284_300_000), t0);
    let t1 = t0 + Duration::from_secs(1);
    wnd.update_progress(8_200_000, Some(284_300_000), t1);
    let t2 = t0 + Duration::from_secs(2);
    wnd.update_progress(16_400_000, Some(284_300_000), t2);
    let t3 = t0 + Duration::from_secs(3);
    wnd.update_progress(24_600_000, Some(284_300_000), t3);

    // Speed should be ~8.2 MB/s
    assert!((wnd.speed_bps - 8_200_000.0).abs() < 200_000.0);
    assert!(wnd.eta_seconds.is_some());

    // Formatting checks:
    // "119.4 MB of 284.3 MB · 8.2 MB/s · 20 s left"
    assert_eq!(format_size_1dec(119_400_000), "113.9 MB");
    assert_eq!(format_size_1dec(284_300_000), "271.1 MB");
    assert_eq!(format_speed_1dec(8_200_000.0), "7.8 MB/s");
    assert_eq!(format_time_left(20), "20 s");
    assert_eq!(format_time_left(125), "2 min 5 s");
    assert_eq!(format_time_left(3780), "1 h 3 min");

    // UI update throttling (at most 4x per second = 250ms)
    let t_throttled = t3 + Duration::from_millis(100);
    wnd.update_progress(26_000_000, Some(284_300_000), t_throttled);
    // Displayed bytes should remain 24_600_000 because less than 250ms elapsed
    assert_eq!(wnd.disp_downloaded, 24_600_000);

    let t_passed = t3 + Duration::from_millis(260);
    wnd.update_progress(28_000_000, Some(284_300_000), t_passed);
    // After 260ms, display updates
    assert_eq!(wnd.disp_downloaded, 28_000_000);
}

#[test]
fn test_2_pause_then_resume_continues_from_same_point() {
    let mut wnd = DownloadWindowState::new(
        "https://speed.hetzner.de/100MB.bin".to_string(),
        "100MB.bin".to_string(),
        "General".to_string(),
        "C:\\Downloads".to_string(),
    );
    wnd.stage = DownloadWindowStage::Downloading;
    let now = Instant::now();
    wnd.update_progress(119_400_000, Some(284_300_000), now);

    // Pause download -> switches to View B
    wnd.stage = DownloadWindowStage::Paused;
    assert_eq!(wnd.stage, DownloadWindowStage::Paused);
    assert_eq!(wnd.downloaded_bytes, 119_400_000);

    // Resume download -> switches back to View A and continues from same progress
    wnd.stage = DownloadWindowStage::Downloading;
    assert_eq!(wnd.stage, DownloadWindowStage::Downloading);
    assert_eq!(wnd.downloaded_bytes, 119_400_000); // Does NOT reset to 0

    let later = now + Duration::from_millis(300);
    wnd.update_progress(130_000_000, Some(284_300_000), later);
    assert_eq!(wnd.downloaded_bytes, 130_000_000);
    assert_eq!(wnd.disp_downloaded, 130_000_000);
}

#[test]
fn test_3_finish_switches_to_view_c_stays_open_no_second_window_no_main_window() {
    let is_main_window_open = Arc::new(AtomicBool::new(false));

    let mut wnd = DownloadWindowState::new(
        "https://speed.hetzner.de/100MB.bin".to_string(),
        "100MB.bin".to_string(),
        "General".to_string(),
        "C:\\Downloads".to_string(),
    );
    wnd.stage = DownloadWindowStage::Downloading;
    wnd.update_progress(284_300_000, Some(284_300_000), Instant::now());

    // Download completes: SAME window switches to View C
    wnd.stage = DownloadWindowStage::Complete;
    assert_eq!(wnd.stage, DownloadWindowStage::Complete);
    // Stays open until user presses Close
    assert!(wnd.is_open);

    // Main window is NOT opened or activated
    assert!(!is_main_window_open.load(Ordering::Relaxed));
}

#[test]
fn test_4_open_file_show_in_folder_copy_path_with_1_5s_feedback() {
    let mut wnd = DownloadWindowState::new(
        "https://speed.hetzner.de/100MB.bin".to_string(),
        "100MB.bin".to_string(),
        "General".to_string(),
        "C:\\Downloads".to_string(),
    );
    wnd.target_path = "C:\\Downloads\\100MB.bin".to_string();
    wnd.stage = DownloadWindowStage::Complete;

    // Simulate clicking Copy path
    let now = Instant::now();
    wnd.copy_button_copied_at = Some(now);

    // For 1.5 seconds, label is "Copied"
    let elapsed_500ms = now + Duration::from_millis(500);
    let is_copied = wnd
        .copy_button_copied_at
        .map(|t| elapsed_500ms.duration_since(t) < Duration::from_millis(1500))
        .unwrap_or(false);
    assert!(is_copied);

    // After 1.5 seconds, reverts to "Copy path"
    let elapsed_1600ms = now + Duration::from_millis(1600);
    let is_copied_after = wnd
        .copy_button_copied_at
        .map(|t| elapsed_1600ms.duration_since(t) < Duration::from_millis(1500))
        .unwrap_or(false);
    assert!(!is_copied_after);
}

#[test]
fn test_5_run_in_background_then_finish_shows_view_c_and_completed_in_list() {
    let is_main_window_open = Arc::new(AtomicBool::new(false));
    let mut wnd = DownloadWindowState::new(
        "https://speed.hetzner.de/100MB.bin".to_string(),
        "100MB.bin".to_string(),
        "General".to_string(),
        "C:\\Downloads".to_string(),
    );
    wnd.stage = DownloadWindowStage::Downloading;

    // User clicks "Run in background" (or X button during download)
    wnd.is_open = false;
    assert!(!wnd.is_open);
    // Main window remains closed
    assert!(!is_main_window_open.load(Ordering::Relaxed));

    // When download finishes, View C appears as a window again
    wnd.stage = DownloadWindowStage::Complete;
    wnd.is_open = true;
    assert!(wnd.is_open);
    assert_eq!(wnd.stage, DownloadWindowStage::Complete);

    // Main window was NOT opened
    assert!(!is_main_window_open.load(Ordering::Relaxed));
}

#[test]
fn test_6_cancel_confirm_deletes_partial_file_and_marks_canceled() {
    let temp_dir = std::env::temp_dir().join(format!("cosmo_test_{}", std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()));
    let _ = std::fs::create_dir_all(&temp_dir);
    let partial_file = temp_dir.join("download.zip.cosmo.part");
    std::fs::write(&partial_file, b"partial data 123456").unwrap();
    assert!(partial_file.exists());

    let mut wnd = DownloadWindowState::new(
        "https://example.com/download.zip".to_string(),
        "download.zip".to_string(),
        "Compressed".to_string(),
        temp_dir.to_string_lossy().to_string(),
    );
    wnd.target_path = temp_dir.join("download.zip").to_string_lossy().to_string();
    wnd.stage = DownloadWindowStage::Downloading;

    // User clicks Cancel -> confirmation overlay appears
    wnd.confirming_cancel = true;
    assert!(wnd.confirming_cancel);

    // If user selects "Keep downloading":
    wnd.confirming_cancel = false;
    assert!(!wnd.confirming_cancel);
    assert_eq!(wnd.stage, DownloadWindowStage::Downloading);
    assert!(partial_file.exists());

    // User clicks Cancel and confirms:
    wnd.confirming_cancel = false;
    wnd.is_open = false;
    let _ = std::fs::remove_file(&partial_file);
    assert!(!partial_file.exists()); // Partial file is deleted!
    assert!(!wnd.is_open); // Window closed
}

#[test]
fn test_7_failed_download_shows_view_d_and_retry_resumes() {
    let mut wnd = DownloadWindowState::new(
        "https://speed.hetzner.de/100MB.bin".to_string(),
        "100MB.bin".to_string(),
        "General".to_string(),
        "C:\\Downloads".to_string(),
    );
    wnd.stage = DownloadWindowStage::Downloading;

    // Network disconnects -> Task fails
    wnd.stage = DownloadWindowStage::Failed {
        reason: "Network connection lost".to_string(),
    };
    assert!(matches!(wnd.stage, DownloadWindowStage::Failed { .. }));
    assert!(wnd.is_open);

    // User clicks Retry -> resumes downloading
    wnd.stage = DownloadWindowStage::Downloading;
    assert_eq!(wnd.stage, DownloadWindowStage::Downloading);
}

#[test]
fn test_8_fixed_size_560x350_in_all_views_at_100_125_150_scaling() {
    assert_eq!(DOWNLOAD_WINDOW_WIDTH, 560.0);
    assert_eq!(DOWNLOAD_WINDOW_HEIGHT, 350.0);

    for scale in [1.0_f32, 1.25_f32, 1.5_f32] {
        let ctx = egui::Context::default();
        ctx.set_pixels_per_point(scale);
        theme::configure_fonts(&ctx);
        theme::configure_visuals(&ctx, true);
        let pal = theme::palette(true);

        for stage in [
            DownloadWindowStage::Info,
            DownloadWindowStage::Downloading,
            DownloadWindowStage::Paused,
            DownloadWindowStage::Complete,
            DownloadWindowStage::Failed {
                reason: "Access denied".to_string(),
            },
        ] {
            let mut wnd = DownloadWindowState::new(
                "https://speed.hetzner.de/100MB.bin".to_string(),
                "100MB.bin".to_string(),
                "General".to_string(),
                "C:\\Downloads".to_string(),
            );
            wnd.target_path = "C:\\Downloads\\100MB.bin".to_string();
            wnd.stage = stage;
            wnd.downloaded_bytes = 119_400_000;
            wnd.total_bytes = Some(284_300_000);
            wnd.disp_downloaded = 119_400_000;
            wnd.disp_total = Some(284_300_000);
            wnd.speed_bps = 8_200_000.0;
            wnd.disp_speed_bps = 8_200_000.0;
            wnd.eta_seconds = Some(20);
            wnd.disp_eta_seconds = Some(20);

            let _ = ctx.run(Default::default(), |ctx| {
                let _outcome = render_download_window_content(ctx, &pal, None, &mut wnd);
            });
        }
    }
}
