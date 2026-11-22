use std::path::{Path, PathBuf};
use percent_encoding::percent_decode_str;
use reqwest::header::{HeaderMap, ACCEPT_RANGES, CONTENT_DISPOSITION, CONTENT_LENGTH, CONTENT_RANGE, CONTENT_TYPE, ETAG, LAST_MODIFIED, RANGE};
use reqwest::Client;
use tracing::{debug, warn};
use url::Url;

use crate::error::{DownloadError, Result};

/// Validates that a URL string is a well-formed HTTP or HTTPS URL with a host.
pub fn validate_http_url(raw: &str) -> Result<Url> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(DownloadError::InvalidUrl("URL cannot be empty".to_string()));
    }
    let parsed = Url::parse(trimmed)
        .map_err(|e| DownloadError::InvalidUrl(format!("Invalid URL '{}': {}", trimmed, e)))?;
    match parsed.scheme() {
        "http" | "https" => {}
        other => {
            return Err(DownloadError::InvalidUrl(format!(
                "Unsupported URL scheme '{}': only http:// and https:// are allowed",
                other
            )));
        }
    }
    if parsed.host_str().map(|h| h.is_empty()).unwrap_or(true) {
        return Err(DownloadError::InvalidUrl("URL must contain a valid host".to_string()));
    }
    Ok(parsed)
}

/// Redacts credentials and query string tokens before writing a URL to logs.
pub fn redact_url_for_log(raw: &str) -> String {
    match Url::parse(raw) {
        Ok(mut u) => {
            if !u.username().is_empty() || u.password().is_some() {
                let _ = u.set_username("***");
                let _ = u.set_password(Some("***"));
            }
            if u.query().is_some() {
                u.set_query(Some("***"));
            }
            u.to_string()
        }
        Err(_) => "<invalid-url>".to_string(),
    }
}

/// Information obtained from probing a remote URL.
#[derive(Debug, Clone)]
pub struct FileInfo {
    /// The canonical URL after redirects.
    pub url: Url,
    /// Total size in bytes if reported by the server.
    pub content_length: Option<u64>,
    /// Whether the server supports HTTP Range requests.
    pub accepts_ranges: bool,
    /// Server ETag if provided.
    pub etag: Option<String>,
    /// Server Last-Modified timestamp string if provided.
    pub last_modified: Option<String>,
    /// Best filename determined from Content-Disposition, URL path, or default.
    pub suggested_filename: String,
    /// MIME type if reported.
    pub mime_type: Option<String>,
}

/// Probes a remote URL using HEAD (with fallback to Range GET) to determine file characteristics.
pub async fn probe_url(client: &Client, url: &Url, custom_headers: &HeaderMap) -> Result<FileInfo> {
    let start = std::time::Instant::now();
    let redacted_url = redact_url_for_log(url.as_str());
    crate::log_event(&format!("PROBE: start url='{}'", redacted_url));
    debug!("Probing URL via HEAD: {}", redacted_url);

    let head_result = client
        .head(url.clone())
        .headers(custom_headers.clone())
        .send()
        .await;

    let res = match head_result {
        Ok(resp) if resp.status().is_success() => {
            let final_url = resp.url().clone();
            let headers = resp.headers();

            let accepts_ranges = headers
                .get(ACCEPT_RANGES)
                .and_then(|v| v.to_str().ok())
                .map(|v| v.eq_ignore_ascii_case("bytes"))
                .unwrap_or(false);

            let content_length = headers
                .get(CONTENT_LENGTH)
                .and_then(|v| v.to_str().ok())
                .and_then(|v| v.parse::<u64>().ok());

            let etag = headers
                .get(ETAG)
                .and_then(|v| v.to_str().ok())
                .map(|s| s.trim().to_string());

            let last_modified = headers
                .get(LAST_MODIFIED)
                .and_then(|v| v.to_str().ok())
                .map(|s| s.trim().to_string());

            let mime_type = headers
                .get(CONTENT_TYPE)
                .and_then(|v| v.to_str().ok())
                .map(|s| s.split(';').next().unwrap_or(s).trim().to_string());

            let suggested_filename = extract_filename(headers, &final_url);

            Ok(FileInfo {
                url: final_url,
                content_length,
                accepts_ranges,
                etag,
                last_modified,
                suggested_filename,
                mime_type,
            })
        }
        Ok(resp) if resp.status().as_u16() == 405 || resp.status().as_u16() == 403 || resp.status().as_u16() == 501 => {
            debug!("HEAD returned {}, falling back to Range GET 0-0", resp.status());
            probe_via_range_get(client, url, custom_headers).await
        }
        Ok(resp) => {
            warn!("HEAD request failed with status: {}", resp.status());
            // Try Range GET as fallback before giving up
            probe_via_range_get(client, url, custom_headers).await
        }
        Err(e) => {
            debug!("HEAD request error ({:?}), falling back to Range GET", e);
            probe_via_range_get(client, url, custom_headers).await
        }
    };

    match &res {
        Ok(info) => crate::log_event(&format!(
            "PROBE: completed in {:.2}ms for url='{}': filename='{}' size={:?}",
            start.elapsed().as_secs_f64() * 1000.0,
            redacted_url,
            info.suggested_filename,
            info.content_length
        )),
        Err(e) => crate::log_event(&format!(
            "PROBE: failed in {:.2}ms for url='{}': error={}",
            start.elapsed().as_secs_f64() * 1000.0,
            redacted_url,
            e
        )),
    }

    res
}

/// Fallback probe using GET with Range: bytes=0-0 to test both range support and content length.
async fn probe_via_range_get(client: &Client, url: &Url, custom_headers: &HeaderMap) -> Result<FileInfo> {
    let mut headers = custom_headers.clone();
    headers.insert(RANGE, "bytes=0-0".parse().map_err(|_| DownloadError::InvalidUrl(url.to_string()))?);

    let resp = client
        .get(url.clone())
        .headers(headers)
        .send()
        .await?;

    let final_url = resp.url().clone();
    let status = resp.status();
    let resp_headers = resp.headers();

    if !status.is_success() && status.as_u16() != 206 {
        return Err(DownloadError::ServerFailed {
            status: status.as_u16(),
            message: format!("HTTP request failed with status {}", status),
        });
    }

    let is_partial = status.as_u16() == 206;
    let mut accepts_ranges = is_partial;
    let mut total_length: Option<u64> = None;

    if let Some(range_val) = resp_headers.get(CONTENT_RANGE).and_then(|v| v.to_str().ok()) {
        // Format: bytes 0-0/12345 or bytes */12345
        accepts_ranges = true;
        if let Some(slash_idx) = range_val.rfind('/') {
            let total_str = &range_val[slash_idx + 1..].trim();
            if *total_str != "*" {
                total_length = total_str.parse::<u64>().ok();
            }
        }
    }

    if total_length.is_none() {
        // Fall back to Content-Length
        if !is_partial {
            total_length = resp_headers
                .get(CONTENT_LENGTH)
                .and_then(|v| v.to_str().ok())
                .and_then(|v| v.parse::<u64>().ok());
        }
    }

    if !accepts_ranges {
        accepts_ranges = resp_headers
            .get(ACCEPT_RANGES)
            .and_then(|v| v.to_str().ok())
            .map(|v| v.eq_ignore_ascii_case("bytes"))
            .unwrap_or(false);
    }

    let etag = resp_headers
        .get(ETAG)
        .and_then(|v| v.to_str().ok())
        .map(|s| s.trim().to_string());

    let last_modified = resp_headers
        .get(LAST_MODIFIED)
        .and_then(|v| v.to_str().ok())
        .map(|s| s.trim().to_string());

    let mime_type = resp_headers
        .get(CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .map(|s| s.split(';').next().unwrap_or(s).trim().to_string());

    let suggested_filename = extract_filename(resp_headers, &final_url);

    Ok(FileInfo {
        url: final_url,
        content_length: total_length,
        accepts_ranges,
        etag,
        last_modified,
        suggested_filename,
        mime_type,
    })
}

/// Extracts a clean, sanitized filename from Content-Disposition or the URL path.
/// Uses the MIME type from the Content-Type header to infer the correct extension
/// when the filename extracted from the URL has no extension.
pub fn extract_filename(headers: &HeaderMap, url: &Url) -> String {
    // Extract MIME type early so we can use it as a fallback for extension detection.
    let mime_type = headers
        .get(CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .map(|s| s.split(';').next().unwrap_or(s).trim().to_lowercase());

    // 0. Detect YouTube and streaming platform URLs directly
    let url_str = url.as_str().to_ascii_lowercase();
    if url_str.contains("youtube.com/") || url_str.contains("youtu.be/") || url_str.contains("vimeo.com/") {
        if let Some(v_id) = url.query_pairs().find(|(k, _)| k == "v").map(|(_, v)| v.to_string()) {
            if !v_id.is_empty() {
                return format!("YouTube_{}.mp4", sanitize_filename(&v_id));
            }
        }
        return "YouTube_Video.mp4".to_string();
    }

    // 1. Try Content-Disposition
    if let Some(disp) = headers.get(CONTENT_DISPOSITION).and_then(|v| v.to_str().ok()) {
        if let Some(name) = parse_content_disposition(disp) {
            let sanitized = sanitize_filename(&name);
            if !sanitized.is_empty() {
                return sanitized;
            }
        }
    }

    // 2. Try URL path
    if let Some(segments) = url.path_segments() {
        for seg in segments.rev() {
            let decoded = percent_decode_str(seg).decode_utf8_lossy().to_string();
            let sanitized = sanitize_filename(&decoded);
            if !sanitized.is_empty() && sanitized != "." && sanitized != ".." {
                // Skip generic URL path segments that aren't real filenames
                let lower = sanitized.to_ascii_lowercase();
                if lower == "watch" || lower == "video" || lower == "view"
                    || lower == "download" || lower == "export" || lower == "uc"
                    || lower == "file" || lower == "d" || lower == "open"
                {
                    continue;
                }

                if sanitized.contains('.') {
                    return sanitized;
                }

                // No extension in URL segment — infer from MIME type
                let ext = mime_to_extension(mime_type.as_deref());
                return format!("{}.{}", sanitized, ext);
            }
        }
    }

    // 3. Fallback: use MIME-based extension for the default filename
    let ext = mime_to_extension(mime_type.as_deref());
    format!("download.{}", ext)
}

/// Maps a MIME type string to a file extension.
pub fn mime_to_extension(mime: Option<&str>) -> &'static str {
    match mime {
        // Images
        Some("image/png") => "png",
        Some("image/jpeg") | Some("image/jpg") => "jpg",
        Some("image/gif") => "gif",
        Some("image/webp") => "webp",
        Some("image/svg+xml") => "svg",
        Some("image/bmp") => "bmp",
        Some("image/tiff") => "tiff",
        Some("image/x-icon") | Some("image/vnd.microsoft.icon") => "ico",
        Some("image/avif") => "avif",
        // Video
        Some("video/mp4") => "mp4",
        Some("video/webm") => "webm",
        Some("video/x-matroska") => "mkv",
        Some("video/x-msvideo") => "avi",
        Some("video/quicktime") => "mov",
        Some("video/x-flv") => "flv",
        Some("video/mp2t") | Some("video/mpeg") => "ts",
        // Audio
        Some("audio/mpeg") => "mp3",
        Some("audio/ogg") => "ogg",
        Some("audio/wav") | Some("audio/x-wav") => "wav",
        Some("audio/flac") | Some("audio/x-flac") => "flac",
        Some("audio/aac") => "aac",
        Some("audio/mp4") => "m4a",
        Some("audio/webm") => "weba",
        // Documents
        Some("application/pdf") => "pdf",
        Some("application/zip") => "zip",
        Some("application/x-7z-compressed") => "7z",
        Some("application/x-rar-compressed") | Some("application/vnd.rar") => "rar",
        Some("application/gzip") | Some("application/x-gzip") => "gz",
        Some("application/x-tar") => "tar",
        Some("application/x-bzip2") => "bz2",
        Some("application/x-xz") => "xz",
        Some("application/json") => "json",
        Some("application/xml") | Some("text/xml") => "xml",
        Some("text/plain") => "txt",
        Some("text/html") => "html",
        Some("text/css") => "css",
        Some("text/javascript") | Some("application/javascript") => "js",
        Some("application/x-iso9660-image") => "iso",
        Some("application/vnd.microsoft.portable-executable") | Some("application/x-msdownload") => "exe",
        Some("application/vnd.debian.binary-package") | Some("application/x-deb") => "deb",
        Some("application/x-rpm") => "rpm",
        Some("application/x-apple-diskimage") => "dmg",
        Some("application/vnd.android.package-archive") => "apk",
        // MS Office
        Some("application/vnd.openxmlformats-officedocument.wordprocessingml.document") => "docx",
        Some("application/vnd.openxmlformats-officedocument.spreadsheetml.sheet") => "xlsx",
        Some("application/vnd.openxmlformats-officedocument.presentationml.presentation") => "pptx",
        Some("application/msword") => "doc",
        Some("application/vnd.ms-excel") => "xls",
        // Fallback
        Some(s) if s.starts_with("image/") => "bin",
        Some(s) if s.starts_with("video/") => "mp4",
        Some(s) if s.starts_with("audio/") => "mp3",
        _ => "bin",
    }
}

/// Parses RFC 6266 / RFC 5987 Content-Disposition header (case-insensitive parameter names).
fn parse_content_disposition(header: &str) -> Option<String> {
    // Look for filename*=UTF-8''encoded_name
    for part in header.split(';') {
        let trimmed = part.trim();
        let lower = trimmed.to_ascii_lowercase();
        if lower.starts_with("filename*=") {
            let val = trimmed["filename*=".len()..].trim().trim_matches('"');
            let lower_val = val.to_ascii_lowercase();
            if lower_val.starts_with("utf-8''") {
                let encoded = &val["utf-8''".len()..];
                if let Ok(decoded) = percent_decode_str(encoded).decode_utf8() {
                    let leaf = decoded.rsplit(['/', '\\']).next().unwrap_or(&decoded);
                    if !leaf.trim().is_empty() {
                        return Some(leaf.to_string());
                    }
                }
            }
        }
    }

    // Look for filename="name" or filename=name
    for part in header.split(';') {
        let trimmed = part.trim();
        let lower = trimmed.to_ascii_lowercase();
        if lower.starts_with("filename=") {
            let val = trimmed["filename=".len()..].trim().trim_matches('"').trim();
            let leaf = val.rsplit(['/', '\\']).next().unwrap_or(val).trim();
            if !leaf.is_empty() {
                return Some(leaf.to_string());
            }
        }
    }

    None
}

/// Returns true if `stem` (before any extension) matches a Windows reserved DOS device name.
fn is_windows_reserved_name(stem: &str) -> bool {
    let upper = stem.trim().to_ascii_uppercase();
    matches!(
        upper.as_str(),
        "CON"
            | "PRN"
            | "AUX"
            | "NUL"
            | "COM0"
            | "COM1"
            | "COM2"
            | "COM3"
            | "COM4"
            | "COM5"
            | "COM6"
            | "COM7"
            | "COM8"
            | "COM9"
            | "LPT0"
            | "LPT1"
            | "LPT2"
            | "LPT3"
            | "LPT4"
            | "LPT5"
            | "LPT6"
            | "LPT7"
            | "LPT8"
            | "LPT9"
    )
}

/// Replaces illegal Windows characters (< > : " / \ | ? *) with look-alikes (＜ ＞ ： ＂ ⧸ ⧹ ｜ ？ ＊),
/// removes invisible and bidi control characters while preserving ZWJ (U+200D) and VS-16 (U+FE0F),
/// strips leading notification counts like "(19) " and trailing " - YouTube" / " | YouTube",
/// normalizes non-standard spaces to standard space, collapses repeated spaces, trims trailing dots/spaces,
/// prefixes Windows reserved names with '_', and cuts length so full path stays under 250 characters.
pub fn sanitize_filename(name: &str) -> String {
    // 1. Log code points of last 10 characters
    let last_10: Vec<char> = name.chars().rev().take(10).collect::<Vec<_>>().into_iter().rev().collect();
    let codepoints: Vec<String> = last_10.iter().map(|&c| format!("U+{:04X}", c as u32)).collect();
    tracing::info!(
        "sanitize_filename: input='{}', last_10_chars={:?}, codepoints=[{}]",
        name,
        last_10,
        codepoints.join(", ")
    );

    // 2. Strip leading notification counter e.g. "(19) "
    let mut working = name.trim();
    if working.starts_with('(') {
        if let Some(close_idx) = working.find(')') {
            let inside = &working[1..close_idx];
            if !inside.is_empty() && inside.chars().all(|c| c.is_ascii_digit()) {
                working = working[close_idx + 1..].trim_start();
            }
        }
    }

    // 3. Strip " - YouTube" or " | YouTube" (before extension or at end)
    for suffix in &[" - YouTube", " | YouTube", " - youtube", " | youtube"] {
        if let Some(idx) = working.rfind(suffix) {
            let after = &working[idx + suffix.len()..];
            if after.is_empty() || (after.starts_with('.') && !after[1..].contains('.')) {
                let mut stripped = working[..idx].to_string();
                stripped.push_str(after);
                return sanitize_filename(&stripped);
            }
        }
    }

    let mut mapped = String::with_capacity(working.len());
    let mut prev_space = false;

    for c in working.chars() {
        let u = c as u32;

        // Keep emoji joiners/selectors
        let is_emoji_modifier = u == 0x200D || u == 0xFE0F;

        // Remove invisible & bidi control characters:
        // U+200B (ZWSP), U+200C (ZWNJ), U+200E (LRM), U+200F (RLM)
        // U+202A..=U+202E (Embedding/overrides)
        // U+2060..=U+2064 (Word joiner, invisible math/operators)
        // U+FEFF (BOM/ZWNBSP), U+00AD (Soft hyphen), U+180E (MVS)
        if !is_emoji_modifier && (
            u == 0x200B || u == 0x200C || u == 0x200E || u == 0x200F
            || (0x202A..=0x202E).contains(&u)
            || (0x2060..=0x2064).contains(&u)
            || u == 0xFEFF || u == 0x00AD || u == 0x180E
        ) {
            // Drop completely (do not insert space)
            continue;
        }

        // Replace non-standard spaces with standard space ' ':
        // U+00A0 (NBSP), U+2000..=U+200A (En/Em/Thin/Hair spaces), U+202F (NNBSP), U+205F (MMSP), U+3000 (Ideographic space)
        let is_nonstandard_space = u == 0x00A0
            || (0x2000..=0x200A).contains(&u)
            || u == 0x202F || u == 0x205F || u == 0x3000;

        if is_nonstandard_space || c.is_whitespace() {
            if !prev_space {
                mapped.push(' ');
                prev_space = true;
            }
            continue;
        }

        // Never write U+FFFD () into the filename
        if u == 0xFFFD {
            tracing::warn!("sanitize_filename: dropping U+FFFD replacement character from '{}'", name);
            continue;
        }

        // Drop raw control characters (excluding emoji modifiers) without emitting spaces
        if c.is_control() && !is_emoji_modifier {
            continue;
        }

        // Strip characters forbidden by Windows: < > : " / \ | ? *
        // Replace with space so adjacent words do not concatenate, collapsing any double spaces.
        if matches!(c, '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*') {
            if !prev_space {
                mapped.push(' ');
                prev_space = true;
            }
            continue;
        }

        mapped.push(c);
        prev_space = false;
    }

    let trimmed = mapped
        .trim_matches(|c: char| c.is_whitespace() || c == '.')
        .trim();
    if trimmed.is_empty() {
        return "download.bin".to_string();
    }

    // Trim spaces and dots from the stem before the extension (e.g. "title ❤  .mp4" -> "title ❤.mp4")
    let cleaned_extension = if let Some(dot_idx) = trimmed.rfind('.') {
        let stem = trimmed[..dot_idx].trim_end_matches([' ', '.']);
        let ext = &trimmed[dot_idx..];
        if !stem.is_empty() && ext.chars().count() <= 10 {
            format!("{}{}", stem, ext)
        } else {
            trimmed.to_string()
        }
    } else {
        trimmed.to_string()
    };

    // Protect Windows reserved device names (e.g. CON, NUL, COM1.log, etc.)
    let stem = cleaned_extension.split('.').next().unwrap_or(&cleaned_extension);
    let safe = if is_windows_reserved_name(stem) {
        format!("_{}", cleaned_extension)
    } else {
        cleaned_extension
    };

    // Enforce safe filename length (< 180 chars) to ensure full path stays well under 250 chars
    if safe.chars().count() <= 180 {
        return safe;
    }

    if let Some(dot_idx) = safe.rfind('.') {
        let ext = &safe[dot_idx..];
        if ext.chars().count() <= 20 {
            let max_stem = 180usize.saturating_sub(ext.chars().count());
            let truncated_stem: String = safe[..dot_idx].chars().take(max_stem).collect();
            let result = format!("{}{}", truncated_stem.trim_end_matches([' ', '.']), ext);
            return if result.is_empty() {
                "download.bin".to_string()
            } else {
                result
            };
        }
    }

    safe.chars().take(180).collect()
}

/// If the target file already exists in the destination directory, appends " (1)", " (2)", etc.
/// so the existing file is never overwritten. Returns the unique path and filename.
pub fn resolve_unique_path(save_dir: &Path, filename: &str) -> (PathBuf, String) {
    let sanitized = sanitize_filename(filename);
    let (stem, ext) = if let Some(dot_idx) = sanitized.rfind('.') {
        if dot_idx > 0 {
            (&sanitized[..dot_idx], &sanitized[dot_idx..])
        } else {
            (sanitized.as_str(), "")
        }
    } else {
        (sanitized.as_str(), "")
    };

    let mut candidate_name = sanitized.clone();
    let mut candidate_path = save_dir.join(&candidate_name);
    let mut counter = 1;

    while candidate_path.exists() {
        candidate_name = format!("{} ({}){}", stem, counter, ext);
        candidate_path = save_dir.join(&candidate_name);
        counter += 1;
    }

    (candidate_path, candidate_name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_content_disposition_standard() {
        let header = "attachment; filename=\"ubuntu-24.04-desktop.iso\"";
        assert_eq!(
            parse_content_disposition(header),
            Some("ubuntu-24.04-desktop.iso".to_string())
        );
    }

    #[test]
    fn test_parse_content_disposition_rfc5987() {
        let header = "attachment; filename*=UTF-8''my%20test%20file.pdf";
        assert_eq!(
            parse_content_disposition(header),
            Some("my test file.pdf".to_string())
        );
    }

    #[test]
    fn test_sanitize_filename() {
        assert_eq!(sanitize_filename("valid_name.zip"), "valid_name.zip");
        assert_eq!(sanitize_filename("bad/name:with*chars?.txt"), "bad name with chars.txt");
        assert_eq!(
            sanitize_filename("Famous Alfaaz Video (Full) – Voice of Angels | William Gamborg & Surtaal Music Academy ❤.mp4"),
            "Famous Alfaaz Video (Full) – Voice of Angels William Gamborg & Surtaal Music Academy ❤.mp4"
        );
        assert_eq!(
            sanitize_filename("[2] Famous Alfaaz Video (Full) - William Gamborg | Surtaal Music Academy"),
            "[2] Famous Alfaaz Video (Full) - William Gamborg Surtaal Music Academy"
        );
        assert_eq!(
            sanitize_filename("(19) Famous Alfaaz Video (Full) - YouTube.mp4"),
            "Famous Alfaaz Video (Full).mp4"
        );
        assert_eq!(
            sanitize_filename("Famous Alfaaz Video (Full) | Surtaal Music Academy ❤\u{200e}\u{200b}    .mp4"),
            "Famous Alfaaz Video (Full) Surtaal Music Academy ❤.mp4"
        );
        assert_eq!(
            sanitize_filename("Title with \u{FFFD} replacement.mp4"),
            "Title with replacement.mp4"
        );
        assert_eq!(
            sanitize_filename("(42) Video with ideographic\u{3000}space\u{00a0}and emoji 💖 - YouTube"),
            "Video with ideographic space and emoji 💖"
        );
        assert_eq!(
            sanitize_filename("আমার সোনার বাংলা - পর্ব ১"),
            "আমার সোনার বাংলা - পর্ব ১"
        );
        assert_eq!(sanitize_filename("NUL.mp4"), "_NUL.mp4");
        assert_eq!(sanitize_filename("NUL"), "_NUL");
        assert_eq!(sanitize_filename(".."), "download.bin");
        assert_eq!(sanitize_filename(""), "download.bin");
    }

    #[test]
    fn test_extract_filename_from_url() {
        let url = Url::parse("https://example.com/files/archive%202026.tar.gz?token=abc").unwrap();
        let headers = HeaderMap::new();
        assert_eq!(extract_filename(&headers, &url), "archive 2026.tar.gz");
    }

    #[test]
    fn test_extract_filename_google_drive_image() {
        // Google Drive URLs have paths like /file/d/ID/view — no real filename
        let url = Url::parse("https://drive.google.com/file/d/1aBcDeFgHiJkLmNoPqRs/view").unwrap();
        let mut headers = HeaderMap::new();
        headers.insert(CONTENT_TYPE, "image/png".parse().unwrap());
        let result = extract_filename(&headers, &url);
        assert!(result.ends_with(".png"), "Expected .png extension, got: {}", result);
    }

    #[test]
    fn test_extract_filename_no_extension_with_mime() {
        let url = Url::parse("https://example.com/somefile").unwrap();
        let mut headers = HeaderMap::new();
        headers.insert(CONTENT_TYPE, "image/jpeg".parse().unwrap());
        assert_eq!(extract_filename(&headers, &url), "somefile.jpg");
    }

    #[test]
    fn test_extract_filename_fallback_with_mime() {
        // All segments are generic — should use MIME for default filename extension
        let url = Url::parse("https://example.com/download").unwrap();
        let mut headers = HeaderMap::new();
        headers.insert(CONTENT_TYPE, "application/pdf".parse().unwrap());
        assert_eq!(extract_filename(&headers, &url), "download.pdf");
    }

    #[test]
    fn test_mime_to_extension() {
        assert_eq!(mime_to_extension(Some("image/png")), "png");
        assert_eq!(mime_to_extension(Some("image/jpeg")), "jpg");
        assert_eq!(mime_to_extension(Some("video/mp4")), "mp4");
        assert_eq!(mime_to_extension(Some("application/pdf")), "pdf");
        assert_eq!(mime_to_extension(Some("application/zip")), "zip");
        assert_eq!(mime_to_extension(None), "bin");
    }

    #[test]
    fn test_resolve_unique_path() {
        let temp = tempfile::tempdir().unwrap();
        let p1 = temp.path().join("video.mp4");
        std::fs::write(&p1, b"original").unwrap();

        let (unique_p, unique_name) = resolve_unique_path(temp.path(), "video.mp4");
        assert_eq!(unique_name, "video (1).mp4");
        assert_eq!(unique_p, temp.path().join("video (1).mp4"));

        std::fs::write(&unique_p, b"copy 1").unwrap();
        let (unique_p2, unique_name2) = resolve_unique_path(temp.path(), "video.mp4");
        assert_eq!(unique_name2, "video (2).mp4");
        assert_eq!(unique_p2, temp.path().join("video (2).mp4"));
    }
}
