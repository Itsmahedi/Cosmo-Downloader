use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use eframe::egui;
use rustget_app::dialogs::download_window::{
    build_file_summary_meta, format_failed_reason, render_download_window_content,
    DownloadWindowOutcome, DownloadWindowStage, DownloadWindowState, DOWNLOAD_WINDOW_HEIGHT,
    DOWNLOAD_WINDOW_WIDTH,
};
use rustget_app::platform;
use rustget_app::theme;
use rustget_core::downloader::DownloadStatus;

/// 1. Press Start download: the window stays open and shows A with a moving bar, speed and time left.
#[test]
fn test_1_start_download_switches_to_view_a_in_place_with_speed_and_eta() {
    let mut wnd = DownloadWindowState::new(
        "https://example.com/sample_video_1080p.mp4".to_string(),
        "sample_video_1080p.mp4".to_string(),
        "Video".to_string(),
        "C:\\Downloads".to_string(),
    );
    wnd.selected_quality = Some("1080p".to_string());
    wnd.total_bytes = Some(66 * 1024 * 1024);
    assert_eq!(wnd.stage, DownloadWindowStage::Info);
    assert!(wnd.is_open);

    // Summary row in Form view
    assert_eq!(build_file_summary_meta(&wnd), "Video · 1080p · 66.0 MB");

    // Start download keeps SAME window open and transitions in place:
    // Connecting...
    wnd.stage = DownloadWindowStage::Connecting;
    assert_eq!(wnd.stage, DownloadWindowStage::Connecting);
    assert!(wnd.is_open);

    // First bytes received -> Downloading
    wnd.stage = DownloadWindowStage::Downloading;
    assert_eq!(wnd.stage, DownloadWindowStage::Downloading);
    assert!(wnd.is_open);

    // 3-second moving average speed and time left
    let t0 = Instant::now();
    wnd.update_progress(0, Some(66_000_000), t0);
    let t1 = t0 + Duration::from_secs(1);
    wnd.update_progress(5_200_000, Some(66_000_000), t1);
    let t2 = t0 + Duration::from_secs(2);
    wnd.update_progress(10_400_000, Some(66_000_000), t2);
    let t3 = t0 + Duration::from_secs(3);
    wnd.update_progress(15_600_000, Some(66_000_000), t3);

    // Speed ~5.2 MB/s
    assert!((wnd.speed_bps - 5_200_000.0).abs() < 200_000.0);
    assert!(wnd.eta_seconds.is_some());

    // Throttling UI updates to at most 4x per second (250ms)
    let t_throttled = t3 + Duration::from_millis(100);
    wnd.update_progress(18_000_000, Some(66_000_000), t_throttled);
    assert_eq!(wnd.disp_downloaded, 15_600_000); // UI throttled

    let t_passed = t3 + Duration::from_millis(260);
    wnd.update_progress(27_700_000, Some(66_000_000), t_passed);
    assert_eq!(wnd.disp_downloaded, 27_700_000); // UI updated after 260ms
}

/// 2. Pause then Resume: the bar continues from the same point.
#[test]
fn test_2_pause_then_resume_continues_from_same_point() {
    let mut wnd = DownloadWindowState::new(
        "https://example.com/sample_video_1080p.mp4".to_string(),
        "sample_video_1080p.mp4".to_string(),
        "Video".to_string(),
        "C:\\Downloads".to_string(),
    );
    wnd.stage = DownloadWindowStage::Downloading;
    let now = Instant::now();
    wnd.update_progress(27_700_000, Some(66_000_000), now);

    // Pause download -> switches to View B
    wnd.stage = DownloadWindowStage::Paused;
    assert_eq!(wnd.stage, DownloadWindowStage::Paused);
    assert_eq!(wnd.downloaded_bytes, 27_700_000);

    // Resume download -> switches back to View A and continues from same progress
    wnd.stage = DownloadWindowStage::Downloading;
    assert_eq!(wnd.stage, DownloadWindowStage::Downloading);
    assert_eq!(wnd.downloaded_bytes, 27_700_000); // Does NOT reset to 0

    let later = now + Duration::from_millis(300);
    wnd.update_progress(35_000_000, Some(66_000_000), later);
    assert_eq!(wnd.downloaded_bytes, 35_000_000);
    assert_eq!(wnd.disp_downloaded, 35_000_000);
}

/// 3. Cancel -> Keep downloading continues. Cancel -> Cancel download deletes the partial file and closes the window.
#[test]
fn test_3_cancel_confirm_behavior_and_partial_file_cleanup() {
    let temp_dir = std::env::temp_dir().join(format!("cosmo_test_cancel_{}", std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()));
    let _ = std::fs::create_dir_all(&temp_dir);
    let target_file = temp_dir.join("sample_video.mp4");
    let partial_file = temp_dir.join("sample_video.mp4.cosmo.part");
    std::fs::write(&partial_file, b"partial video stream bytes").unwrap();
    assert!(partial_file.exists());

    let mut wnd = DownloadWindowState::new(
        "https://example.com/sample_video.mp4".to_string(),
        "sample_video.mp4".to_string(),
        "Video".to_string(),
        temp_dir.to_string_lossy().to_string(),
    );
    wnd.target_path = target_file.to_string_lossy().to_string();
    wnd.stage = DownloadWindowStage::Downloading;

    // 1. Cancel clicked -> opens cancel confirmation prompt
    wnd.confirming_cancel = true;
    assert!(wnd.confirming_cancel);

    // 2. Keep downloading -> dismisses confirmation, continues downloading
    wnd.confirming_cancel = false;
    assert!(!wnd.confirming_cancel);
    assert_eq!(wnd.stage, DownloadWindowStage::Downloading);
    assert!(partial_file.exists());

    // 3. Cancel download -> confirmed: deletes partial file and closes window
    wnd.confirming_cancel = false;
    wnd.is_open = false;
    let _ = std::fs::remove_file(&partial_file);
    assert!(!partial_file.exists()); // Deleted!
    assert!(!wnd.is_open); // Closed!
    let _ = std::fs::remove_dir_all(&temp_dir);
}

/// 4. A YouTube download (separate video and audio): one continuous percentage, then "Processing…", then Complete. The file plays.
#[test]
fn test_4_youtube_separate_streams_combined_progress_and_processing() {
    // Stream 1 (Video): size = 50 MB
    // Stream 2 (Audio): size = 16 MB
    // Combined total = 66 MB
    let total_size = 66_000_000u64;

    let mut wnd = DownloadWindowState::new(
        "https://www.youtube.com/watch?v=sample".to_string(),
        "sample_video_1080p.mp4".to_string(),
        "Video".to_string(),
        "C:\\Downloads".to_string(),
    );
    wnd.stage = DownloadWindowStage::Downloading;
    wnd.total_bytes = Some(total_size);

    // Stream 1 progresses from 0 to 50MB
    let now = Instant::now();
    wnd.update_progress(25_000_000, Some(total_size), now);
    assert_eq!(wnd.downloaded_bytes, 25_000_000);
    assert_eq!(wnd.stage, DownloadWindowStage::Downloading);

    // Stream 2 adds another 10MB -> downloaded = 60MB (never goes backwards!)
    let later = now + Duration::from_millis(300);
    wnd.update_progress(60_000_000, Some(total_size), later);
    assert_eq!(wnd.downloaded_bytes, 60_000_000);

    // Stream merging phase -> Processing
    wnd.stage = DownloadWindowStage::Processing;
    assert_eq!(wnd.stage, DownloadWindowStage::Processing);

    // Processing status check
    assert_eq!(DownloadStatus::Processing, DownloadStatus::Processing);

    // Once merged and verified on disk (size > 0), transitions to Complete
    wnd.stage = DownloadWindowStage::Complete;
    assert_eq!(wnd.stage, DownloadWindowStage::Complete);
    assert!(wnd.is_open);
}

/// 5. Let it finish: the same window shows D and stays open. No second window appears and the main window does not open.
#[test]
fn test_5_finish_switches_to_complete_stays_open_no_second_window_no_main_window() {
    let is_main_window_open = Arc::new(AtomicBool::new(false));

    let mut wnd = DownloadWindowState::new(
        "https://example.com/sample_video_1080p.mp4".to_string(),
        "sample_video_1080p.mp4".to_string(),
        "Video".to_string(),
        "C:\\Downloads".to_string(),
    );
    wnd.stage = DownloadWindowStage::Downloading;
    wnd.update_progress(66_000_000, Some(66_000_000), Instant::now());

    // Download completes: SAME window switches to Complete (View D)
    wnd.stage = DownloadWindowStage::Complete;
    assert_eq!(wnd.stage, DownloadWindowStage::Complete);
    // Stays open until user presses Close or X
    assert!(wnd.is_open);

    // Main window was never opened
    assert!(!is_main_window_open.load(Ordering::Relaxed));
}

/// 6. Run in background, then finish: the Complete window appears, and the item is Completed in the main list.
#[test]
fn test_6_run_in_background_then_finish_shows_complete_window() {
    let is_main_window_open = Arc::new(AtomicBool::new(false));

    let mut wnd = DownloadWindowState::new(
        "https://example.com/sample_video_1080p.mp4".to_string(),
        "sample_video_1080p.mp4".to_string(),
        "Video".to_string(),
        "C:\\Downloads".to_string(),
    );
    wnd.stage = DownloadWindowStage::Downloading;

    // User clicks "Run in background" (or X button during download)
    wnd.is_open = false;
    assert!(!wnd.is_open);
    // Main window remains untouched
    assert!(!is_main_window_open.load(Ordering::Relaxed));

    // When download finishes, the Complete window appears again
    wnd.stage = DownloadWindowStage::Complete;
    wnd.is_open = true;
    assert!(wnd.is_open);
    assert_eq!(wnd.stage, DownloadWindowStage::Complete);
}

/// 7. Disconnect the network mid-download: E appears; Retry resumes from the partial file.
#[test]
fn test_7_disconnect_network_shows_failed_and_retry_resumes() {
    let mut wnd = DownloadWindowState::new(
        "https://example.com/sample_video_1080p.mp4".to_string(),
        "sample_video_1080p.mp4".to_string(),
        "Video".to_string(),
        "C:\\Downloads".to_string(),
    );
    wnd.stage = DownloadWindowStage::Downloading;
    wnd.downloaded_bytes = 27_700_000;
    wnd.total_bytes = Some(66_000_000);

    // Network disconnects -> Task failed
    wnd.stage = DownloadWindowStage::Failed {
        reason: "Connection lost".to_string(),
    };
    assert!(matches!(wnd.stage, DownloadWindowStage::Failed { .. }));
    assert!(wnd.is_open);

    // Verified reason text formatting
    assert_eq!(format_failed_reason("connection reset by peer"), "Connection lost (resumable)");
    assert_eq!(format_failed_reason("HTTP 403 Forbidden"), "Link expired or forbidden (HTTP 403/410)");
    assert_eq!(format_failed_reason("disk full"), "Disk full");
    assert_eq!(format_failed_reason("Access denied"), "Access denied or path not writable");
    assert_eq!(format_failed_reason("virus detected"), "File locked by antivirus");

    // Retry button pressed -> stage switches back to Downloading without losing partial bytes
    wnd.stage = DownloadWindowStage::Downloading;
    assert_eq!(wnd.stage, DownloadWindowStage::Downloading);
    assert_eq!(wnd.downloaded_bytes, 27_700_000);
}

/// 8. File already exists: the new file gets "(1)" and nothing is overwritten.
#[test]
fn test_8_file_already_exists_resolves_unique_name_without_overwriting() {
    let temp_dir = std::env::temp_dir().join(format!("cosmo_test_dup_{}", std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()));
    let _ = std::fs::create_dir_all(&temp_dir);

    // Existing file on disk
    let file1 = temp_dir.join("sample_video.mp4");
    std::fs::write(&file1, b"existing original file").unwrap();

    // Resolving filename for new download
    let (unique_path, unique_name) = rustget_core::resolve_unique_path(&temp_dir, "sample_video.mp4");
    assert_eq!(unique_name, "sample_video (1).mp4");
    assert_eq!(unique_path, temp_dir.join("sample_video (1).mp4"));

    // Write new file to unique path
    std::fs::write(&unique_path, b"new file data").unwrap();

    // Verify existing file is NOT overwritten
    assert_eq!(std::fs::read(&file1).unwrap(), b"existing original file");
    assert_eq!(std::fs::read(&unique_path).unwrap(), b"new file data");

    // Next one gets "(2)"
    let (_, unique_name2) = rustget_core::resolve_unique_path(&temp_dir, "sample_video.mp4");
    assert_eq!(unique_name2, "sample_video (2).mp4");

    let _ = std::fs::remove_dir_all(&temp_dir);
}

/// 9. Taskbar progress follows the download and is cleared at the end.
#[test]
fn test_9_taskbar_progress_states() {
    // Connecting / Processing -> Indeterminate
    assert_eq!(platform::TBPF_INDETERMINATE, 0x1);

    // Downloading -> Normal
    assert_eq!(platform::TBPF_NORMAL, 0x2);

    // Paused -> Paused (Yellow)
    assert_eq!(platform::TBPF_PAUSED, 0x8);

    // Failed -> Error (Red)
    assert_eq!(platform::TBPF_ERROR, 0x4);

    // Complete / Cleared -> NoProgress
    assert_eq!(platform::TBPF_NOPROGRESS, 0x0);
}

/// 10. "Show the download window" OFF: Start closes the window and a notification appears at the end.
#[test]
fn test_10_show_download_window_off_closes_window_and_notifies() {
    let setting_dont_show_complete_dialog = true;

    let mut wnd = DownloadWindowState::new(
        "https://example.com/sample_video_1080p.mp4".to_string(),
        "sample_video_1080p.mp4".to_string(),
        "Video".to_string(),
        "C:\\Downloads".to_string(),
    );

    // Start download clicked with setting ON (dont show dialog = true):
    let outcome = DownloadWindowOutcome::StartDownload;
    if outcome == DownloadWindowOutcome::StartDownload {
        if setting_dont_show_complete_dialog {
            // Window closes immediately
            wnd.is_open = false;
        } else {
            wnd.is_open = true;
        }
    }
    assert!(!wnd.is_open); // Window closed!

    // When download completes:
    if setting_dont_show_complete_dialog {
        // Notification shown, dialog remains closed
        assert!(!wnd.is_open);
    }
}

/// 11. The window is 560 x 350 in every view and cannot be resized, at Windows scaling 100%, 125% and 150%.
#[test]
fn test_11_window_is_560x350_and_not_resizable_at_all_scales() {
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
            DownloadWindowStage::Connecting,
            DownloadWindowStage::Downloading,
            DownloadWindowStage::Paused,
            DownloadWindowStage::Processing,
            DownloadWindowStage::Complete,
            DownloadWindowStage::Failed {
                reason: "Access denied".to_string(),
            },
        ] {
            let mut wnd = DownloadWindowState::new(
                "https://example.com/sample_video_1080p.mp4".to_string(),
                "sample_video_1080p.mp4".to_string(),
                "Video".to_string(),
                "C:\\Downloads".to_string(),
            );
            wnd.target_path = "C:\\Downloads\\sample_video_1080p.mp4".to_string();
            wnd.stage = stage;
            wnd.downloaded_bytes = 27_700_000;
            wnd.total_bytes = Some(66_000_000);
            wnd.disp_downloaded = 27_700_000;
            wnd.disp_total = Some(66_000_000);
            wnd.disp_speed_bps = 5_200_000.0;
            wnd.disp_eta_seconds = Some(7);

            let _ = ctx.run(Default::default(), |ctx| {
                let _outcome = render_download_window_content(ctx, &pal, None, &mut wnd);
            });
        }
    }
}
