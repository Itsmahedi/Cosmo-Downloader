use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::process::Command;
use serde::{Deserialize, Serialize};
use tracing::{error, info};

#[cfg(windows)]
use std::os::windows::process::CommandExt;

#[cfg(windows)]
extern "system" {
    fn AllowSetForegroundWindow(dwProcessId: u32) -> i32;
    fn OpenMutexA(dwDesiredAccess: u32, bInheritHandle: i32, lpName: *const i8) -> *mut std::ffi::c_void;
    fn CloseHandle(hObject: *mut std::ffi::c_void) -> i32;
}

#[cfg(windows)]
const ASFW_ANY: u32 = 0xFFFFFFFF;

use rustget_core::{
    db::Database,
    downloader::DownloadOptions,
    queue::QueueManager,
};

const HOST_NAME: &str = "com.cosmodownloader.downloader";
const LEGACY_HOST_NAME: &str = "com.rustget.downloader";

#[derive(Debug, Deserialize)]
#[serde(rename_all = "lowercase")]
enum HostAction {
    Ping,
    Download,
    Status,
    Launch,
    Formats,
    #[serde(alias = "video_formats", alias = "getVideoFormats")]
    VideoFormats,
}

#[derive(Debug, Deserialize)]
struct ExtensionMessage {
    action: Option<HostAction>,
    url: Option<String>,
    filename: Option<String>,
    cookies: Option<String>,
    #[serde(alias = "referrer")]
    referer: Option<String>,
    user_agent: Option<String>,
    num_segments: Option<usize>,
    quality: Option<String>,
    size: Option<u64>,
    size_formatted: Option<String>,
    is_approx: Option<bool>,
    format_id: Option<String>,
}

#[derive(Debug, Default, Serialize)]
struct HostResponse {
    success: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    app_running: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    already_running: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    host_version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    message: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    task_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    formats: Option<rustget_core::youtube::CompactVideoFormats>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

pub async fn run() -> anyhow::Result<()> {
    // Route tracing / logs to stderr ONLY. stdout is strictly for binary Native Messaging frames.
    tracing_subscriber::fmt()
        .with_writer(io::stderr)
        .init();

    let args: Vec<String> = std::env::args().collect();
    if args.len() > 1 {
        match args[1].as_str() {
            "--register" | "register" | "--install" => {
                let manifest_path = if args.len() > 2 {
                    PathBuf::from(&args[2])
                } else {
                    let mut dir = std::env::current_exe()?;
                    dir.pop();
                    dir.push(format!("{}.chrome.json", HOST_NAME));
                    dir
                };
                register_manifests(&manifest_path)?;
                return Ok(());
            }
            "--unregister" | "unregister" => {
                unregister_manifests()?;
                return Ok(());
            }
            "--help" | "-h" => {
                eprintln!("CosmoDownloader Native Messaging Host");
                eprintln!("Usage:");
                eprintln!("  cosmodownloader-host                   Run as Native Messaging host (stdin/stdout)");
                eprintln!("  cosmodownloader-host --register [path] Register host in Chrome, Edge, and Firefox registries");
                eprintln!("  cosmodownloader-host --unregister      Remove registry entries");
                return Ok(());
            }
            _ => {
                // If launched with an origin argument from browser
            }
        }
    }

    info!("CosmoDownloader native messaging host started");

    let mut stdin = io::stdin().lock();
    let mut stdout = io::stdout().lock();

    loop {
        // Read 4-byte message length (little-endian)
        let mut len_bytes = [0u8; 4];
        if let Err(e) = stdin.read_exact(&mut len_bytes) {
            if e.kind() == io::ErrorKind::UnexpectedEof {
                // Extension closed the pipe
                break;
            }
            error!("Failed to read message length: {}", e);
            break;
        }

        let len = u32::from_ne_bytes(len_bytes) as usize;
        if len == 0 || len > 1024 * 1024 {
            error!("Invalid message length: {} bytes", len);
            break;
        }

        let mut body = vec![0u8; len];
        if let Err(e) = stdin.read_exact(&mut body) {
            error!("Failed to read message body: {}", e);
            break;
        }

        let response = match serde_json::from_slice::<ExtensionMessage>(&body) {
            Ok(msg) => handle_message(msg).await,
            Err(e) => HostResponse {
                success: false,
                app_running: None,
                already_running: None,
                host_version: Some(env!("CARGO_PKG_VERSION").to_string()),
                message: None,
                task_id: None,
                formats: None,
                error: Some(format!("Invalid JSON request: {}", e)),
            },
        };

        let resp_json = match serde_json::to_vec(&response) {
            Ok(bytes) => bytes,
            Err(e) => {
                error!("Failed to serialize response: {}", e);
                continue;
            }
        };

        let resp_len = (resp_json.len() as u32).to_ne_bytes();
        if stdout.write_all(&resp_len).is_err() || stdout.write_all(&resp_json).is_err() || stdout.flush().is_err() {
            error!("Failed to write response to stdout");
            break;
        }
    }

    Ok(())
}

async fn handle_message(msg: ExtensionMessage) -> HostResponse {
    match msg.action {
        Some(HostAction::Ping) => HostResponse {
            success: true,
            app_running: None,
            already_running: None,
            host_version: Some(env!("CARGO_PKG_VERSION").to_string()),
            message: Some(format!("pong: Cosmo Downloader Host v{}", env!("CARGO_PKG_VERSION"))),
            task_id: None,
            formats: None,
            error: None,
        },
        Some(HostAction::Status) => {
            let (running, port) = check_bridge_health().await;
            let process_alive = is_app_running_on_system();
            let is_running = running || process_alive;
            HostResponse {
                success: true,
                app_running: Some(is_running),
                already_running: Some(is_running),
                host_version: Some(env!("CARGO_PKG_VERSION").to_string()),
                message: Some(if is_running {
                    format!("CosmoDownloader GUI is active (port {})", if port > 0 { port } else { 24242 })
                } else {
                    "CosmoDownloader GUI is offline".to_string()
                }),
                task_id: None,
                formats: None,
                error: None,
            }
        }
        Some(HostAction::Launch) => {
            let (running, port) = check_bridge_health().await;
            let process_alive = is_app_running_on_system();
            if running {
                let _ = trigger_show_window(port).await;
                HostResponse {
                    success: true,
                    app_running: Some(true),
                    already_running: Some(true),
                    host_version: Some(env!("CARGO_PKG_VERSION").to_string()),
                    message: Some("CosmoDownloader is already running; brought to front".to_string()),
                    task_id: None,
                    formats: None,
                    error: None,
                }
            } else if process_alive {
                let _ = trigger_show_window(0).await;
                #[cfg(windows)]
                unsafe {
                    AllowSetForegroundWindow(ASFW_ANY);
                }
                HostResponse {
                    success: true,
                    app_running: Some(true),
                    already_running: Some(true),
                    host_version: Some(env!("CARGO_PKG_VERSION").to_string()),
                    message: Some("CosmoDownloader is already running; brought to front via IPC".to_string()),
                    task_id: None,
                    formats: None,
                    error: None,
                }
            } else if let Some(exe_path) = find_app_exe() {
                let mut cmd = Command::new(&exe_path);
                cmd.stdin(std::process::Stdio::null());
                cmd.stdout(std::process::Stdio::null());
                cmd.stderr(std::process::Stdio::null());
                #[cfg(windows)]
                {
                    const DETACHED_PROCESS: u32 = 0x00000008;
                    const CREATE_NO_WINDOW: u32 = 0x08000000;
                    cmd.creation_flags(DETACHED_PROCESS | CREATE_NO_WINDOW);
                }
                let spawn_res = cmd.spawn();
                #[cfg(windows)]
                if let Ok(ref child) = spawn_res {
                    unsafe {
                        AllowSetForegroundWindow(child.id());
                        AllowSetForegroundWindow(ASFW_ANY);
                    }
                }
                match spawn_res {
                    Ok(_) => HostResponse {
                        success: true,
                        app_running: Some(false),
                        already_running: Some(false),
                        host_version: Some(env!("CARGO_PKG_VERSION").to_string()),
                        message: Some("CosmoDownloader spawned successfully".to_string()),
                        task_id: None,
                        formats: None,
                        error: None,
                    },
                    Err(e) => HostResponse {
                        success: false,
                        app_running: Some(false),
                        already_running: Some(false),
                        host_version: Some(env!("CARGO_PKG_VERSION").to_string()),
                        message: None,
                        task_id: None,
                        formats: None,
                        error: Some(format!("Failed to spawn CosmoDownloader: {}", e)),
                    },
                }
            } else {
                HostResponse {
                    success: false,
                    app_running: Some(false),
                    already_running: Some(false),
                    host_version: Some(env!("CARGO_PKG_VERSION").to_string()),
                    message: None,
                    task_id: None,
                    formats: None,
                    error: Some("CosmoDownloader executable not found".to_string()),
                }
            }
        }
        Some(HostAction::Formats) | Some(HostAction::VideoFormats) => {
            let url = match msg.url.as_deref() {
                Some(u) if !u.trim().is_empty() => u.trim().to_string(),
                _ => {
                    return HostResponse {
                        success: false,
                        error: Some("Missing 'url' parameter in formats request".to_string()),
                        ..Default::default()
                    };
                }
            };

            let (running, port) = check_bridge_health().await;
            if running {
                if let Ok(formats) = forward_formats_to_bridge_server(port, &url).await {
                    return HostResponse {
                        success: true,
                        app_running: Some(true),
                        message: Some("Formats retrieved via CosmoDownloader bridge".to_string()),
                        formats: Some(formats),
                        ..Default::default()
                    };
                }
            }

            match rustget_core::youtube::query_video_formats(&url).await {
                Ok(formats) => HostResponse {
                    success: true,
                    app_running: Some(running),
                    message: Some("Formats retrieved directly".to_string()),
                    formats: Some(formats),
                    ..Default::default()
                },
                Err(err) => HostResponse {
                    success: false,
                    app_running: Some(running),
                    error: Some(err),
                    ..Default::default()
                },
            }
        }
        _ => {
            let url = match msg.url.as_deref() {
                Some(u) if !u.trim().is_empty() => u.trim().to_string(),
                _ => {
                    return HostResponse {
                        success: false,
                        app_running: None,
                        already_running: None,
                        host_version: Some(env!("CARGO_PKG_VERSION").to_string()),
                        message: None,
                        task_id: None,
                        formats: None,
                        error: Some("Missing 'url' parameter in download request".to_string()),
                    };
                }
            };

            // First attempt: HTTP POST to local bridge server (if CosmoDownloader GUI or CLI queue is running)
            #[cfg(windows)]
            unsafe {
                AllowSetForegroundWindow(ASFW_ANY);
            }
            let (running, port) = check_bridge_health().await;
            if running {
                if let Ok(task_id) = forward_to_bridge_server(port, &url, &msg).await {
                    #[cfg(windows)]
                    unsafe {
                        AllowSetForegroundWindow(ASFW_ANY);
                    }
                    return HostResponse {
                        success: true,
                        app_running: Some(true),
                        already_running: None,
                        host_version: Some(env!("CARGO_PKG_VERSION").to_string()),
                        message: Some("Download sent to CosmoDownloader bridge server".to_string()),
                        task_id: Some(task_id),
                        formats: None,
                        error: None,
                    };
                }
            }

            // If app process is running, deliver directly to its IPC requests queue
            if is_app_running_on_system() {
                if let Some(base) = directories::BaseDirs::new() {
                    let ipc_dir = base.data_local_dir().join("CosmoDownloader").join("ipc_requests");
                    let _ = std::fs::create_dir_all(&ipc_dir);
                    let payload = serde_json::json!({
                        "url": url,
                        "filename": msg.filename,
                        "cookies": msg.cookies,
                        "referer": msg.referer,
                        "user_agent": msg.user_agent,
                        "num_segments": msg.num_segments.unwrap_or(8),
                        "quality": msg.quality,
                        "size": msg.size,
                        "size_formatted": msg.size_formatted,
                        "is_approx": msg.is_approx,
                        "format_id": msg.format_id,
                    });
                    if std::fs::write(ipc_dir.join(format!("req_{}.json", uuid::Uuid::new_v4())), payload.to_string()).is_ok() {
                        #[cfg(windows)]
                        unsafe {
                            AllowSetForegroundWindow(ASFW_ANY);
                        }
                        return HostResponse {
                            success: true,
                            app_running: Some(true),
                            already_running: Some(true),
                            host_version: Some(env!("CARGO_PKG_VERSION").to_string()),
                            message: Some("Download sent to running CosmoDownloader via IPC".to_string()),
                            task_id: Some("ipc".to_string()),
                            formats: None,
                            error: None,
                        };
                    }
                }
            }

            // If GUI/bridge is offline, launch on-demand popup
            if let Some(exe) = find_app_exe() {
                let temp_dir = std::env::temp_dir();
                let timestamp = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_millis())
                    .unwrap_or(0);
                let temp_file = temp_dir.join(format!("cosmo_dl_req_{}_{}.json", std::process::id(), timestamp));
                let payload = serde_json::json!({
                    "url": url,
                    "filename": msg.filename,
                    "cookies": msg.cookies,
                    "referer": msg.referer,
                    "user_agent": msg.user_agent,
                    "num_segments": msg.num_segments.unwrap_or(8),
                    "quality": msg.quality,
                    "size": msg.size,
                    "size_formatted": msg.size_formatted,
                    "is_approx": msg.is_approx,
                    "format_id": msg.format_id,
                });
                if std::fs::write(&temp_file, payload.to_string()).is_ok() {
                    let mut popup_cmd = Command::new(&exe);
                    popup_cmd.stdin(std::process::Stdio::null());
                    popup_cmd.stdout(std::process::Stdio::null());
                    popup_cmd.stderr(std::process::Stdio::null());
                    #[cfg(windows)]
                    {
                        const DETACHED_PROCESS: u32 = 0x00000008;
                        const CREATE_NO_WINDOW: u32 = 0x08000000;
                        popup_cmd.creation_flags(DETACHED_PROCESS | CREATE_NO_WINDOW);
                    }
                    popup_cmd.arg("--from-extension").arg("--popup-file").arg(&temp_file);
                    let _spawn_res = popup_cmd.spawn();
                    #[cfg(windows)]
                    if let Ok(ref child) = _spawn_res {
                        unsafe {
                            AllowSetForegroundWindow(child.id());
                            AllowSetForegroundWindow(ASFW_ANY);
                        }
                    }

                    return HostResponse {
                        success: true,
                        app_running: Some(false),
                        already_running: None,
                        host_version: Some(env!("CARGO_PKG_VERSION").to_string()),
                        message: Some("CosmoDownloader on-demand popup opened".to_string()),
                        task_id: Some("popup".to_string()),
                        formats: None,
                        error: None,
                    };
                }
            }

            // Fallback: Directly insert into SQLite database via QueueManager
            match queue_directly_in_db(&url, msg.filename.as_deref(), msg.cookies, msg.referer, msg.user_agent, msg.num_segments.unwrap_or(8)).await {
                Ok(task_id) => HostResponse {
                    success: true,
                    app_running: Some(false),
                    already_running: None,
                    host_version: Some(env!("CARGO_PKG_VERSION").to_string()),
                    message: Some("Download saved directly to CosmoDownloader database queue".to_string()),
                    task_id: Some(task_id),
                    formats: None,
                    error: None,
                },
                Err(e) => HostResponse {
                    success: false,
                    app_running: Some(false),
                    already_running: None,
                    host_version: Some(env!("CARGO_PKG_VERSION").to_string()),
                    message: None,
                    task_id: None,
                    formats: None,
                    error: Some(format!("Failed to queue download: {}", e)),
                },
            }
        }
    }
}

fn is_app_running_on_system() -> bool {
    #[cfg(windows)]
    unsafe {
        let h_mutex = OpenMutexA(
            0x00100000 | 0x0001, // SYNCHRONIZE | MUTEX_MODIFY_STATE
            0,
            c"CosmoDownloader_v1_0_SingleInstance_Mutex".as_ptr().cast(),
        );
        if !h_mutex.is_null() {
            CloseHandle(h_mutex);
            return true;
        }
    }
    false
}

fn get_active_bridge_port() -> Option<u16> {
    if let Some(base) = directories::BaseDirs::new() {
        let port_file = base.data_local_dir().join("CosmoDownloader").join("bridge_port.txt");
        if let Ok(content) = std::fs::read_to_string(&port_file) {
            if let Ok(port) = content.trim().parse::<u16>() {
                return Some(port);
            }
        }
    }
    None
}

async fn check_bridge_health() -> (bool, u16) {
    let mut candidate_ports = vec![42424, 24242, 18765];
    if let Some(active) = get_active_bridge_port() {
        candidate_ports.retain(|&p| p != active);
        candidate_ports.insert(0, active);
    }

    let client = match reqwest::Client::builder()
        .timeout(std::time::Duration::from_millis(400))
        .build()
    {
        Ok(c) => c,
        Err(_) => return (false, 0),
    };

    for port in candidate_ports {
        let url = format!("http://127.0.0.1:{}/health", port);
        if let Ok(resp) = client.get(&url).send().await {
            if resp.status().is_success() {
                return (true, port);
            }
        }
    }
    (false, 0)
}

async fn forward_to_bridge_server(
    port: u16,
    url: &str,
    msg: &ExtensionMessage,
) -> anyhow::Result<String> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(2))
        .build()?;

    let payload = serde_json::json!({
        "url": url,
        "filename": msg.filename,
        "cookies": msg.cookies,
        "referer": msg.referer,
        "user_agent": msg.user_agent,
        "num_segments": msg.num_segments.unwrap_or(8),
        "quality": msg.quality,
        "size": msg.size,
        "size_formatted": msg.size_formatted,
        "is_approx": msg.is_approx,
        "format_id": msg.format_id,
    });

    let bridge_url = format!("http://127.0.0.1:{}/download", port);
    let resp = client.post(&bridge_url)
        .header("Content-Type", "application/json")
        .body(payload.to_string())
        .send()
        .await?;

    let status = resp.status();
    if status.is_success() {
        let body_str = resp.text().await?;
        let v: serde_json::Value = serde_json::from_str(&body_str)?;
        if let Some(task_id) = v.get("task_id").and_then(|t| t.as_str()) {
            return Ok(task_id.to_string());
        }
        return Ok("queued".to_string());
    }

    anyhow::bail!("Bridge server returned status: {}", status)
}

async fn forward_formats_to_bridge_server(
    port: u16,
    url: &str,
) -> anyhow::Result<rustget_core::youtube::CompactVideoFormats> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(8))
        .build()?;

    let bridge_url = format!("http://127.0.0.1:{}/formats", port);
    let payload = serde_json::json!({ "url": url });
    let resp = client.post(&bridge_url)
        .header("Content-Type", "application/json")
        .body(payload.to_string())
        .send()
        .await?;

    let status = resp.status();
    if status.is_success() {
        let body_str = resp.text().await?;
        let v: serde_json::Value = serde_json::from_str(&body_str)?;
        if let Some(formats_val) = v.get("formats") {
            let formats: rustget_core::youtube::CompactVideoFormats = serde_json::from_value(formats_val.clone())?;
            return Ok(formats);
        }
    }

    anyhow::bail!("Bridge server returned status: {}", status)
}

async fn queue_directly_in_db(
    url: &str,
    custom_filename: Option<&str>,
    cookies: Option<String>,
    referer: Option<String>,
    user_agent: Option<String>,
    num_segments: usize,
) -> anyhow::Result<String> {
    let db = Database::open_default()?;
    let (tx, _rx) = tokio::sync::mpsc::channel(16);
    let queue = QueueManager::new(db, 3, 0, tx);

    let output_path = custom_filename.map(PathBuf::from);
    let options = DownloadOptions {
        url: url.to_string(),
        output_path,
        num_segments,
        cookie: cookies,
        referer,
        user_agent,
        ..Default::default()
    };

    let id = queue.add_download(options, None, 0).await?;
    info!("Directly inserted download {} to SQLite database queue", id);
    Ok(id)
}

fn find_app_exe() -> Option<PathBuf> {
    // 1. In the same directory as this native-host binary
    if let Ok(mut exe) = std::env::current_exe() {
        exe.pop();
        let candidate_win = exe.join("CosmoDownloader.exe");
        if candidate_win.is_file() {
            return Some(candidate_win);
        }
        let candidate_legacy = exe.join("rustget-gui.exe");
        if candidate_legacy.is_file() {
            return Some(candidate_legacy);
        }
    }

    // 2. Query Windows Registry for Cosmo Downloader install / startup path
    #[cfg(windows)]
    {
        for (key, val) in [
            (r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run", "Cosmo Downloader"),
            (r"HKCU\Software\Microsoft\Windows\CurrentVersion\Uninstall\{D37E7492-3580-4D3B-A924-E8251C9B71C8}_is1", "InstallLocation"),
            (r"HKLM\Software\Microsoft\Windows\CurrentVersion\Uninstall\{D37E7492-3580-4D3B-A924-E8251C9B71C8}_is1", "InstallLocation"),
        ] {
            if let Ok(out) = std::process::Command::new("reg")
                .args(["query", key, "/v", val])
                .creation_flags(0x08000000)
                .output()
            {
                if out.status.success() {
                    let s = String::from_utf8_lossy(&out.stdout);
                    for token in s.split('"') {
                        let p = PathBuf::from(token.trim());
                        if p.is_file() {
                            return Some(p);
                        }
                        if p.is_dir() && p.join("CosmoDownloader.exe").is_file() {
                            return Some(p.join("CosmoDownloader.exe"));
                        }
                    }
                    if let Some(idx) = s.find("REG_SZ") {
                        let raw_val = s[idx + 6..].trim().trim_matches('"');
                        let p = PathBuf::from(raw_val);
                        if p.is_dir() && p.join("CosmoDownloader.exe").is_file() {
                            return Some(p.join("CosmoDownloader.exe"));
                        }
                    }
                }
            }
        }
    }

    // 3. Standard non-admin & per-user Inno Setup install directories
    if let Some(dirs) = directories::BaseDirs::new() {
        for candidate in [
            dirs.data_local_dir().join("Programs").join("Cosmo Downloader").join("CosmoDownloader.exe"),
            dirs.data_local_dir().join("Cosmo Downloader").join("CosmoDownloader.exe"),
        ] {
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }

    None
}

async fn trigger_show_window(port: u16) -> anyhow::Result<()> {
    if port > 0 {
        if let Ok(client) = reqwest::Client::builder()
            .timeout(std::time::Duration::from_millis(800))
            .build()
        {
            let token = rustget_core::server::get_or_create_bridge_token();
            let show_url = format!("http://127.0.0.1:{}/show", port);
            let _ = client.post(&show_url)
                .header("X-Cosmo-Token", token.trim())
                .send()
                .await;
        }
    }
    // Also signal via file IPC as fallback
    if let Some(base) = directories::BaseDirs::new() {
        let ipc_dir = base.data_local_dir().join("CosmoDownloader").join("ipc_requests");
        let _ = std::fs::create_dir_all(&ipc_dir);
        let _ = std::fs::write(ipc_dir.join(format!("show_{}.json", uuid::Uuid::new_v4())), "{}");
    }
    Ok(())
}

#[cfg(windows)]
fn reg_add_default_str(key: &str, value: &str) {
    let _ = Command::new("reg")
        .args(["add", key, "/ve", "/t", "REG_SZ", "/d", value, "/f"])
        .creation_flags(0x08000000)
        .output();
}

#[cfg(not(windows))]
fn reg_add_default_str(_key: &str, _value: &str) {}

#[cfg(windows)]
fn reg_delete_key(key: &str) {
    let _ = Command::new("reg")
        .args(["delete", key, "/f"])
        .creation_flags(0x08000000)
        .output();
}

#[cfg(not(windows))]
fn reg_delete_key(_key: &str) {}

fn register_manifests(chrome_manifest: &Path) -> anyhow::Result<()> {
    let manifest_dir = chrome_manifest.parent().unwrap_or_else(|| Path::new("."));
    let chrome_file = if chrome_manifest.exists() {
        chrome_manifest.to_path_buf()
    } else {
        manifest_dir.join(format!("{}.chrome.json", HOST_NAME))
    };
    let edge_candidate = manifest_dir.join(format!("{}.edge.json", HOST_NAME));
    let edge_file = if edge_candidate.exists() { &edge_candidate } else { &chrome_file };
    let firefox_file = manifest_dir.join(format!("{}.firefox.json", HOST_NAME));

    let chrome_str = chrome_file.to_string_lossy().to_string();
    let edge_str = edge_file.to_string_lossy().to_string();

    for reg_root in [
        r"HKCU\Software\Google\Chrome\NativeMessagingHosts",
        r"HKCU\Software\BraveSoftware\Brave-Browser\NativeMessagingHosts",
        r"HKCU\Software\Chromium\NativeMessagingHosts",
        r"HKCU\Software\Opera Software\Opera Stable\NativeMessagingHosts",
    ] {
        if chrome_file.exists() {
            reg_add_default_str(&format!(r"{}\{}", reg_root, HOST_NAME), &chrome_str);
        }
    }

    if edge_file.exists() {
        reg_add_default_str(
            &format!(r"HKCU\Software\Microsoft\Edge\NativeMessagingHosts\{}", HOST_NAME),
            &edge_str,
        );
    }

    // Never overwrite Firefox NativeMessagingHosts with the Chrome manifest.
    if firefox_file.exists() {
        let firefox_str = firefox_file.to_string_lossy().to_string();
        reg_add_default_str(
            &format!(r"HKCU\Software\Mozilla\NativeMessagingHosts\{}", HOST_NAME),
            &firefox_str,
        );
    }

    eprintln!("Registered CosmoDownloader native messaging hosts in Windows Registry.");
    Ok(())
}

fn unregister_manifests() -> anyhow::Result<()> {
    for reg_root in [
        r"HKCU\Software\Google\Chrome\NativeMessagingHosts",
        r"HKCU\Software\BraveSoftware\Brave-Browser\NativeMessagingHosts",
        r"HKCU\Software\Chromium\NativeMessagingHosts",
        r"HKCU\Software\Opera Software\Opera Stable\NativeMessagingHosts",
        r"HKCU\Software\Microsoft\Edge\NativeMessagingHosts",
        r"HKCU\Software\Mozilla\NativeMessagingHosts",
    ] {
        for name in [HOST_NAME, LEGACY_HOST_NAME] {
            reg_delete_key(&format!(r"{}\{}", reg_root, name));
        }
    }

    eprintln!("Unregistered Native Messaging hosts from Windows Registry.");
    Ok(())
}
