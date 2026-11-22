pub mod category;
pub mod checksum;
pub mod clipboard;
pub mod db;
pub mod downloader;
pub mod error;
pub mod probe;
pub mod queue;
pub mod rate_limiter;
pub mod segment;
pub mod server;
pub mod speed;
pub mod state;
pub mod torrent;
pub mod youtube;

// Re-exports for convenient top-level access
pub use category::{categorize, Category, CategoryFilter};
pub use checksum::{compute_sha256, verify_sha256};
pub use clipboard::ClipboardMonitor;
pub use db::{Database, DownloadRecord};
pub use downloader::{DownloadOptions, DownloadProgress, DownloadStatus, Downloader};
pub use error::{DownloadError, Result};
pub use probe::{extract_filename, probe_url, resolve_unique_path, sanitize_filename, FileInfo};
pub use queue::{QueueEvent, QueueManager};
pub use rate_limiter::TokenBucketRateLimiter;
pub use segment::{calculate_segments, Segment};
pub use server::{BridgeServer, ExtensionDownloadRequest, spawn_on_demand_popup, get_or_create_bridge_token};
pub use speed::SpeedTracker;
pub use state::DownloadState;
pub use youtube::{CompactVideoFormats, VideoQualityOption, query_video_formats, get_cached_video_formats, format_quality_size};

pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

use std::sync::OnceLock;

static LOG_CALLBACK: OnceLock<Box<dyn Fn(&str) + Send + Sync>> = OnceLock::new();

pub fn set_log_callback(cb: impl Fn(&str) + Send + Sync + 'static) {
    let _ = LOG_CALLBACK.set(Box::new(cb));
}

pub fn log_event(msg: &str) {
    if let Some(cb) = LOG_CALLBACK.get() {
        cb(msg);
    } else {
        tracing::info!("{}", msg);
    }
}

