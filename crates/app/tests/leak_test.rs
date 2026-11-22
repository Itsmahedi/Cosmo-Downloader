use std::time::Instant;
use rustget_app::dialogs::download_window::{DownloadWindowState, DownloadWindowStage};
use rustget_app::theme;
use rustget_core::db::{Database, DownloadRecord};

fn get_memory_bytes() -> usize {
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
            counters.pagefile_usage
        }
    }
    #[cfg(not(windows))]
    {
        if let Ok(statm) = std::fs::read_to_string("/proc/self/statm") {
            let parts: Vec<&str> = statm.split_whitespace().collect();
            if parts.len() >= 2 {
                if let Ok(pages) = parts[1].parse::<usize>() {
                    return pages * 4096;
                }
            }
        }
        0
    }
}

#[tokio::test]
async fn test_leak_open_close_windows_and_add_remove_items() {
    let ctx = eframe::egui::Context::default();
    theme::configure_fonts(&ctx);

    let db = Database::open_in_memory().unwrap();

    // Warm-up iteration
    for i in 0..5 {
        let mut wnd = DownloadWindowState::new(
            format!("https://example.com/warmup_{}.zip", i),
            format!("warmup_{}.zip", i),
            "General".to_string(),
            "C:\\Downloads".to_string(),
        );
        wnd.stage = DownloadWindowStage::Downloading;
        wnd.update_progress(1024, Some(2048), Instant::now());
        drop(wnd);
    }

    let baseline_mem = get_memory_bytes();
    println!("Baseline memory: {} bytes ({} KB)", baseline_mem, baseline_mem / 1024);

    // 1. Open and close the download window 50 times
    for i in 0..50 {
        let mut wnd = DownloadWindowState::new(
            format!("https://example.com/leak_test_{}.mp4", i),
            format!("video_{}.mp4", i),
            "Video".to_string(),
            "C:\\Downloads".to_string(),
        );
        wnd.stage = DownloadWindowStage::Connecting;
        wnd.stage = DownloadWindowStage::Downloading;
        wnd.update_progress(500_000 * (i as u64 + 1), Some(50_000_000), Instant::now());
        wnd.is_open = false;
        drop(wnd);
    }

    // 2. Add and remove 100 list items
    let mut task_ids = Vec::with_capacity(100);
    for i in 0..100 {
        let id = format!("task-{}", i);
        let rec = DownloadRecord {
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
        task_ids.push(id);
    }

    assert_eq!(task_ids.len(), 100);

    for id in &task_ids {
        db.delete_download(id).await.unwrap();
    }

    let downloads_left = db.list_downloads(None).await.unwrap();
    assert_eq!(downloads_left.len(), 0);

    let final_mem = get_memory_bytes();
    println!("Final memory after 50 windows and 100 adds/removes: {} bytes ({} KB)", final_mem, final_mem / 1024);

    if baseline_mem > 0 {
        let allowed_ceiling = (baseline_mem as f64 * 1.10) as usize;
        println!("Allowed ceiling (baseline + 10%): {} bytes", allowed_ceiling);
        assert!(
            final_mem <= allowed_ceiling || final_mem.saturating_sub(baseline_mem) < 5 * 1024 * 1024,
            "Memory leak detected! Baseline: {} bytes, Final: {} bytes (increase: {} bytes)",
            baseline_mem, final_mem, final_mem.saturating_sub(baseline_mem)
        );
    }
}
