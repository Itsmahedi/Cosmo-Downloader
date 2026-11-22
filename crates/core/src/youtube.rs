use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use tracing::{info, warn};

use crate::downloader::{DownloadProgress, DownloadStatus};
use crate::error::{DownloadError, Result};
use crate::probe::redact_url_for_log;

/// Detect whether a URL is from a known video streaming platform (YouTube, Facebook, Vimeo, TikTok, etc.)
/// or points to a streaming manifest / segment (.m3u8, .mpd, blob).
pub fn is_streaming_site(url: &str) -> bool {
    let lower = url.to_ascii_lowercase();
    lower.contains("youtube.com/")
        || lower.contains("youtu.be/")
        || lower.contains("facebook.com/")
        || lower.contains("fb.watch/")
        || lower.contains("fb.com/")
        || lower.contains("instagram.com/")
        || lower.contains("tiktok.com/")
        || lower.contains("twitter.com/")
        || lower.contains("x.com/")
        || lower.contains("vimeo.com/")
        || lower.contains("reddit.com/")
        || lower.contains("dailymotion.com/")
        || lower.contains("twitch.tv/")
        || lower.contains("bilibili.com/")
        || lower.contains(".m3u8")
        || lower.contains(".mpd")
        || lower.contains("/manifest")
        || lower.contains("/hls/")
        || lower.contains("/dash/")
}

/// Detect whether a URL is specifically a YouTube video.
pub fn is_youtube_url(url: &str) -> bool {
    let lower = url.to_ascii_lowercase();
    lower.contains("youtube.com/") || lower.contains("youtu.be/")
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
    let binary_name = if cfg!(windows) { "yt-dlp.exe" } else { "yt-dlp" };

    // 1. Next to current executable
    if let Ok(mut exe) = std::env::current_exe() {
        exe.pop();
        let candidate = exe.join(binary_name);
        if candidate.exists() {
            return Some(candidate);
        }
    }

    // 2. In %LOCALAPPDATA%\Programs\Cosmo Downloader\yt-dlp.exe (or legacy)
    if let Some(local_app_data) = std::env::var_os("LOCALAPPDATA") {
        let base = PathBuf::from(local_app_data).join("Programs");
        let cosmo_spaced = base.join("Cosmo Downloader").join(binary_name);
        if cosmo_spaced.exists() {
            return Some(cosmo_spaced);
        }
        let cosmo = base.join("CosmoDownloader").join(binary_name);
        if cosmo.exists() {
            return Some(cosmo);
        }
        let legacy = base.join("RustGet").join(binary_name);
        if legacy.exists() {
            return Some(legacy);
        }
    }

    // 3. Search in PATH directories
    if let Some(path_var) = std::env::var_os("PATH") {
        for dir in std::env::split_paths(&path_var) {
            let candidate = dir.join(binary_name);
            if candidate.exists() {
                return Some(candidate);
            }
            if cfg!(windows) {
                let candidate_bare = dir.join("yt-dlp");
                if candidate_bare.exists() {
                    return Some(candidate_bare);
                }
            }
        }
    }

    // 4. Current working directory and parent directories (workspace roots) - canonicalized
    let candidates = [
        PathBuf::from(binary_name),
        PathBuf::from(format!("../{}", binary_name)),
        PathBuf::from(format!("../../{}", binary_name)),
    ];
    for c in &candidates {
        if c.exists() {
            if let Ok(abs) = std::fs::canonicalize(c) {
                return Some(abs);
            }
            return Some(c.clone());
        }
    }

    None
}

/// Locate ffmpeg binary on the system.
pub fn find_ffmpeg_binary() -> Option<PathBuf> {
    let binary_name = if cfg!(windows) { "ffmpeg.exe" } else { "ffmpeg" };

    // 1. Next to current executable
    if let Ok(mut exe) = std::env::current_exe() {
        exe.pop();
        let candidate = exe.join(binary_name);
        if candidate.exists() {
            return Some(candidate);
        }
    }

    // 2. Next to yt-dlp binary if found
    if let Some(ytdlp) = find_ytdlp_binary() {
        if let Some(parent) = ytdlp.parent() {
            let candidate = parent.join(binary_name);
            if candidate.exists() {
                return Some(candidate);
            }
        }
    }

    // 3. In %LOCALAPPDATA%\Programs\Cosmo Downloader\ffmpeg.exe (or legacy)
    if let Some(local_app_data) = std::env::var_os("LOCALAPPDATA") {
        let base = PathBuf::from(local_app_data).join("Programs");
        for app_folder in ["Cosmo Downloader", "CosmoDownloader", "RustGet"] {
            let candidate = base.join(app_folder).join(binary_name);
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

    // 5. Search in PATH directories
    if let Some(path_var) = std::env::var_os("PATH") {
        for dir in std::env::split_paths(&path_var) {
            let candidate = dir.join(binary_name);
            if candidate.exists() {
                return Some(candidate);
            }
        }
    }

    // 6. Current working directory and parent directories (workspace roots) - canonicalized
    let sharex_x86_candidate = PathBuf::from("C:\\Program Files (x86)\\ShareX\\ffmpeg.exe");
    if sharex_x86_candidate.exists() {
        return Some(sharex_x86_candidate);
    }

    // 5. Search in PATH directories
    if let Some(path_var) = std::env::var_os("PATH") {
        for dir in std::env::split_paths(&path_var) {
            let candidate = dir.join(binary_name);
            if candidate.exists() {
                return Some(candidate);
            }
        }
    }

    // 6. Current working directory and parent directories (workspace roots) - canonicalized
    let candidates = [
        PathBuf::from(binary_name),
        PathBuf::from(format!("../{}", binary_name)),
        PathBuf::from(format!("../../{}", binary_name)),
    ];
    for c in &candidates {
        if c.exists() {
            if let Ok(abs) = std::fs::canonicalize(c) {
                return Some(abs);
            }
            return Some(c.clone());
        }
    }

    None
}

/// Query video title asynchronously using yt-dlp.
pub async fn fetch_video_title(bin: &Path, url: &str) -> Option<String> {
    #[cfg(windows)]
    const CREATE_NO_WINDOW: u32 = 0x08000000;

    let mut cmd = Command::new(bin);
    cmd.kill_on_drop(true);
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

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct VideoQualityOption {
    pub label: String,
    pub format: String,
    pub height: Option<u32>,
    pub format_id: String,
    pub size: Option<u64>,
    pub size_formatted: String,
    pub is_approx: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct CompactVideoFormats {
    pub video_id: String,
    pub title: String,
    pub duration: u64,
    pub is_live: bool,
    pub audio: Option<VideoQualityOption>,
    pub best: Option<VideoQualityOption>,
    pub qualities: Vec<VideoQualityOption>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct StreamingVideoInfo {
    pub title: String,
    pub filename: String,
    pub filesize_approx: Option<u64>,
}

/// Format size according to specification:
/// KB/MB/GB, 1 decimal below 10 (9.4 MB), no decimals from 10 to 999 MB, GB with 2 decimals (1.24 GB).
/// Estimates marked with "~" (e.g. "~150 MB"). If unknown or zero, show "—".
pub fn format_quality_size(bytes: Option<u64>, is_approx: bool) -> String {
    let bytes_val = match bytes {
        Some(b) if b > 0 => b,
        _ => return "—".to_string(),
    };

    const KB: u64 = 1024;
    const MB: u64 = 1024 * KB;
    const GB: u64 = 1024 * MB;

    let prefix = if is_approx { "~" } else { "" };

    if bytes_val >= GB {
        let val = bytes_val as f64 / GB as f64;
        format!("{}{:.2} GB", prefix, val)
    } else if bytes_val >= MB {
        let val = bytes_val as f64 / MB as f64;
        if val < 10.0 {
            format!("{}{:.1} MB", prefix, val)
        } else {
            format!("{}{:.0} MB", prefix, val.round())
        }
    } else if bytes_val >= KB {
        let val = bytes_val as f64 / KB as f64;
        if val < 10.0 {
            format!("{}{:.1} KB", prefix, val)
        } else {
            format!("{}{:.0} KB", prefix, val.round())
        }
    } else {
        format!("{}{} B", prefix, bytes_val)
    }
}

pub fn clean_video_title(raw_title: &str) -> String {
    if raw_title.is_empty() {
        return "YouTube_Video".to_string();
    }
    let mut t = raw_title.trim();
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
    let cleaned = t.trim();
    if cleaned.is_empty() {
        "YouTube_Video".to_string()
    } else {
        cleaned.to_string()
    }
}

fn extract_stream_size(f: &serde_json::Value, duration: f64) -> (Option<u64>, bool) {
    if let Some(fs) = f.get("filesize").and_then(|v| v.as_u64()) {
        if fs > 0 {
            return (Some(fs), false);
        }
    }
    if let Some(fsa) = f.get("filesize_approx").and_then(|v| v.as_u64()) {
        if fsa > 0 {
            return (Some(fsa), true);
        }
    }
    let br = f.get("tbr").and_then(|v| v.as_f64())
        .or_else(|| f.get("vbr").and_then(|v| v.as_f64()))
        .or_else(|| f.get("abr").and_then(|v| v.as_f64()));
    if let Some(br_val) = br {
        if br_val > 0.0 && duration > 0.0 {
            let est = (br_val * 1000.0 * duration / 8.0).round() as u64;
            if est > 0 {
                return (Some(est), true);
            }
        }
    }
    (None, false)
}

/// Parse compact list of video formats from yt-dlp JSON dump.
pub fn parse_compact_video_formats(json: &serde_json::Value, fallback_url: &str) -> CompactVideoFormats {
    let raw_title = json.get("title").and_then(|v| v.as_str()).unwrap_or("").trim();
    let title = clean_video_title(raw_title);

    let duration_secs = json.get("duration").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let duration = duration_secs as u64;

    let is_live = json.get("is_live").and_then(|v| v.as_bool()).unwrap_or(false)
        || json.get("live_status").and_then(|v| v.as_str()).map(|s| s == "is_live" || s == "is_upcoming").unwrap_or(false);

    let video_id = json.get("id").and_then(|v| v.as_str()).map(String::from).unwrap_or_else(|| {
        let key = normalize_url_key(fallback_url);
        if let Some(stripped) = key.strip_prefix("youtube:") {
            stripped.to_string()
        } else {
            key
        }
    });

    let empty_formats = Vec::new();
    let formats = json.get("formats").and_then(|v| v.as_array()).unwrap_or(&empty_formats);

    // 1. Audio stream
    let mut audio_row: Option<VideoQualityOption> = None;
    let mut best_audio_size: Option<u64> = None;
    let mut best_audio_is_est = false;
    let mut best_audio_fid = String::new();

    if !formats.is_empty() {
        let audio_candidates: Vec<&serde_json::Value> = formats
            .iter()
            .filter(|f| {
                let vcodec = f.get("vcodec").and_then(|v| v.as_str()).unwrap_or("none");
                let acodec = f.get("acodec").and_then(|v| v.as_str()).unwrap_or("none");
                let ext = f.get("ext").and_then(|v| v.as_str()).unwrap_or("");
                vcodec == "none" && acodec != "none" && ext != "mhtml"
            })
            .collect();

        let best_a = audio_candidates
            .iter()
            .filter(|f| {
                let proto = f.get("protocol").and_then(|v| v.as_str()).unwrap_or("https");
                proto.starts_with("http")
            })
            .next_back()
            .copied()
            .or_else(|| audio_candidates.last().copied());

        if let Some(best_a) = best_a {
            best_audio_fid = best_a.get("format_id").and_then(|v| v.as_str()).unwrap_or("bestaudio").to_string();
            if !is_live {
                let (sz, is_est) = extract_stream_size(best_a, duration_secs);
                best_audio_size = sz;
                best_audio_is_est = is_est;
            }
            let sz_fmt = if is_live {
                "—".to_string()
            } else {
                format_quality_size(best_audio_size, best_audio_is_est)
            };
            audio_row = Some(VideoQualityOption {
                label: "Audio only".to_string(),
                format: "audio".to_string(),
                height: None,
                format_id: best_audio_fid.clone(),
                size: best_audio_size,
                size_formatted: sz_fmt,
                is_approx: best_audio_is_est,
            });
        }
    }

    // 2. Video resolutions: 2160p, 1440p, 1080p, 720p, 480p, 360p, 240p, 144p
    const STANDARD_RESOLUTIONS: [u32; 8] = [2160, 1440, 1080, 720, 480, 360, 240, 144];
    let mut qualities: Vec<VideoQualityOption> = Vec::new();

    for &res in &STANDARD_RESOLUTIONS {
        let matching: Vec<&serde_json::Value> = formats
            .iter()
            .filter(|f| {
                let h = f.get("height").and_then(|v| v.as_u64()).map(|v| v as u32);
                let vcodec = f.get("vcodec").and_then(|v| v.as_str()).unwrap_or("none");
                let ext = f.get("ext").and_then(|v| v.as_str()).unwrap_or("");
                h == Some(res) && vcodec != "none" && ext != "mhtml"
            })
            .collect();

        if matching.is_empty() {
            continue;
        }

        let best_v = matching
            .iter()
            .filter(|f| {
                let proto = f.get("protocol").and_then(|v| v.as_str()).unwrap_or("https");
                proto.starts_with("http")
            })
            .next_back()
            .copied()
            .or_else(|| matching.last().copied());

        if let Some(best_v) = best_v {
            let v_id = best_v.get("format_id").and_then(|v| v.as_str()).unwrap_or("").to_string();
            let is_combined = best_v.get("acodec").and_then(|v| v.as_str()).map(|a| a != "none").unwrap_or(false);

            let (total_size, is_approx, fid) = if is_live {
                let fid = if is_combined || best_audio_fid.is_empty() {
                    v_id
                } else {
                    format!("{}+{}", v_id, best_audio_fid)
                };
                (None, false, fid)
            } else if is_combined {
                let (v_size, v_est) = extract_stream_size(best_v, duration_secs);
                (v_size, v_est, v_id)
            } else {
                let (v_size, v_est) = extract_stream_size(best_v, duration_secs);
                let combined_size = match (v_size, best_audio_size) {
                    (Some(v), Some(a)) => Some(v + a),
                    (Some(v), None) => Some(v),
                    (None, Some(a)) => Some(a),
                    (None, None) => None,
                };
                let fid = if best_audio_fid.is_empty() {
                    v_id
                } else {
                    format!("{}+{}", v_id, best_audio_fid)
                };
                let combined_est = v_est || best_audio_is_est;
                (combined_size, combined_est, fid)
            };

            let sz_fmt = if is_live {
                "—".to_string()
            } else {
                format_quality_size(total_size, is_approx)
            };

            qualities.push(VideoQualityOption {
                label: format!("{}p", res),
                format: format!("{}p", res),
                height: Some(res),
                format_id: fid,
                size: total_size,
                size_formatted: sz_fmt,
                is_approx,
            });
        }
    }

    // If standard resolutions were not matched (e.g. Facebook videos, vertical reels, custom HLS), collect available video formats
    if qualities.is_empty() && !formats.is_empty() {
        let mut candidates: Vec<(u32, &serde_json::Value)> = formats
            .iter()
            .filter_map(|f| {
                let h = f.get("height").and_then(|v| v.as_u64()).map(|v| v as u32).unwrap_or(0);
                let vcodec = f.get("vcodec").and_then(|v| v.as_str()).unwrap_or("none");
                let ext = f.get("ext").and_then(|v| v.as_str()).unwrap_or("");
                if (h > 0 || vcodec != "none") && ext != "mhtml" {
                    Some((h, f))
                } else {
                    None
                }
            })
            .collect();
        candidates.sort_by(|a, b| b.0.cmp(&a.0));

        let mut seen = std::collections::HashSet::new();
        for (h, f) in candidates {
            let label = if h > 0 {
                format!("{}p", h)
            } else {
                f.get("format_note").and_then(|v| v.as_str()).unwrap_or("HD").to_string()
            };
            if seen.insert(label.clone()) {
                let (sz, is_est) = extract_stream_size(f, duration_secs);
                let fid = f.get("format_id").and_then(|v| v.as_str()).unwrap_or("best").to_string();
                let sz_fmt = format_quality_size(sz, is_est);
                qualities.push(VideoQualityOption {
                    label: label.clone(),
                    format: label,
                    height: if h > 0 { Some(h) } else { None },
                    format_id: fid,
                    size: sz,
                    size_formatted: sz_fmt,
                    is_approx: is_est,
                });
                if qualities.len() >= 6 {
                    break;
                }
            }
        }
    }

    if qualities.is_empty() {
        qualities.push(VideoQualityOption {
            label: "Best Quality".to_string(),
            format: "best".to_string(),
            height: None,
            format_id: "best".to_string(),
            size: None,
            size_formatted: "Auto".to_string(),
            is_approx: false,
        });
    }

    // 3. Best available / Auto
    let best = qualities.first().map(|top| {
        let meta = if is_live {
            format!("Auto · {}", top.label)
        } else if top.size.is_some() {
            format!("Auto · {} · {}", top.label, top.size_formatted)
        } else {
            format!("Auto · {}", top.label)
        };
        VideoQualityOption {
            label: "Best available".to_string(),
            format: "best".to_string(),
            height: top.height,
            format_id: top.format_id.clone(),
            size: top.size,
            size_formatted: meta,
            is_approx: top.is_approx,
        }
    }).or_else(|| {
        Some(VideoQualityOption {
            label: "Best available".to_string(),
            format: "best".to_string(),
            height: None,
            format_id: "best".to_string(),
            size: None,
            size_formatted: "Auto".to_string(),
            is_approx: false,
        })
    });

    CompactVideoFormats {
        video_id,
        title,
        duration,
        is_live,
        audio: audio_row,
        best,
        qualities,
    }
}

#[derive(Clone)]
struct CacheItem {
    info: CompactVideoFormats,
    cached_at: Instant,
}

static FORMATS_CACHE: OnceLock<Mutex<HashMap<String, CacheItem>>> = OnceLock::new();
static IN_FLIGHT: OnceLock<tokio::sync::Mutex<HashMap<String, Arc<tokio::sync::broadcast::Sender<std::result::Result<CompactVideoFormats, String>>>>>> = OnceLock::new();

fn get_formats_cache() -> &'static Mutex<HashMap<String, CacheItem>> {
    FORMATS_CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

fn get_in_flight() -> &'static tokio::sync::Mutex<HashMap<String, Arc<tokio::sync::broadcast::Sender<std::result::Result<CompactVideoFormats, String>>>>> {
    IN_FLIGHT.get_or_init(|| tokio::sync::Mutex::new(HashMap::new()))
}

/// Look up cached video formats by URL/video ID (10-minute cache).
pub fn get_cached_video_formats(url: &str) -> Option<CompactVideoFormats> {
    let key = normalize_url_key(url);
    if key.is_empty() {
        return None;
    }
    if let Ok(mut cache) = get_formats_cache().lock() {
        cache.retain(|_, item| item.cached_at.elapsed() < Duration::from_secs(600));
        if let Some(item) = cache.get(&key) {
            return Some(item.info.clone());
        }
    }
    None
}

/// Query compact video formats using yt-dlp, shared with Download File Info dialog and cached for 10 minutes.
pub async fn query_video_formats(url: &str) -> std::result::Result<CompactVideoFormats, String> {
    query_video_formats_with_headers(url, None, None, None).await
}

/// Query compact video formats with optional session cookies, referer, and user-agent.
pub async fn query_video_formats_with_headers(
    url: &str,
    cookie: Option<&str>,
    referer: Option<&str>,
    user_agent: Option<&str>,
) -> std::result::Result<CompactVideoFormats, String> {
    let key = normalize_url_key(url);
    if key.is_empty() {
        return Err("Empty URL".to_string());
    }

    // 1. Check in-memory 10-minute cache
    if let Some(info) = get_cached_video_formats(url) {
        return Ok(info);
    }

    // 2. In-flight deduplication: shared lookup, not two
    let rx_opt = {
        let mut in_flight = get_in_flight().lock().await;
        if let Some(tx) = in_flight.get(&key) {
            Some(tx.subscribe())
        } else {
            let (tx, _) = tokio::sync::broadcast::channel(1);
            in_flight.insert(key.clone(), Arc::new(tx));
            None
        }
    };

    if let Some(mut rx) = rx_opt {
        return match rx.recv().await {
            Ok(res) => res,
            Err(_) => Err("In-flight video lookup cancelled".to_string()),
        };
    }

    // Leader: execute yt-dlp
    let result = execute_query_video_formats(url, cookie, referer, user_agent).await;

    // Save to cache on success
    if let Ok(ref info) = result {
        if let Ok(mut cache) = get_formats_cache().lock() {
            cache.insert(key.clone(), CacheItem {
                info: info.clone(),
                cached_at: Instant::now(),
            });
        }
    }

    // Notify all waiters
    let tx = {
        let mut in_flight = get_in_flight().lock().await;
        in_flight.remove(&key)
    };
    if let Some(tx) = tx {
        let _ = tx.send(result.clone());
    }

    result
}

async fn execute_query_video_formats(
    url: &str,
    cookie: Option<&str>,
    referer: Option<&str>,
    user_agent: Option<&str>,
) -> std::result::Result<CompactVideoFormats, String> {
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
    cmd.kill_on_drop(true);
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
    ]);

    if is_youtube_url(url) {
        cmd.args(["--extractor-args", "youtube:player_client=android,web,default"]);
    }

    if let Some(c) = cookie {
        if !c.trim().is_empty() {
            cmd.args(["--add-header", &format!("Cookie: {}", c.trim())]);
        }
    }
    if let Some(r) = referer {
        if !r.trim().is_empty() {
            cmd.args(["--referer", r.trim()]);
        }
    }
    if let Some(ua) = user_agent {
        if !ua.trim().is_empty() {
            cmd.args(["--user-agent", ua.trim()]);
        }
    }

    cmd.arg(url);

    #[cfg(windows)]
    cmd.creation_flags(CREATE_NO_WINDOW);

    let start_probe = Instant::now();
    let redacted_url = redact_url_for_log(url);
    crate::log_event(&format!("YTDLP: start probe formats for url='{}'", redacted_url));

    let output_res = tokio::time::timeout(Duration::from_secs(8), cmd.output()).await;
    let _ = tokio::fs::remove_dir_all(&temp_dir).await;

    match output_res {
        Ok(Ok(output)) => {
            crate::log_event(&format!(
                "YTDLP: probe formats exited in {:.2}s for url='{}' exit_code={:?}",
                start_probe.elapsed().as_secs_f64(),
                redacted_url,
                output.status.code()
            ));
            if output.status.success() {
                if let Ok(json) = serde_json::from_slice::<serde_json::Value>(&output.stdout) {
                    let compact = parse_compact_video_formats(&json, url);
                    return Ok(compact);
                }
                Err("Failed to parse yt-dlp JSON".to_string())
            } else {
                let stderr = String::from_utf8_lossy(&output.stderr);
                let err_msg = if !stderr.trim().is_empty() {
                    stderr.trim().to_string()
                } else {
                    "Failed to extract video info".to_string()
                };

                // Fallback for media streams or streaming URLs (e.g. HLS, DASH, direct streams)
                if is_streaming_site(url) || url.contains(".m3u8") || url.contains(".mpd") {
                    let fallback_title = if is_youtube_url(url) {
                        "YouTube Video"
                    } else if url.contains("facebook.com") || url.contains("fb.watch") {
                        "Facebook Video"
                    } else {
                        "Web Video Stream"
                    };
                    return Ok(CompactVideoFormats {
                        video_id: normalize_url_key(url),
                        title: fallback_title.to_string(),
                        duration: 0,
                        is_live: false,
                        qualities: vec![
                            VideoQualityOption {
                                label: "Best Available (Auto)".to_string(),
                                format: "best".to_string(),
                                height: None,
                                format_id: "best".to_string(),
                                size: None,
                                size_formatted: "Auto".to_string(),
                                is_approx: false,
                            }
                        ],
                        audio: Some(VideoQualityOption {
                            label: "Audio only".to_string(),
                            format: "audio".to_string(),
                            height: None,
                            format_id: "bestaudio".to_string(),
                            size: None,
                            size_formatted: "Auto".to_string(),
                            is_approx: false,
                        }),
                        best: Some(VideoQualityOption {
                            label: "Best Available (MP4)".to_string(),
                            format: "best".to_string(),
                            height: None,
                            format_id: "best".to_string(),
                            size: None,
                            size_formatted: "Auto".to_string(),
                            is_approx: false,
                        }),
                    });
                }

                Err(err_msg)
            }
        }
        Ok(Err(e)) => Err(format!("Process error: {}", e)),
        Err(_) => {
            // If probe timed out but URL is a known streaming / media site, return graceful stream option
            if is_streaming_site(url) || url.contains(".m3u8") || url.contains(".mpd") {
                return Ok(CompactVideoFormats {
                    video_id: normalize_url_key(url),
                    title: "Web Video Stream".to_string(),
                    duration: 0,
                    is_live: false,
                    qualities: vec![
                        VideoQualityOption {
                            label: "Best Available (Auto)".to_string(),
                            format: "best".to_string(),
                            height: None,
                            format_id: "best".to_string(),
                            size: None,
                            size_formatted: "Auto".to_string(),
                            is_approx: false,
                        }
                    ],
                    audio: Some(VideoQualityOption {
                        label: "Audio only".to_string(),
                        format: "audio".to_string(),
                        height: None,
                        format_id: "bestaudio".to_string(),
                        size: None,
                        size_formatted: "Auto".to_string(),
                        is_approx: false,
                    }),
                    best: Some(VideoQualityOption {
                        label: "Best Available (MP4)".to_string(),
                        format: "best".to_string(),
                        height: None,
                        format_id: "best".to_string(),
                        size: None,
                        size_formatted: "Auto".to_string(),
                        is_approx: false,
                    }),
                });
            }
            Err("Video info lookup timed out after 8s".to_string())
        }
    }
}

/// Query video title and approximate size asynchronously using shared yt-dlp lookup.
pub async fn query_streaming_video_info(url: &str) -> std::result::Result<StreamingVideoInfo, String> {
    let formats = query_video_formats(url).await?;
    let sanitized = crate::probe::sanitize_filename(&formats.title);
    let filename = if sanitized.contains('.') {
        sanitized
    } else {
        format!("{}.mp4", sanitized)
    };
    let filesize_approx = formats.best.as_ref().and_then(|b| b.size);
    Ok(StreamingVideoInfo {
        title: formats.title,
        filename,
        filesize_approx,
    })
}

/// Download a streaming video (YouTube, Facebook, Vimeo, HLS, etc.) with progress tracking and cancellation.
pub async fn download_streaming_video(
    bin: &Path,
    url: &str,
    output_path: &Path,
    cancel: CancellationToken,
    progress_tx: mpsc::Sender<DownloadProgress>,
    format_selector: Option<&str>,
    cookie: Option<&str>,
    referer: Option<&str>,
    user_agent: Option<&str>,
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
        let default_name = if is_audio { "Video_Audio.mp3" } else { "Video_Download.mp4" };
        final_path = parent.join(default_name);
    } else if !fname.contains('.') {
        final_path.set_extension(if is_audio { "mp3" } else { "mp4" });
    }
    let out_template = final_path.to_string_lossy().to_string();

    let mut args = vec![
        "-N".to_string(), "8".to_string(),
        "--newline".to_string(),
        "--progress-template".to_string(), "%(progress.downloaded_bytes)s/%(progress.total_bytes)s/%(progress.speed)s".to_string(),
        "--progress-template".to_string(), "postprocess:%(progress.status)s".to_string(),
    ];

    if is_youtube_url(url) {
        args.push("--extractor-args".to_string());
        args.push("youtube:player_client=android,web,default".to_string());
    }

    if let Some(c) = cookie {
        if !c.trim().is_empty() {
            args.push("--add-header".to_string());
            args.push(format!("Cookie: {}", c.trim()));
        }
    }
    if let Some(r) = referer {
        if !r.trim().is_empty() {
            args.push("--referer".to_string());
            args.push(r.trim().to_string());
        }
    }
    if let Some(ua) = user_agent {
        if !ua.trim().is_empty() {
            args.push("--user-agent".to_string());
            args.push(ua.trim().to_string());
        }
    }

    let ffmpeg_bin = find_ffmpeg_binary();
    if let Some(ref ffmpeg_path) = ffmpeg_bin {
        args.push("--ffmpeg-location".to_string());
        args.push(ffmpeg_path.to_string_lossy().to_string());
    }

    if is_audio {
        if let Some(fs) = format_selector {
            if fs != "audio" && fs != "best" {
                args.push("-f".to_string());
                args.push(format!("{fs}/bestaudio/best"));
            } else if ffmpeg_bin.is_some() {
                args.push("-x".to_string());
                args.push("--audio-format".to_string());
                args.push("mp3".to_string());
            } else {
                args.push("-f".to_string());
                args.push("bestaudio/best".to_string());
            }
        } else if ffmpeg_bin.is_some() {
            args.push("-x".to_string());
            args.push("--audio-format".to_string());
            args.push("mp3".to_string());
        } else {
            args.push("-f".to_string());
            args.push("bestaudio/best".to_string());
        }
    } else {
        if let Some(fs) = format_selector {
            if fs.contains('+') || fs.chars().all(|c| c.is_ascii_digit()) {
                args.push("-f".to_string());
                args.push(format!("{fs}/bestvideo+bestaudio/best"));
                if ffmpeg_bin.is_some() {
                    args.push("--merge-output-format".to_string());
                    args.push("mp4".to_string());
                }
            } else if fs.ends_with('p') && fs[..fs.len() - 1].chars().all(|c| c.is_ascii_digit()) {
                let h = &fs[..fs.len() - 1];
                if ffmpeg_bin.is_some() {
                    args.push("-f".to_string());
                    args.push(format!("bestvideo[height<={}][ext=mp4]+bestaudio[ext=m4a]/bestvideo[height<={}]+bestaudio/best[height<={}]/best", h, h, h));
                    args.push("--merge-output-format".to_string());
                    args.push("mp4".to_string());
                } else {
                    args.push("-f".to_string());
                    args.push(format!("best[height<={}][ext=mp4]/best[height<={}]/best", h, h));
                }
            } else if fs == "best" {
                if ffmpeg_bin.is_some() {
                    args.push("-f".to_string());
                    args.push("bestvideo[ext=mp4]+bestaudio[ext=m4a]/bestvideo+bestaudio/best[ext=mp4]/best".to_string());
                    args.push("--merge-output-format".to_string());
                    args.push("mp4".to_string());
                } else {
                    args.push("-f".to_string());
                    args.push("best[ext=mp4]/best".to_string());
                }
            } else {
                args.push("-f".to_string());
                args.push(format!("{fs}/best"));
                if ffmpeg_bin.is_some() {
                    args.push("--merge-output-format".to_string());
                    args.push("mp4".to_string());
                }
            }
        } else if ffmpeg_bin.is_some() {
            args.push("-f".to_string());
            args.push("bestvideo[ext=mp4]+bestaudio[ext=m4a]/bestvideo+bestaudio/best[ext=mp4]/best".to_string());
            args.push("--merge-output-format".to_string());
            args.push("mp4".to_string());
        } else {
            args.push("-f".to_string());
            args.push("best[ext=mp4]/best".to_string());
        }
    }

    args.push("-o".to_string());
    args.push(out_template);
    args.push(url.to_string());

    let mut cmd = Command::new(bin);
    cmd.kill_on_drop(true);
    cmd.env("PYTHONIOENCODING", "utf-8");
    cmd.env("PYTHONUTF8", "1");
    cmd.args(&args);

    cmd.stdout(std::process::Stdio::piped());
    cmd.stderr(std::process::Stdio::piped());

    #[cfg(windows)]
    cmd.creation_flags(CREATE_NO_WINDOW);

    let ytdlp_start = Instant::now();
    let redacted_url = redact_url_for_log(url);
    crate::log_event(&format!(
        "YTDLP: start download binary='{}' url='{}' args='{}'",
        bin.display(),
        redacted_url,
        args.join(" ")
    ));

    let mut child = cmd.spawn().map_err(|e| {
        crate::log_event(&format!("YTDLP: failed to spawn yt-dlp: {}", e));
        DownloadError::Io(std::io::Error::other(format!("Failed to spawn yt-dlp: {}", e)))
    })?;

    let stdout = child.stdout.take().ok_or_else(|| {
        DownloadError::Io(std::io::Error::other("Failed to capture yt-dlp stdout"))
    })?;

    let stderr = child.stderr.take();
    let last_stderr_lines = std::sync::Arc::new(std::sync::Mutex::new(std::collections::VecDeque::<String>::with_capacity(30)));
    let stderr_holder = std::sync::Arc::clone(&last_stderr_lines);
    if let Some(err_stream) = stderr {
        tokio::spawn(async move {
            let mut lines = BufReader::new(err_stream).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                let trimmed = line.trim();
                if !trimmed.is_empty() {
                    tracing::debug!("[yt-dlp stderr] {}", trimmed);
                    if let Ok(mut holder) = stderr_holder.lock() {
                        if holder.len() >= 30 {
                            holder.pop_front();
                        }
                        holder.push_back(trimmed.to_string());
                    }
                }
            }
        });
    }

    let mut reader = BufReader::new(stdout).lines();
    let mut last_progress_send = Instant::now();
    let mut prev_completed_bytes = 0u64;
    let mut current_stream_last_dl = 0u64;
    let mut current_stream_total: Option<u64> = None;
    let mut max_emitted_downloaded = 0u64;
    let mut speed = 0.0f64;
    let mut is_processing = false;

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

                        if trimmed.starts_with("postprocess:") || trimmed.contains("[Merger]") || trimmed.contains("Merging formats") {
                            is_processing = true;
                            let combined_tot = match current_stream_total {
                                Some(curr_tot) => Some(prev_completed_bytes + curr_tot),
                                None => None,
                            };
                            let _ = progress_tx.send(DownloadProgress {
                                status: DownloadStatus::Processing,
                                downloaded_bytes: combined_tot.unwrap_or(max_emitted_downloaded),
                                total_bytes: combined_tot,
                                speed_bps: 0.0,
                                eta_seconds: None,
                                segments: Vec::new(),
                                file_path: output_path.to_path_buf(),
                            }).await;
                            continue;
                        }

                        // Format: <downloaded_bytes>/<total_bytes>/<speed_bps>
                        if let Some((dl_str, rest)) = trimmed.split_once('/') {
                            if let Some((tot_str, spd_str)) = rest.split_once('/') {
                                let mut dl = 0u64;
                                let mut tot = 0u64;
                                if let Ok(parsed_dl) = dl_str.parse::<u64>() {
                                    dl = parsed_dl;
                                }
                                if let Ok(parsed_tot) = tot_str.parse::<u64>() {
                                    tot = parsed_tot;
                                }
                                if let Ok(spd) = spd_str.parse::<f64>() {
                                    speed = spd;
                                }

                                // Detect switch to second stream (e.g. video finished, audio starts)
                                if dl < current_stream_last_dl && current_stream_last_dl > 100_000 {
                                    let stream_finish = current_stream_total.unwrap_or(current_stream_last_dl);
                                    prev_completed_bytes += stream_finish;
                                    current_stream_last_dl = dl;
                                    current_stream_total = if tot > 0 { Some(tot) } else { None };
                                } else {
                                    current_stream_last_dl = dl;
                                    if tot > 0 {
                                        current_stream_total = Some(tot);
                                    }
                                }

                                let combined_dl = (prev_completed_bytes + current_stream_last_dl).max(max_emitted_downloaded);
                                max_emitted_downloaded = combined_dl;

                                let combined_tot = match current_stream_total {
                                    Some(curr_tot) => Some(prev_completed_bytes + curr_tot),
                                    None => None,
                                };

                                if last_progress_send.elapsed() >= std::time::Duration::from_millis(50) {
                                    let eta_seconds = match combined_tot {
                                        Some(tot_val) if speed > 0.0 => {
                                            let remaining = tot_val.saturating_sub(combined_dl);
                                            Some((remaining as f64 / speed) as u64)
                                        }
                                        _ => None,
                                    };

                                    let _ = progress_tx.send(DownloadProgress {
                                        status: if is_processing { DownloadStatus::Processing } else { DownloadStatus::Downloading },
                                        downloaded_bytes: combined_dl,
                                        total_bytes: combined_tot,
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
    let ytdlp_duration = ytdlp_start.elapsed();
    crate::log_event(&format!(
        "YTDLP: download exited in {:.2}s for url='{}' exit_code={:?}",
        ytdlp_duration.as_secs_f64(),
        redacted_url,
        status.code()
    ));

    if !status.success() {
        let err_summary = {
            if let Ok(holder) = last_stderr_lines.lock() {
                let relevant: Vec<String> = holder
                    .iter()
                    .filter(|l| {
                        l.contains("ERROR")
                            || l.contains("Error")
                            || l.contains("Forbidden")
                            || l.contains("HTTP")
                            || l.contains("failed")
                    })
                    .cloned()
                    .collect();
                if !relevant.is_empty() {
                    relevant.join(" | ")
                } else if !holder.is_empty() {
                    holder.iter().cloned().collect::<Vec<_>>().join(" | ")
                } else {
                    format!("yt-dlp exited with status code: {:?}", status.code())
                }
            } else {
                format!("yt-dlp exited with status code: {:?}", status.code())
            }
        };
        crate::log_event(&format!(
            "YTDLP: download error summary after {:.2}s for url='{}': {}",
            ytdlp_duration.as_secs_f64(),
            redacted_url,
            err_summary
        ));
        return Err(DownloadError::Io(std::io::Error::other(err_summary)));
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

    let actual_size = std::fs::metadata(&actual_saved_path).map(|m| m.len()).unwrap_or(0);
    if !actual_saved_path.exists() || actual_size == 0 {
        return Err(DownloadError::Io(std::io::Error::other(
            format!("Streaming video file '{:?}' does not exist on disk or has size 0", actual_saved_path),
        )));
    }

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
        assert!(is_streaming_site("https://www.facebook.com/reel/123456789"));
        assert!(is_streaming_site("https://fb.watch/abcd1234/"));
        assert!(is_streaming_site("https://example.com/live/playlist.m3u8"));
        assert!(is_streaming_site("https://stream.local.net/video/dash.mpd"));
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

    #[test]
    fn test_format_quality_size_rules() {
        // Unknown or 0 -> "—"
        assert_eq!(format_quality_size(None, false), "—");
        assert_eq!(format_quality_size(Some(0), false), "—");
        assert_eq!(format_quality_size(None, true), "—");

        // Below 10 MB: 1 decimal, e.g. 7.5 MB
        let bytes_7_5_mb = (7.5f64 * 1024.0 * 1024.0).round() as u64;
        assert_eq!(format_quality_size(Some(bytes_7_5_mb), false), "7.5 MB");
        assert_eq!(format_quality_size(Some(bytes_7_5_mb), true), "~7.5 MB");

        // 10 to 999 MB: no decimals, e.g. 150 MB, 85 MB, 55 MB, 25 MB
        let bytes_150_mb = 150 * 1024 * 1024;
        assert_eq!(format_quality_size(Some(bytes_150_mb), false), "150 MB");
        assert_eq!(format_quality_size(Some(bytes_150_mb), true), "~150 MB");

        let bytes_85_mb = 85 * 1024 * 1024;
        assert_eq!(format_quality_size(Some(bytes_85_mb), false), "85 MB");

        let bytes_55_mb = 55 * 1024 * 1024;
        assert_eq!(format_quality_size(Some(bytes_55_mb), false), "55 MB");

        let bytes_25_mb = 25 * 1024 * 1024;
        assert_eq!(format_quality_size(Some(bytes_25_mb), false), "25 MB");

        // GB: 2 decimals, e.g. 1.24 GB
        let bytes_1_24_gb = (1.24f64 * 1024.0 * 1024.0 * 1024.0).round() as u64;
        assert_eq!(format_quality_size(Some(bytes_1_24_gb), false), "1.24 GB");
        assert_eq!(format_quality_size(Some(bytes_1_24_gb), true), "~1.24 GB");
    }

    #[test]
    fn test_parse_compact_formats_max_720p() {
        let json = serde_json::json!({
            "id": "test_720p",
            "title": "Short 720p Video - YouTube",
            "duration": 30,
            "is_live": false,
            "formats": [
                {
                    "format_id": "140",
                    "ext": "m4a",
                    "vcodec": "none",
                    "acodec": "mp4a.40.2",
                    "abr": 128.0,
                    "filesize": 480_000
                },
                {
                    "format_id": "136",
                    "ext": "mp4",
                    "height": 720,
                    "vcodec": "avc1.4D401F",
                    "acodec": "none",
                    "tbr": 1200.0,
                    "filesize": 4_500_000
                },
                {
                    "format_id": "134",
                    "ext": "mp4",
                    "height": 360,
                    "vcodec": "avc1.4D401E",
                    "acodec": "none",
                    "tbr": 400.0,
                    "filesize": 1_500_000
                }
            ]
        });

        let compact = parse_compact_video_formats(&json, "https://youtube.com/watch?v=test_720p");
        assert_eq!(compact.title, "Short 720p Video");
        assert_eq!(compact.is_live, false);

        // Max resolution is 720p -> must NOT have 1080p, 1440p, or 2160p row
        let labels: Vec<&str> = compact.qualities.iter().map(|q| q.label.as_str()).collect();
        assert_eq!(labels, vec!["720p", "360p"]);
        assert!(!labels.contains(&"1080p"), "Video topping out at 720p must not have 1080p row");

        // Row sizes: 720p is 4_500_000 + 480_000 = 4_980_000 = 4.7 MB (under 10MB -> 1 dec)
        let row_720 = &compact.qualities[0];
        assert_eq!(row_720.format_id, "136+140");
        assert_eq!(row_720.size, Some(4_980_000));
        assert_eq!(row_720.size_formatted, "4.7 MB");

        // Best available row
        assert!(compact.best.is_some());
        let best = compact.best.unwrap();
        assert_eq!(best.label, "Best available");
        assert_eq!(best.height, Some(720));
        assert_eq!(best.size_formatted, "Auto · 720p · 4.7 MB");

        // Audio row
        assert!(compact.audio.is_some());
        let audio = compact.audio.unwrap();
        assert_eq!(audio.label, "Audio only");
        assert_eq!(audio.size, Some(480_000));
    }

    #[test]
    fn test_parse_compact_formats_live_stream() {
        let json = serde_json::json!({
            "id": "live_video",
            "title": "Breaking News Live - YouTube",
            "duration": null,
            "is_live": true,
            "live_status": "is_live",
            "formats": [
                {
                    "format_id": "95",
                    "ext": "mp4",
                    "height": 1080,
                    "vcodec": "avc1.640028",
                    "acodec": "mp4a.40.2"
                },
                {
                    "format_id": "94",
                    "ext": "mp4",
                    "height": 720,
                    "vcodec": "avc1.4D401F",
                    "acodec": "mp4a.40.2"
                }
            ]
        });

        let compact = parse_compact_video_formats(&json, "https://youtube.com/watch?v=live_video");
        assert_eq!(compact.is_live, true);

        // Live stream: no sizes!
        for q in &compact.qualities {
            assert!(q.size.is_none(), "Live stream must have no size number");
            assert_eq!(q.size_formatted, "—", "Live stream size must be —");
        }
        let best = compact.best.unwrap();
        assert_eq!(best.size_formatted, "Auto · 1080p", "Best available on live stream must show no size number");
    }
}
