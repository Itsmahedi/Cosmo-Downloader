use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use tracing::{info, warn};

use crate::downloader::{DownloadProgress, DownloadStatus};
use crate::error::{DownloadError, Result};

/// Detect whether a URL is from a known video streaming platform (YouTube, Vimeo, TikTok, etc.)
pub fn is_streaming_site(url: &str) -> bool {
    let lower = url.to_ascii_lowercase();
    lower.contains("youtube.com/")
        || lower.contains("youtu.be/")
        || lower.contains("vimeo.com/")
        || lower.contains("tiktok.com/")
        || lower.contains("twitter.com/")
        || lower.contains("x.com/")
        || lower.contains("facebook.com/")
        || lower.contains("instagram.com/")
        || lower.contains("reddit.com/")
}

/// Normalizes streaming URLs: if the URL has a "v" query parameter, keeps only the single
/// video by dropping playlist parameters (list, start_radio, index).
/// Real playlist URLs (e.g. /playlist?list=...) without a "v" parameter are kept intact.
pub fn normalize_streaming_url(raw_url: &str) -> String {
    let trimmed = raw_url.trim();
    if let Ok(mut parsed) = url::Url::parse(trimmed) {
        let has_v = parsed.query_pairs().any(|(k, _)| k == "v");
        if has_v {
            let filtered: Vec<(String, String)> = parsed
                .query_pairs()
                .filter(|(k, _)| k != "list" && k != "start_radio" && k != "index")
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect();
            parsed.set_query(None);
            if !filtered.is_empty() {
                let mut query = String::new();
                for (i, (k, v)) in filtered.iter().enumerate() {
                    if i > 0 {
                        query.push('&');
                    }
                    query.push_str(k);
                    query.push('=');
                    query.push_str(v);
                }
                parsed.set_query(Some(&query));
            }
            return parsed.to_string();
        }
    }
    trimmed.to_string()
}

/// Normalizes a URL into a deduplication key.
/// - For YouTube: extracts video ID (from "v" query param, youtu.be path, /shorts/, /embed/)
///   and ignores list, start_radio, index, t, si, feature, and any other tracking parameters.
///   Returns e.g. "youtube:<video_id>".
/// - For other sites: lowercases host/scheme, removes fragment (#...), and strips
///   known tracking query parameters (utm_*, fbclid, gclid, msclkid, si, ref, feature, etc.).
/// - If invalid or empty: returns trimmed string.
pub fn normalize_url_key(raw_url: &str) -> String {
    let trimmed = raw_url.trim();
    if trimmed.is_empty() {
        return String::new();
    }

    if let Ok(mut parsed) = url::Url::parse(trimmed) {
        let host = parsed.host_str().unwrap_or("").to_ascii_lowercase();

        // 1. YouTube normalization
        if host == "youtube.com" || host.ends_with(".youtube.com") || host == "youtu.be" {
            // Check for ?v=...
            if let Some((_, vid)) = parsed.query_pairs().find(|(k, _)| k == "v") {
                if !vid.trim().is_empty() {
                    return format!("youtube:{}", vid.trim());
                }
            }

            // Check for youtu.be/<video_id>
            if host == "youtu.be" {
                let path = parsed.path().trim_start_matches('/');
                let vid = path.split('/').next().unwrap_or("");
                if !vid.is_empty() {
                    return format!("youtube:{}", vid);
                }
            }

            // Check for /shorts/<video_id>, /embed/<video_id>, /v/<video_id>, /live/<video_id>
            let path = parsed.path();
            for prefix in &["/shorts/", "/embed/", "/v/", "/live/"] {
                if path.starts_with(prefix) {
                    let vid = path.trim_start_matches(prefix).split('/').next().unwrap_or("");
                    if !vid.is_empty() {
                        return format!("youtube:{}", vid);
                    }
                }
            }
        }

        // 2. Other sites normalization
        parsed.set_fragment(None);

        let tracking_params = [
            "utm_source", "utm_medium", "utm_campaign", "utm_term", "utm_content", "utm_id",
            "fbclid", "gclid", "msclkid", "mc_cid", "mc_eid", "_ga", "_gl", "si", "ref", "source", "feature"
        ];

        let filtered_pairs: Vec<(String, String)> = parsed
            .query_pairs()
            .filter(|(k, _)| !tracking_params.contains(&k.to_ascii_lowercase().as_str()))
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();

        parsed.set_query(None);
        if !filtered_pairs.is_empty() {
            let mut q = String::new();
            for (i, (k, v)) in filtered_pairs.iter().enumerate() {
                if i > 0 {
                    q.push('&');
                }
                q.push_str(k);
                q.push('=');
                q.push_str(v);
            }
            parsed.set_query(Some(&q));
        }

        let mut res = parsed.to_string();
        if res.ends_with('/') && parsed.path() != "/" && parsed.query().is_none() {
            res.pop();
        }
        return res;
    }

    trimmed.to_string()
}

/// Locate yt-dlp binary on the system.
pub fn find_ytdlp_binary() -> Option<PathBuf> {
    // 1. Next to current executable
    if let Ok(mut exe) = std::env::current_exe() {
        exe.pop();
        let candidate = exe.join("yt-dlp.exe");
        if candidate.exists() {
            return Some(candidate);
        }
    }

    // 2. In %LOCALAPPDATA%\Programs\Cosmo Downloader\yt-dlp.exe (or legacy)
    if let Some(local_app_data) = std::env::var_os("LOCALAPPDATA") {
        let base = PathBuf::from(local_app_data).join("Programs");
        let cosmo_spaced = base.join("Cosmo Downloader").join("yt-dlp.exe");
        if cosmo_spaced.exists() {
            return Some(cosmo_spaced);
        }
        let cosmo = base.join("CosmoDownloader").join("yt-dlp.exe");
        if cosmo.exists() {
            return Some(cosmo);
        }
        let legacy = base.join("RustGet").join("yt-dlp.exe");
        if legacy.exists() {
            return Some(legacy);
        }
    }

    // 3. Current working directory and parent directories (workspace roots)
    let candidate = PathBuf::from("yt-dlp.exe");
    if candidate.exists() {
        return Some(candidate);
    }
    let candidate_parent = PathBuf::from("../yt-dlp.exe");
    if candidate_parent.exists() {
        return Some(candidate_parent);
    }
    let candidate_root = PathBuf::from("../../yt-dlp.exe");
    if candidate_root.exists() {
        return Some(candidate_root);
    }

    // 4. Search in PATH directories
    if let Some(path_var) = std::env::var_os("PATH") {
        for dir in std::env::split_paths(&path_var) {
            let candidate = dir.join("yt-dlp.exe");
            if candidate.exists() {
                return Some(candidate);
            }
            let candidate_bare = dir.join("yt-dlp");
            if candidate_bare.exists() {
                return Some(candidate_bare);
            }
        }
    }

    None
}

/// Locate ffmpeg binary on the system.
pub fn find_ffmpeg_binary() -> Option<PathBuf> {
    // 1. Next to current executable
    if let Ok(mut exe) = std::env::current_exe() {
        exe.pop();
        let candidate = exe.join("ffmpeg.exe");
        if candidate.exists() {
            return Some(candidate);
        }
    }

    // 2. Next to yt-dlp binary if found
    if let Some(ytdlp) = find_ytdlp_binary() {
        if let Some(parent) = ytdlp.parent() {
            let candidate = parent.join("ffmpeg.exe");
            if candidate.exists() {
                return Some(candidate);
            }
        }
    }

    // 3. In %LOCALAPPDATA%\Programs\Cosmo Downloader\ffmpeg.exe (or legacy)
    if let Some(local_app_data) = std::env::var_os("LOCALAPPDATA") {
        let base = PathBuf::from(local_app_data).join("Programs");
        for app_folder in ["Cosmo Downloader", "CosmoDownloader", "RustGet"] {
            let candidate = base.join(app_folder).join("ffmpeg.exe");
            if candidate.exists() {
                return Some(candidate);
            }
        }
    }

    // 4. In common external locations (e.g. ShareX)
    let sharex_candidate = PathBuf::from("C:\\Program Files\\ShareX\\ffmpeg.exe");
    if sharex_candidate.exists() {
        return Some(sharex_candidate);
    }
    let sharex_x86_candidate = PathBuf::from("C:\\Program Files (x86)\\ShareX\\ffmpeg.exe");
    if sharex_x86_candidate.exists() {
        return Some(sharex_x86_candidate);
    }

    // 5. Current working directory and parent directories (workspace roots)
    for candidate in [PathBuf::from("ffmpeg.exe"), PathBuf::from("../ffmpeg.exe"), PathBuf::from("../../ffmpeg.exe")] {
        if candidate.exists() {
            return Some(candidate);
        }
    }

    // 6. Search in PATH directories
    if let Some(path_var) = std::env::var_os("PATH") {
        for dir in std::env::split_paths(&path_var) {
            let candidate = dir.join("ffmpeg.exe");
            if candidate.exists() {
                return Some(candidate);
            }
            let candidate_bare = dir.join("ffmpeg");
            if candidate_bare.exists() {
                return Some(candidate_bare);
            }
        }
    }

    None
}

/// Query video title asynchronously using yt-dlp.
pub async fn fetch_video_title(bin: &Path, url: &str) -> Option<String> {
    #[cfg(windows)]
    const CREATE_NO_WINDOW: u32 = 0x08000000;

    let mut cmd = Command::new(bin);
    cmd.env("PYTHONIOENCODING", "utf-8");
    cmd.env("PYTHONUTF8", "1");
    cmd.args(["--encoding", "utf-8", "--dump-single-json", "--no-playlist", "--no-warnings", url]);

    #[cfg(windows)]
    cmd.creation_flags(CREATE_NO_WINDOW);

    let output = cmd.output().await.ok()?;
    if output.status.success() {
        if let Ok(json) = serde_json::from_slice::<serde_json::Value>(&output.stdout) {
            if let Some(title) = json.get("title").and_then(|v| v.as_str()) {
                let trimmed = title.trim();
                if trimmed.contains('\u{FFFD}') {
                    tracing::warn!("fetch_video_title: decoded JSON title contains U+FFFD: '{}'", trimmed);
                }
                let codepoints: Vec<String> = trimmed.chars().map(|c| format!("U+{:04X}", c as u32)).collect();
                info!("yt-dlp JSON title '{}' code points: {}", trimmed, codepoints.join(" "));
                if !trimmed.is_empty() {
                    return Some(trimmed.to_string());
                }
            }
        }

        let text = String::from_utf8(output.stdout.clone())
            .unwrap_or_else(|_| String::from_utf8_lossy(&output.stdout).to_string());
        let trimmed = text.lines().next()?.trim().to_string();
        let codepoints: Vec<String> = trimmed.chars().map(|c| format!("U+{:04X}", c as u32)).collect();
        info!("yt-dlp title '{}' code points: {}", trimmed, codepoints.join(" "));
        if !trimmed.is_empty() {
            return Some(trimmed);
        }
    }
    None
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct StreamingVideoInfo {
    pub title: String,
    pub filename: String,
    pub filesize_approx: Option<u64>,
}

/// Query video title and approximate size asynchronously using yt-dlp with a 15s timeout.
pub async fn query_streaming_video_info(url: &str) -> std::result::Result<StreamingVideoInfo, String> {
    let ytdlp = find_ytdlp_binary().ok_or_else(|| "yt-dlp binary not found".to_string())?;

    let temp_dir = std::env::temp_dir().join(format!(
        "cosmo_probe_{}_{}",
        std::process::id(),
        uuid::Uuid::new_v4()
    ));
    let _ = tokio::fs::create_dir_all(&temp_dir).await;

    #[cfg(windows)]
    const CREATE_NO_WINDOW: u32 = 0x08000000;

    let mut cmd = Command::new(&ytdlp);
    cmd.current_dir(&temp_dir);
    cmd.stdin(std::process::Stdio::null());
    cmd.stdout(std::process::Stdio::piped());
    cmd.stderr(std::process::Stdio::piped());
    cmd.env("PYTHONIOENCODING", "utf-8");
    cmd.env("PYTHONUTF8", "1");
    cmd.args([
        "--encoding", "utf-8",
        "--dump-single-json",
        "--no-playlist",
        "--no-warnings",
        url,
    ]);

    #[cfg(windows)]
    cmd.creation_flags(CREATE_NO_WINDOW);

    let output_res = tokio::time::timeout(Duration::from_secs(15), cmd.output()).await;
    let _ = tokio::fs::remove_dir_all(&temp_dir).await;

    match output_res {
        Ok(Ok(output)) => {
            if output.status.success() {
                let mut raw_title = String::new();
                let mut size: Option<u64> = None;

                if let Ok(json) = serde_json::from_slice::<serde_json::Value>(&output.stdout) {
                    if let Some(t) = json.get("title").and_then(|v| v.as_str()) {
                        raw_title = t.trim().to_string();
                    }
                    size = json.get("filesize").and_then(|v| v.as_u64())
                        .or_else(|| json.get("filesize_approx").and_then(|v| v.as_u64()));
                }

                if raw_title.is_empty() {
                    let stdout = String::from_utf8(output.stdout.clone())
                        .unwrap_or_else(|_| String::from_utf8_lossy(&output.stdout).to_string());
                    let mut lines = stdout.lines();
                    raw_title = lines.next().unwrap_or("").trim().to_string();
                    let raw_size_str = lines.next().unwrap_or("").trim().to_string();
                    if size.is_none() {
                        size = raw_size_str.parse::<u64>().ok();
                    }
                }

                if raw_title.contains('\u{FFFD}') {
                    tracing::warn!("query_streaming_video_info: title contains U+FFFD: '{}'", raw_title);
                }

                let title = if !raw_title.is_empty() {
                    let mut t = raw_title.as_str().trim();
                    if t.starts_with('(') {
                        if let Some(close_idx) = t.find(')') {
                            let inside = &t[1..close_idx];
                            if !inside.is_empty() && inside.chars().all(|c| c.is_ascii_digit()) {
                                t = t[close_idx + 1..].trim_start();
                            }
                        }
                    }
                    for suffix in &[" - YouTube", " | YouTube", " - youtube", " | youtube"] {
                        if let Some(idx) = t.rfind(suffix) {
                            if idx + suffix.len() == t.len() {
                                t = &t[..idx];
                                break;
                            }
                        }
                    }
                    t.trim().to_string()
                } else {
                    "YouTube_Video".to_string()
                };
                let sanitized = crate::probe::sanitize_filename(&title);
                let filename = if sanitized.contains('.') {
                    sanitized
                } else {
                    format!("{}.mp4", sanitized)
                };

                Ok(StreamingVideoInfo {
                    title,
                    filename,
                    filesize_approx: size,
                })
            } else {
                let stderr = String::from_utf8_lossy(&output.stderr);
                let err_msg = if !stderr.trim().is_empty() {
                    stderr.trim().to_string()
                } else {
                    "Failed to extract video info".to_string()
                };
                Err(err_msg)
            }
        }
        Ok(Err(e)) => Err(format!("Process error: {}", e)),
        Err(_) => Err("Video info lookup timed out after 15s".to_string()),
    }
}

/// Download a streaming video (e.g. YouTube) with progress tracking and cancellation.
pub async fn download_streaming_video(
    bin: &Path,
    url: &str,
    output_path: &Path,
    cancel: CancellationToken,
    progress_tx: mpsc::Sender<DownloadProgress>,
) -> Result<PathBuf> {
    #[cfg(windows)]
    const CREATE_NO_WINDOW: u32 = 0x08000000;

    if let Some(parent) = output_path.parent() {
        let _ = tokio::fs::create_dir_all(parent).await;
    }

    let target_ext = output_path.extension().and_then(|s| s.to_str()).unwrap_or("");
    let is_audio = target_ext.eq_ignore_ascii_case("mp3");

    let mut final_path = output_path.to_path_buf();
    let fname = final_path.file_name().and_then(|s| s.to_str()).unwrap_or("").to_string();
    if fname.eq_ignore_ascii_case("watch") || fname.is_empty() {
        let parent = final_path.parent().unwrap_or(Path::new("."));
        let default_name = if is_audio { "YouTube_Audio.mp3" } else { "YouTube_Video.mp4" };
        final_path = parent.join(default_name);
    } else if !fname.contains('.') {
        final_path.set_extension(if is_audio { "mp3" } else { "mp4" });
    }
    let out_template = final_path.to_string_lossy().to_string();

    let mut args = vec![
        "-N".to_string(), "8".to_string(),
        "--newline".to_string(),
        "--progress-template".to_string(), "%(progress.downloaded_bytes)s/%(progress.total_bytes)s/%(progress.speed)s".to_string(),
    ];

    let ffmpeg_bin = find_ffmpeg_binary();
    if let Some(ref ffmpeg_path) = ffmpeg_bin {
        args.push("--ffmpeg-location".to_string());
        args.push(ffmpeg_path.to_string_lossy().to_string());
    }

    if is_audio {
        if ffmpeg_bin.is_some() {
            args.push("-x".to_string());
            args.push("--audio-format".to_string());
            args.push("mp3".to_string());
        } else {
            args.push("-f".to_string());
            args.push("bestaudio/best".to_string());
        }
    } else {
        if ffmpeg_bin.is_some() {
            args.push("-f".to_string());
            args.push("bestvideo[ext=mp4]+bestaudio[ext=m4a]/bestvideo+bestaudio/best[ext=mp4]/best".to_string());
            args.push("--merge-output-format".to_string());
            args.push("mp4".to_string());
        } else {
            // Fallback when ffmpeg is not available: download single combined stream
            // so video and audio are not separated
            args.push("-f".to_string());
            args.push("best[ext=mp4]/best".to_string());
        }
    }

    args.push("-o".to_string());
    args.push(out_template);
    args.push(url.to_string());

    let mut cmd = Command::new(bin);
    cmd.env("PYTHONIOENCODING", "utf-8");
    cmd.env("PYTHONUTF8", "1");
    cmd.args(&args);

    cmd.stdout(std::process::Stdio::piped());
    cmd.stderr(std::process::Stdio::piped());

    #[cfg(windows)]
    cmd.creation_flags(CREATE_NO_WINDOW);

    let mut child = cmd.spawn().map_err(|e| {
        DownloadError::Io(std::io::Error::other(format!("Failed to spawn yt-dlp: {}", e)))
    })?;

    let stdout = child.stdout.take().ok_or_else(|| {
        DownloadError::Io(std::io::Error::other("Failed to capture yt-dlp stdout"))
    })?;

    let mut reader = BufReader::new(stdout).lines();
    let mut last_progress_send = Instant::now();
    let mut downloaded = 0u64;
    let mut total = None;
    let mut speed = 0.0f64;

    loop {
        tokio::select! {
            _ = cancel.cancelled() => {
                let _ = child.kill().await;
                return Err(DownloadError::Cancelled);
            }
            line_res = reader.next_line() => {
                match line_res {
                    Ok(Some(line)) => {
                        let trimmed = line.trim();
                        // Format: <downloaded_bytes>/<total_bytes>/<speed_bps>
                        if let Some((dl_str, rest)) = trimmed.split_once('/') {
                            if let Some((tot_str, spd_str)) = rest.split_once('/') {
                                if let Ok(dl) = dl_str.parse::<u64>() {
                                    downloaded = dl;
                                }
                                if let Ok(tot) = tot_str.parse::<u64>() {
                                    if tot > 0 {
                                        total = Some(tot);
                                    }
                                }
                                if let Ok(spd) = spd_str.parse::<f64>() {
                                    speed = spd;
                                }

                                if last_progress_send.elapsed() >= std::time::Duration::from_millis(100) {
                                    let eta_seconds = match total {
                                        Some(tot) if speed > 0.0 => {
                                            let remaining = tot.saturating_sub(downloaded);
                                            Some((remaining as f64 / speed) as u64)
                                        }
                                        _ => None,
                                    };

                                    let _ = progress_tx.send(DownloadProgress {
                                        status: DownloadStatus::Downloading,
                                        downloaded_bytes: downloaded,
                                        total_bytes: total,
                                        speed_bps: speed,
                                        eta_seconds,
                                        segments: Vec::new(),
                                        file_path: output_path.to_path_buf(),
                                    }).await;

                                    last_progress_send = Instant::now();
                                }
                            }
                        }
                    }
                    Ok(None) => break,
                    Err(e) => {
                        warn!("Error reading yt-dlp output line: {}", e);
                        break;
                    }
                }
            }
        }
    }

    let status = child.wait().await.map_err(DownloadError::Io)?;
    if !status.success() {
        return Err(DownloadError::Io(std::io::Error::other(
            format!("yt-dlp exited with status code: {:?}", status.code()),
        )));
    }

    // Determine actual saved file on disk (yt-dlp may merge or append extension)
    let actual_saved_path = if final_path.exists() {
        final_path
    } else {
        let mut resolved = output_path.to_path_buf();
        if let Some(parent) = final_path.parent() {
            if let Some(stem) = final_path.file_stem().and_then(|s| s.to_str()) {
                if let Ok(entries) = std::fs::read_dir(parent) {
                    for entry in entries.flatten() {
                        let p = entry.path();
                        if p.is_file() {
                            if let Some(name) = p.file_name().and_then(|n| n.to_str()) {
                                if name.starts_with(stem) && !name.ends_with(".part") && !name.ends_with(".ytdl") {
                                    resolved = p;
                                    break;
                                }
                            }
                        }
                    }
                }
            }
        }
        resolved
    };

    let actual_size = std::fs::metadata(&actual_saved_path).map(|m| m.len()).unwrap_or(total.unwrap_or(downloaded));

    // Final 100% progress event with the actual path
    let _ = progress_tx.send(DownloadProgress {
        status: DownloadStatus::Completed,
        downloaded_bytes: actual_size,
        total_bytes: Some(actual_size),
        speed_bps: 0.0,
        eta_seconds: Some(0),
        segments: Vec::new(),
        file_path: actual_saved_path.clone(),
    }).await;

    info!("Streaming video download completed: {:?}", actual_saved_path);
    Ok(actual_saved_path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_streaming_site() {
        assert!(is_streaming_site("https://www.youtube.com/watch?v=dQw4w9WgXcQ"));
        assert!(is_streaming_site("https://youtu.be/dQw4w9WgXcQ"));
        assert!(is_streaming_site("https://vimeo.com/12345678"));
        assert!(is_streaming_site("https://www.tiktok.com/@user/video/123"));
        assert!(!is_streaming_site("https://example.com/files/archive.zip"));
        assert!(!is_streaming_site("https://filecr.com/file-download/?id=123"));
    }

    #[test]
    fn test_find_ytdlp_binary() {
        let bin = find_ytdlp_binary();
        assert!(bin.is_some());
    }

    #[test]
    fn test_find_ffmpeg_binary() {
        let bin = find_ffmpeg_binary();
        assert!(bin.is_some());
    }

    #[test]
    fn test_normalize_streaming_url() {
        let url_with_list = "https://www.youtube.com/watch?v=qpS7tc1KVwI&list=RDqpS7tc1KVwI&start_radio=1";
        assert_eq!(
            normalize_streaming_url(url_with_list),
            "https://www.youtube.com/watch?v=qpS7tc1KVwI"
        );
        let playlist_only = "https://www.youtube.com/playlist?list=PL123456789";
        assert_eq!(
            normalize_streaming_url(playlist_only),
            "https://www.youtube.com/playlist?list=PL123456789"
        );
    }

    #[test]
    fn test_normalize_url_key() {
        // YouTube variants
        assert_eq!(
            normalize_url_key("https://www.youtube.com/watch?v=B402rKl4bUg"),
            "youtube:B402rKl4bUg"
        );
        assert_eq!(
            normalize_url_key("https://www.youtube.com/watch?v=B402rKl4bUg&list=RD123&index=2&t=45s&si=abc"),
            "youtube:B402rKl4bUg"
        );
        assert_eq!(
            normalize_url_key("https://youtu.be/B402rKl4bUg?si=xyz&feature=share"),
            "youtube:B402rKl4bUg"
        );
        assert_eq!(
            normalize_url_key("https://www.youtube.com/shorts/B402rKl4bUg?feature=share"),
            "youtube:B402rKl4bUg"
        );
        assert_eq!(
            normalize_url_key("https://www.youtube.com/embed/B402rKl4bUg"),
            "youtube:B402rKl4bUg"
        );
        assert_eq!(
            normalize_url_key("https://music.youtube.com/watch?v=B402rKl4bUg"),
            "youtube:B402rKl4bUg"
        );
        assert_eq!(
            normalize_url_key("https://m.youtube.com/watch?v=B402rKl4bUg"),
            "youtube:B402rKl4bUg"
        );
        assert_eq!(
            normalize_url_key("https://www.youtube.com/live/B402rKl4bUg"),
            "youtube:B402rKl4bUg"
        );

        // Non-YouTube sites
        assert_eq!(
            normalize_url_key("https://Example.COM/file.iso#part1"),
            "https://example.com/file.iso"
        );
        assert_eq!(
            normalize_url_key("https://EXAMPLE.COM/files/file.iso?utm_source=twitter&utm_medium=social&si=123&token=abc"),
            "https://example.com/files/file.iso?token=abc"
        );
        assert_eq!(normalize_url_key(""), "");
    }
}
