use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

static PROCESS_START: OnceLock<Instant> = OnceLock::new();
static LAST_HEARTBEAT_MS: AtomicU64 = AtomicU64::new(0);
static ANY_WINDOW_VISIBLE: AtomicBool = AtomicBool::new(false);
static IS_FROZEN: AtomicBool = AtomicBool::new(false);
static LAST_ACTIVITY: OnceLock<Mutex<String>> = OnceLock::new();

fn start_time() -> Instant {
    *PROCESS_START.get_or_init(Instant::now)
}

fn get_activity_lock() -> &'static Mutex<String> {
    LAST_ACTIVITY.get_or_init(|| Mutex::new("Startup".to_string()))
}

/// Called from the UI thread (CosmoDownloaderApp::update) on every frame.
pub fn heartbeat(activity: &str, window_visible: bool) {
    let elapsed = start_time().elapsed().as_millis() as u64;
    LAST_HEARTBEAT_MS.store(elapsed, Ordering::Release);
    ANY_WINDOW_VISIBLE.store(window_visible, Ordering::Release);

    if let Ok(mut act) = get_activity_lock().lock() {
        if act.as_str() != activity {
            *act = activity.to_string();
        }
    }
}

/// Start the background watchdog monitor thread.
pub fn init_watchdog() {
    let _ = start_time(); // initialize start time
    let _ = std::thread::Builder::new()
        .name("cosmo_watchdog".to_string())
        .spawn(move || {
            let mut freeze_start_ms = 0u64;

            loop {
                std::thread::sleep(Duration::from_millis(250));

                let visible = ANY_WINDOW_VISIBLE.load(Ordering::Acquire);
                let last_ms = LAST_HEARTBEAT_MS.load(Ordering::Acquire);
                let now_ms = start_time().elapsed().as_millis() as u64;

                // Only monitor for UI freezes when at least one window is visible
                // and after the first heartbeat has been registered
                if visible && last_ms > 0 {
                    let diff_ms = now_ms.saturating_sub(last_ms);

                    if diff_ms >= 2000 {
                        if !IS_FROZEN.load(Ordering::Acquire) {
                            IS_FROZEN.store(true, Ordering::Release);
                            freeze_start_ms = last_ms;

                            let last_activity = get_activity_lock()
                                .lock()
                                .map(|a| a.clone())
                                .unwrap_or_else(|_| "Unknown".to_string());

                            let recent = crate::platform::logger::get_recent_logs(20);
                            let freeze_duration_secs = (diff_ms as f64) / 1000.0;

                            let msg = format!(
                                "UI FROZEN: UI thread has not completed a frame for {:.2}s while window is visible!\n\
                                 Last activity: {}\n\
                                 Last 20 events:\n{}",
                                freeze_duration_secs,
                                last_activity,
                                recent.concat()
                            );

                            crate::platform::logger::log_event(Some("WATCHDOG"), &msg);
                            crate::platform::logger::flush_logs();
                        }
                    } else if IS_FROZEN.load(Ordering::Acquire) {
                        IS_FROZEN.store(false, Ordering::Release);
                        let total_frozen_secs = (now_ms.saturating_sub(freeze_start_ms) as f64) / 1000.0;
                        let msg = format!("UI RECOVERED: UI thread completed a frame after {:.2}s freeze.", total_frozen_secs);
                        crate::platform::logger::log_event(Some("WATCHDOG"), &msg);
                        crate::platform::logger::flush_logs();
                    }
                } else if IS_FROZEN.load(Ordering::Acquire) {
                    // Window was hidden or closed during freeze
                    IS_FROZEN.store(false, Ordering::Release);
                }
            }
        });
}
