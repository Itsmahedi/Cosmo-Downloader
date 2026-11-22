use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{mpsc, Mutex};
use tokio::time::sleep;
use tokio_util::sync::CancellationToken;
use tracing::{error, info};

use librqbit::{
    torrent_from_bytes, AddTorrent, AddTorrentOptions, Magnet, ManagedTorrent,
    Session, SessionOptions, TorrentStatsState,
};

use crate::downloader::{DownloadProgress, DownloadStatus};
use crate::error::{DownloadError, Result};

/// Global or shared session instance holder.
static TORRENT_SESSION: Mutex<Option<Arc<Session>>> = Mutex::const_new(None);

/// Check if a given string represents a BitTorrent source (magnet URI, .torrent file URL or local path).
pub fn is_torrent_url(url_or_path: &str) -> bool {
    let trimmed = url_or_path.trim();
    if is_magnet_link(trimmed) {
        return true;
    }

    // Check if it's a URL ending with .torrent (allowing query params)
    if let Ok(parsed) = url::Url::parse(trimmed) {
        let path = parsed.path().to_ascii_lowercase();
        if path.ends_with(".torrent") {
            return true;
        }
    }

    // Check if it's a local file ending with .torrent
    let p = Path::new(trimmed);
    if let Some(ext) = p.extension().and_then(|e| e.to_str()) {
        if ext.eq_ignore_ascii_case("torrent") {
            return true;
        }
    }

    false
}

/// Check if a string is a magnet link.
pub fn is_magnet_link(raw: &str) -> bool {
    let trimmed = raw.trim();
    trimmed.starts_with("magnet:?") || trimmed.starts_with("magnet:")
}

/// Extract a readable filename from a magnet link or .torrent path/URL.
pub fn extract_torrent_name(url_or_path: &str) -> String {
    let trimmed = url_or_path.trim();
    if is_magnet_link(trimmed) {
        if let Ok(magnet) = Magnet::parse(trimmed) {
            if let Some(name) = magnet.name {
                let cleaned = name.trim().to_string();
                if !cleaned.is_empty() {
                    return crate::probe::sanitize_filename(&cleaned);
                }
            }
        }
        // Fallback to checking dn= parameter manually
        if let Some(idx) = trimmed.find("dn=") {
            let after = &trimmed[idx + 3..];
            let end = after.find('&').unwrap_or(after.len());
            let raw_name = &after[..end];
            if let Ok(decoded) = percent_encoding::percent_decode_str(raw_name).decode_utf8() {
                let s = decoded.trim();
                if !s.is_empty() {
                    return crate::probe::sanitize_filename(s);
                }
            }
        }
        return "Torrent_Download".to_string();
    }

    // If it's a URL ending in .torrent
    if let Ok(parsed) = url::Url::parse(trimmed) {
        if let Some(mut segs) = parsed.path_segments() {
            if let Some(last) = segs.next_back() {
                if !last.trim().is_empty() {
                    let cleaned = last.trim_end_matches(".torrent");
                    return crate::probe::sanitize_filename(cleaned);
                }
            }
        }
    }

    // If it's a local filesystem path
    let p = Path::new(trimmed);
    if let Some(stem) = p.file_stem().and_then(|s| s.to_str()) {
        if !stem.trim().is_empty() {
            return crate::probe::sanitize_filename(stem);
        }
    }

    "Torrent_Download".to_string()
}

/// Read torrent metadata from a local .torrent file synchronously.
/// Returns (content_name, total_bytes) if successful.
pub fn read_torrent_file_metadata(url_or_path: &str) -> Option<(String, u64)> {
    let trimmed = url_or_path.trim().trim_matches('"').trim_matches('\'');
    let path_str = if let Some(stripped) = trimmed.strip_prefix("file:///") {
        stripped
    } else if let Some(stripped) = trimmed.strip_prefix("file://") {
        stripped
    } else {
        trimmed
    };

    let p = Path::new(path_str);
    if p.is_file() {
        if let Ok(bytes) = std::fs::read(p) {
            if let Ok(parsed) = torrent_from_bytes(&bytes) {
                if let Ok(validated) = parsed.info.data.validate() {
                    let total_bytes = validated.lengths().total_length();
                    let name = validated
                        .name()
                        .map(|n| n.to_string())
                        .filter(|n| !n.trim().is_empty())
                        .unwrap_or_else(|| extract_torrent_name(url_or_path));
                    return Some((crate::probe::sanitize_filename(&name), total_bytes));
                }
            }
        }
    }
    None
}

/// Resolve the default storage path for the BitTorrent session state.
fn default_session_dir() -> PathBuf {
    directories::ProjectDirs::from("com", "CosmoDownloader", "CosmoDownloader")
        .map(|dirs| dirs.data_local_dir().join("torrents_session"))
        .unwrap_or_else(|| std::env::temp_dir().join("cosmo_torrents_session"))
}

/// Pre-warm the BitTorrent session in the background so DHT & listening sockets are ready before user clicks download.
pub async fn warm_up_torrent_session() {
    let _ = get_or_init_session(None).await;
}

/// Get or initialize the shared `librqbit::Session`.
pub async fn get_or_init_session(default_output: Option<&Path>) -> Result<Arc<Session>> {
    let mut guard = TORRENT_SESSION.lock().await;
    if let Some(ref session) = *guard {
        return Ok(Arc::clone(session));
    }

    let session_dir = default_session_dir();
    let _ = tokio::fs::create_dir_all(&session_dir).await;

    let output_dir = default_output
        .map(PathBuf::from)
        .unwrap_or_else(|| session_dir.clone());
    let _ = tokio::fs::create_dir_all(&output_dir).await;

    info!(
        "Initializing BitTorrent session at output='{}', session_state='{}'",
        output_dir.display(),
        session_dir.display()
    );

    let session = Session::new_with_opts(output_dir, SessionOptions::default())
        .await
        .map_err(|e| DownloadError::Internal(format!("Failed to create BitTorrent session: {:#}", e)))?;

    *guard = Some(Arc::clone(&session));
    Ok(session)
}

/// Download a torrent or magnet link to completion while broadcasting progress.
pub async fn download_torrent(
    url_or_path: &str,
    target_path: &Path,
    progress_tx: mpsc::Sender<DownloadProgress>,
    cancel_token: CancellationToken,
) -> Result<()> {
    // Determine target output folder:
    // If target_path has an extension (like .mp4 or .zip) or is a file name, use its parent folder.
    let output_folder = if target_path.is_dir() {
        target_path.to_path_buf()
    } else if let Some(parent) = target_path.parent() {
        if parent.as_os_str().is_empty() {
            PathBuf::from(".")
        } else {
            parent.to_path_buf()
        }
    } else {
        PathBuf::from(".")
    };

    let _ = tokio::fs::create_dir_all(&output_folder).await;

    let session = get_or_init_session(Some(&output_folder)).await?;

    let add_source = AddTorrent::from_cli_argument(url_or_path)
        .map_err(|e| DownloadError::InvalidUrl(format!("Failed to parse torrent/magnet source: {:#}", e)))?;

    let opts = AddTorrentOptions {
        output_folder: Some(output_folder.to_string_lossy().to_string()),
        overwrite: true,
        ..Default::default()
    };

    info!(
        "Adding BitTorrent task: '{}' -> output folder: '{}'",
        url_or_path,
        output_folder.display()
    );

    let add_response = session
        .add_torrent(add_source, Some(opts))
        .await
        .map_err(|e| DownloadError::Internal(format!("Failed to add torrent to session: {:#}", e)))?;

    let handle: Arc<ManagedTorrent> = add_response
        .into_handle()
        .ok_or_else(|| DownloadError::Internal("BitTorrent handle could not be acquired".to_string()))?;

    // Determine initial total bytes from handle stats or local file metadata
    let initial_total_bytes = {
        let stats = handle.stats();
        if stats.total_bytes > 0 {
            Some(stats.total_bytes)
        } else {
            read_torrent_file_metadata(url_or_path).map(|(_, sz)| sz)
        }
    };

    // Initial progress report (initializing / metadata fetch)
    let _ = progress_tx
        .send(DownloadProgress {
            status: DownloadStatus::Downloading,
            total_bytes: initial_total_bytes,
            downloaded_bytes: 0,
            speed_bps: 0.0,
            eta_seconds: None,
            segments: Vec::new(),
            file_path: target_path.to_path_buf(),
        })
        .await;

    let mut is_first_poll = true;
    loop {
        if cancel_token.is_cancelled() {
            info!("BitTorrent download task cancelled by user");
            let _ = session.pause(&handle).await;
            return Err(DownloadError::Cancelled);
        }

        let stats = handle.stats();
        let total_bytes = if stats.total_bytes > 0 {
            Some(stats.total_bytes)
        } else {
            initial_total_bytes
        };
        let downloaded_bytes = stats.progress_bytes;

        // Calculate download speed in bytes/sec from librqbit Speed (mbps)
        let speed_bps = if let Some(ref live) = stats.live {
            // live.download_speed.mbps is in megabits per sec: 1 mbps = 125,000 bytes/sec
            live.download_speed.mbps * 125_000.0
        } else {
            0.0
        };

        // Calculate ETA
        let eta_seconds = if let (Some(total), true) = (total_bytes, speed_bps > 0.0) {
            let remaining = total.saturating_sub(downloaded_bytes);
            Some((remaining as f64 / speed_bps) as u64)
        } else {
            None
        };

        match stats.state {
            TorrentStatsState::Error => {
                let err_msg = stats
                    .error
                    .as_deref()
                    .unwrap_or("Unknown BitTorrent protocol error");
                error!("BitTorrent error: {}", err_msg);
                return Err(DownloadError::Internal(err_msg.to_string()));
            }
            _ => {}
        }

        if stats.finished || (total_bytes.is_some() && Some(downloaded_bytes) >= total_bytes) {
            info!(
                "BitTorrent download complete: {} bytes downloaded",
                downloaded_bytes
            );
            let _ = progress_tx
                .send(DownloadProgress {
                    status: DownloadStatus::Completed,
                    total_bytes: Some(downloaded_bytes),
                    downloaded_bytes,
                    speed_bps: 0.0,
                    eta_seconds: Some(0),
                    segments: Vec::new(),
                    file_path: target_path.to_path_buf(),
                })
                .await;
            return Ok(());
        }

        // Send periodic update
        let _ = progress_tx
            .send(DownloadProgress {
                status: DownloadStatus::Downloading,
                total_bytes,
                downloaded_bytes,
                speed_bps,
                eta_seconds,
                segments: Vec::new(),
                file_path: target_path.to_path_buf(),
            })
            .await;

        let sleep_duration = if is_first_poll {
            is_first_poll = false;
            Duration::from_millis(100)
        } else {
            Duration::from_millis(500)
        };

        tokio::select! {
            _ = cancel_token.cancelled() => {
                info!("BitTorrent download task cancelled by user during sleep");
                let _ = session.pause(&handle).await;
                return Err(DownloadError::Cancelled);
            }
            _ = sleep(sleep_duration) => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_torrent_url() {
        assert!(is_torrent_url("magnet:?xt=urn:btih:c12fe1c06bba254a9dc9f519b335380dc1f74c26"));
        assert!(is_torrent_url("https://releases.ubuntu.com/24.04/ubuntu-24.04-desktop-amd64.iso.torrent"));
        assert!(is_torrent_url("https://example.com/downloads/test.torrent?token=abc"));
        assert!(is_torrent_url("/home/user/downloads/archive.torrent"));
        assert!(is_torrent_url("C:\\Users\\user\\Downloads\\movie.torrent"));

        assert!(!is_torrent_url("https://example.com/file.zip"));
        assert!(!is_torrent_url("https://www.youtube.com/watch?v=dQw4w9WgXcQ"));
    }

    #[test]
    fn test_extract_torrent_name() {
        let mag = "magnet:?xt=urn:btih:c12fe1c06bba254a9dc9f519b335380dc1f74c26&dn=Ubuntu+24.04+Desktop&tr=http://tracker.example.com";
        assert_eq!(extract_torrent_name(mag), "Ubuntu 24.04 Desktop");

        let mag_encoded = "magnet:?xt=urn:btih:c12fe1c06bba254a9dc9f519b335380dc1f74c26&dn=My%20Awesome%20Movie";
        assert_eq!(extract_torrent_name(mag_encoded), "My Awesome Movie");

        let torrent_url = "https://example.com/distro/archlinux-2026.iso.torrent";
        assert_eq!(extract_torrent_name(torrent_url), "archlinux-2026.iso");

        let local_path = "/home/user/downloads/Debian-12.torrent";
        assert_eq!(extract_torrent_name(local_path), "Debian-12");
    }

    #[test]
    fn test_read_torrent_file_metadata() {
        // Non-existent path returns None gracefully
        assert!(read_torrent_file_metadata("/path/does/not/exist.torrent").is_none());

        // Check if cached test torrent exists in cargo registry
        let sample = "/home/steam/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/librqbit-9.1.0/resources/ubuntu-21.04-desktop-amd64.iso.torrent";
        if Path::new(sample).exists() {
            let res = read_torrent_file_metadata(sample);
            assert!(res.is_some());
            let (name, size) = res.unwrap();
            assert_eq!(name, "ubuntu-21.04-desktop-amd64.iso");
            assert!(size > 0);
        }

        let safari = "/mnt/c/Users/steam/Downloads/Safari-TV.v0.7.59-OFME.torrent";
        if Path::new(safari).exists() {
            let res = read_torrent_file_metadata(safari);
            assert!(res.is_some());
        }
    }
}
