use std::collections::VecDeque;
use std::fs::{self, OpenOptions};
use std::io::{BufWriter, Write};
use std::path::PathBuf;
use std::sync::{
    mpsc::{self, Sender},
    Mutex, OnceLock,
};
use std::time::{Duration, Instant};

const MAX_LOG_FILES: usize = 5;
const MAX_LOG_FILE_BYTES: u64 = 2 * 1024 * 1024; // 2 MB
const MAX_RECENT_LOGS: usize = 300;

enum LogCmd {
    Write(String),
    Flush(Sender<()>),
}

static LOG_TX: OnceLock<Sender<LogCmd>> = OnceLock::new();
static RECENT_LOGS: OnceLock<Mutex<VecDeque<String>>> = OnceLock::new();

fn get_recent_buffer() -> &'static Mutex<VecDeque<String>> {
    RECENT_LOGS.get_or_init(|| Mutex::new(VecDeque::with_capacity(MAX_RECENT_LOGS)))
}

/// Returns `%LOCALAPPDATA%\Cosmo Downloader\logs` on Windows,
/// or platform fallback on non-Windows.
pub fn get_logs_dir() -> PathBuf {
    #[cfg(windows)]
    {
        if let Some(local_app_data) = std::env::var_os("LOCALAPPDATA") {
            let p = PathBuf::from(local_app_data).join("Cosmo Downloader").join("logs");
            if fs::create_dir_all(&p).is_ok() {
                return p;
            }
        }
    }
    // Cross-platform fallback
    let p = directories::BaseDirs::new()
        .map(|d| d.data_local_dir().join("Cosmo Downloader").join("logs"))
        .unwrap_or_else(|| std::env::temp_dir().join("Cosmo Downloader").join("logs"));
    let _ = fs::create_dir_all(&p);
    p
}

/// Rotates log files in logs_dir if cosmodownloader.log exceeds MAX_LOG_FILE_BYTES.
/// cosmodownloader.log -> cosmodownloader.log.1 -> ... -> cosmodownloader.log.4 (5 files total)
fn rotate_logs_if_needed(logs_dir: &std::path::Path) {
    let main_log = logs_dir.join("cosmodownloader.log");
    if let Ok(meta) = fs::metadata(&main_log) {
        if meta.len() >= MAX_LOG_FILE_BYTES {
            // Delete oldest if exists (cosmodownloader.log.4)
            let oldest = logs_dir.join(format!("cosmodownloader.log.{}", MAX_LOG_FILES - 1));
            if oldest.exists() {
                let _ = fs::remove_file(&oldest);
            }
            // Shift down: .3 -> .4, .2 -> .3, .1 -> .2
            for i in (1..(MAX_LOG_FILES - 1)).rev() {
                let from = logs_dir.join(format!("cosmodownloader.log.{}", i));
                let to = logs_dir.join(format!("cosmodownloader.log.{}", i + 1));
                if from.exists() {
                    let _ = fs::rename(&from, &to);
                }
            }
            // Shift main log to .1
            let to_first = logs_dir.join("cosmodownloader.log.1");
            let _ = fs::rename(&main_log, &to_first);
        }
    }
}

fn init_logger() -> Sender<LogCmd> {
    let (tx, rx) = mpsc::channel::<LogCmd>();
    let _ = std::thread::Builder::new()
        .name("cosmo_logger".to_string())
        .spawn(move || {
            let logs_dir = get_logs_dir();
            let _ = fs::create_dir_all(&logs_dir);
            let main_log_path = logs_dir.join("cosmodownloader.log");

            rotate_logs_if_needed(&logs_dir);

            let mut current_file = OpenOptions::new()
                .create(true)
                .append(true)
                .open(&main_log_path)
                .ok();
            let mut writer = current_file.take().map(|f| BufWriter::with_capacity(65536, f));
            let mut current_bytes: u64 = fs::metadata(&main_log_path).map(|m| m.len()).unwrap_or(0);
            let mut last_flush = Instant::now();

            while let Ok(cmd) = rx.recv() {
                match cmd {
                    LogCmd::Write(line) => {
                        let line_bytes = line.len() as u64;
                        if current_bytes + line_bytes >= MAX_LOG_FILE_BYTES {
                            if let Some(mut w) = writer.take() {
                                let _ = w.flush();
                            }
                            rotate_logs_if_needed(&logs_dir);
                            current_file = OpenOptions::new()
                                .create(true)
                                .append(true)
                                .open(&main_log_path)
                                .ok();
                            writer = current_file.take().map(|f| BufWriter::with_capacity(65536, f));
                            current_bytes = fs::metadata(&main_log_path).map(|m| m.len()).unwrap_or(0);
                        }

                        if let Some(ref mut w) = writer {
                            let _ = w.write_all(line.as_bytes());
                            current_bytes += line_bytes;
                        }

                        // Drain any available lines in the channel without blocking
                        while let Ok(next) = rx.try_recv() {
                            match next {
                                LogCmd::Write(next_line) => {
                                    let n_bytes = next_line.len() as u64;
                                    if current_bytes + n_bytes >= MAX_LOG_FILE_BYTES {
                                        if let Some(mut w) = writer.take() {
                                            let _ = w.flush();
                                        }
                                        rotate_logs_if_needed(&logs_dir);
                                        current_file = OpenOptions::new()
                                            .create(true)
                                            .append(true)
                                            .open(&main_log_path)
                                            .ok();
                                        writer = current_file.take().map(|f| BufWriter::with_capacity(65536, f));
                                        current_bytes = fs::metadata(&main_log_path).map(|m| m.len()).unwrap_or(0);
                                    }
                                    if let Some(ref mut w) = writer {
                                        let _ = w.write_all(next_line.as_bytes());
                                        current_bytes += n_bytes;
                                    }
                                }
                                LogCmd::Flush(reply) => {
                                    if let Some(ref mut w) = writer {
                                        let _ = w.flush();
                                    }
                                    let _ = reply.send(());
                                }
                            }
                        }

                        if last_flush.elapsed() >= Duration::from_millis(1000) {
                            if let Some(ref mut w) = writer {
                                let _ = w.flush();
                            }
                            last_flush = Instant::now();
                        }
                    }
                    LogCmd::Flush(reply) => {
                        if let Some(ref mut w) = writer {
                            let _ = w.flush();
                        }
                        let _ = reply.send(());
                    }
                }
            }

            if let Some(ref mut w) = writer {
                let _ = w.flush();
            }
        });
    tx
}

/// Log an event with optional request / dialog / task id.
/// Cheap, buffered, zero disk I/O in the caller's thread.
pub fn log_event(id: Option<&str>, msg: &str) {
    let timestamp = chrono::Local::now().format("%Y-%m-%d %H:%M:%S%.3f");
    let thread_name = std::thread::current().name().unwrap_or("unnamed").to_string();
    let thread_id = std::thread::current().id();

    let id_str = match id {
        Some(d) if !d.is_empty() => format!("[{}]", d),
        _ => String::new(),
    };

    let line = format!("[{}][{}/{:?}]{} {}\n", timestamp, thread_name, thread_id, id_str, msg);
    eprint!("{}", line);

    // Keep recent log entries in memory
    if let Ok(mut buf) = get_recent_buffer().lock() {
        if buf.len() >= MAX_RECENT_LOGS {
            buf.pop_front();
        }
        buf.push_back(line.clone());
    }

    let sender = LOG_TX.get_or_init(init_logger);
    let _ = sender.send(LogCmd::Write(line));
}

/// Convenience logging wrapper for legacy platform calls
pub fn log_daemon(msg: &str) {
    log_event(None, msg);
}

/// Synchronously flush all pending buffered log lines to disk.
pub fn flush_logs() {
    if let Some(sender) = LOG_TX.get() {
        let (ack_tx, ack_rx) = mpsc::channel();
        if sender.send(LogCmd::Flush(ack_tx)).is_ok() {
            let _ = ack_rx.recv_timeout(Duration::from_millis(2000));
        }
    }
}

/// Retrieve the last `count` log lines from the in-memory ring buffer.
pub fn get_recent_logs(count: usize) -> Vec<String> {
    if let Ok(buf) = get_recent_buffer().lock() {
        let skip = buf.len().saturating_sub(count);
        buf.iter().skip(skip).cloned().collect()
    } else {
        Vec::new()
    }
}

/// Write a full crash report to disk and return the path of the timestamped crash file.
pub fn write_crash_report(
    panic_msg: &str,
    location: Option<&str>,
    backtrace: &std::backtrace::Backtrace,
) -> PathBuf {
    let logs_dir = get_logs_dir();
    let timestamp = chrono::Local::now().format("%Y%m%d_%H%M%S");
    let crash_file = logs_dir.join(format!("crash_{}.txt", timestamp));
    let latest_crash_file = logs_dir.join("crash.txt");

    let os_info = format!("{} {}", std::env::consts::OS, std::env::consts::ARCH);
    let recent = get_recent_logs(200);

    let content = format!(
        "=====================================================\n\
         COSMO DOWNLOADER CRASH REPORT\n\
         =====================================================\n\
         Timestamp: {}\n\
         App Version: {}\n\
         Platform: {}\n\
         Thread: {:?}\n\
         Location: {}\n\
         \n\
         PANIC REASON:\n\
         {}\n\
         \n\
         BACKTRACE:\n\
         {:?}\n\
         \n\
         =====================================================\n\
         RECENT LOGS (Last {} lines):\n\
         =====================================================\n\
         {}\n",
        chrono::Local::now().format("%Y-%m-%d %H:%M:%S%.3f"),
        env!("CARGO_PKG_VERSION"),
        os_info,
        std::thread::current().name(),
        location.unwrap_or("unknown"),
        panic_msg,
        backtrace,
        recent.len(),
        recent.concat()
    );

    let _ = fs::write(&crash_file, &content);
    let _ = fs::write(&latest_crash_file, &content);

    crash_file
}

/// Install custom panic hook to log panics, write a crash report, and show an error dialog.
pub fn install_panic_hook() {
    std::panic::set_hook(Box::new(|info| {
        let msg = if let Some(s) = info.payload().downcast_ref::<&str>() {
            (*s).to_string()
        } else if let Some(s) = info.payload().downcast_ref::<String>() {
            s.clone()
        } else {
            "Unknown panic payload".to_string()
        };

        let location = info.location().map(|l| format!("{}:{}:{}", l.file(), l.line(), l.column()));
        let bt = std::backtrace::Backtrace::force_capture();

        log_event(None, &format!("FATAL PANIC: {} at {:?}", msg, location));
        let crash_path = write_crash_report(&msg, location.as_deref(), &bt);
        flush_logs();

        // Show error window
        #[cfg(windows)]
        {
            #[link(name = "user32")]
            extern "system" {
                fn MessageBoxW(hwnd: isize, text: *const u16, caption: *const u16, u_type: u32) -> i32;
            }
            const MB_OK: u32 = 0x00000000;
            const MB_ICONERROR: u32 = 0x00000010;
            const MB_TASKMODAL: u32 = 0x00002000;
            const MB_SETFOREGROUND: u32 = 0x00010000;

            let title: Vec<u16> = "Cosmo Downloader - Unexpected Error\0".encode_utf16().collect();
            let text: Vec<u16> = format!(
                "Cosmo Downloader encountered an unexpected problem and must close.\n\n\
                 Error: {}\n\
                 Location: {}\n\n\
                 A crash report was written to:\n{}\n\n\
                 Please report this issue if it recurs.\0",
                msg,
                location.as_deref().unwrap_or("unknown"),
                crash_path.display()
            )
            .encode_utf16()
            .collect();

            unsafe {
                MessageBoxW(
                    0,
                    text.as_ptr(),
                    title.as_ptr(),
                    MB_ICONERROR | MB_OK | MB_TASKMODAL | MB_SETFOREGROUND,
                );
            }
        }

        #[cfg(not(windows))]
        {
            eprintln!(
                "\n*****************************************************\n\
                 FATAL ERROR: Cosmo Downloader encountered an unexpected error.\n\
                 Error: {}\n\
                 Crash report written to: {}\n\
                 *****************************************************\n",
                msg,
                crash_path.display()
            );
        }
    }));
}
