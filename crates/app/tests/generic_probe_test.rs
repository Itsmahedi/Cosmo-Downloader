use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;
use url::Url;

use rustget_app::dialogs::download_window::{
    DownloadWindowOutcome, DownloadWindowState, DownloadWindowStage, InfoLookupStatus,
};
use rustget_core::probe::{extract_filename, probe_url};

#[test]
fn test_monotonic_dialog_ids() {
    let d1 = DownloadWindowState::new(
        "https://example.com/file1.zip".to_string(),
        "file1.zip".to_string(),
        "General".to_string(),
        "C:\\Downloads".to_string(),
    );
    let d2 = DownloadWindowState::new(
        "https://example.com/file2.zip".to_string(),
        "file2.zip".to_string(),
        "General".to_string(),
        "C:\\Downloads".to_string(),
    );
    let d3 = DownloadWindowState::new(
        "https://example.com/file3.zip".to_string(),
        "file3.zip".to_string(),
        "General".to_string(),
        "C:\\Downloads".to_string(),
    );

    assert!(d1.id < d2.id, "d1.id ({}) must be strictly less than d2.id ({})", d1.id, d2.id);
    assert!(d2.id < d3.id, "d2.id ({}) must be strictly less than d3.id ({})", d2.id, d3.id);
    assert_ne!(d1.viewport_id(), d2.viewport_id(), "ViewportIds must be unique");
    assert_ne!(d2.viewport_id(), d3.viewport_id(), "ViewportIds must be unique");
}

#[tokio::test]
async fn test_generic_probe_200_ok_with_rfc5987() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("Failed to bind listener");
    let addr = listener.local_addr().unwrap();
    let stop = Arc::new(AtomicBool::new(false));
    let stop_clone = stop.clone();

    let server_handle = thread::spawn(move || {
        listener.set_nonblocking(true).unwrap();
        while !stop_clone.load(Ordering::Relaxed) {
            if let Ok((mut stream, _)) = listener.accept() {
                let mut buf = [0u8; 1024];
                let _ = stream.read(&mut buf);
                let req_str = String::from_utf8_lossy(&buf);

                if req_str.starts_with("HEAD") {
                    let resp = "HTTP/1.1 200 OK\r\n\
                        Content-Length: 10485760\r\n\
                        Accept-Ranges: bytes\r\n\
                        Content-Type: application/zip\r\n\
                        Content-Disposition: attachment; filename*=UTF-8''my%20archive%20%E2%9D%A4.zip\r\n\
                        Connection: close\r\n\r\n";
                    let _ = stream.write_all(resp.as_bytes());
                } else {
                    let resp = "HTTP/1.1 200 OK\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";
                    let _ = stream.write_all(resp.as_bytes());
                }
            }
            thread::sleep(Duration::from_millis(5));
        }
    });

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(3))
        .build()
        .unwrap();
    let url = Url::parse(&format!("http://{}/download", addr)).unwrap();
    let file_info = probe_url(&client, &url, &reqwest::header::HeaderMap::new()).await.unwrap();

    assert_eq!(file_info.content_length, Some(10485760));
    assert!(file_info.accepts_ranges);
    assert_eq!(file_info.suggested_filename, "my archive ❤.zip");

    stop.store(true, Ordering::Relaxed);
    let _ = server_handle.join();
}

#[tokio::test]
async fn test_generic_probe_405_fallback_to_range_get() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("Failed to bind listener");
    let addr = listener.local_addr().unwrap();
    let stop = Arc::new(AtomicBool::new(false));
    let stop_clone = stop.clone();

    let server_handle = thread::spawn(move || {
        listener.set_nonblocking(true).unwrap();
        while !stop_clone.load(Ordering::Relaxed) {
            if let Ok((mut stream, _)) = listener.accept() {
                let mut buf = [0u8; 1024];
                let _ = stream.read(&mut buf);
                let req_str = String::from_utf8_lossy(&buf);

                if req_str.starts_with("HEAD") {
                    // 405 Method Not Allowed on HEAD
                    let resp = "HTTP/1.1 405 Method Not Allowed\r\n\
                        Allow: GET\r\n\
                        Connection: close\r\n\r\n";
                    let _ = stream.write_all(resp.as_bytes());
                } else if req_str.starts_with("GET") {
                    // Range GET bytes=0-0 returns 206 Partial Content
                    let resp = "HTTP/1.1 206 Partial Content\r\n\
                        Content-Range: bytes 0-0/5242880\r\n\
                        Content-Length: 1\r\n\
                        Content-Type: application/octet-stream\r\n\
                        Content-Disposition: attachment; filename=\"fallback_payload.bin\"\r\n\
                        Connection: close\r\n\r\nX";
                    let _ = stream.write_all(resp.as_bytes());
                }
            }
            thread::sleep(Duration::from_millis(5));
        }
    });

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(3))
        .build()
        .unwrap();
    let url = Url::parse(&format!("http://{}/resource", addr)).unwrap();
    let file_info = probe_url(&client, &url, &reqwest::header::HeaderMap::new()).await.unwrap();

    assert_eq!(file_info.content_length, Some(5242880));
    assert!(file_info.accepts_ranges);
    assert_eq!(file_info.suggested_filename, "fallback_payload.bin");

    stop.store(true, Ordering::Relaxed);
    let _ = server_handle.join();
}

#[tokio::test]
async fn test_generic_probe_chunked_unknown_size() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("Failed to bind listener");
    let addr = listener.local_addr().unwrap();
    let stop = Arc::new(AtomicBool::new(false));
    let stop_clone = stop.clone();

    let server_handle = thread::spawn(move || {
        listener.set_nonblocking(true).unwrap();
        while !stop_clone.load(Ordering::Relaxed) {
            if let Ok((mut stream, _)) = listener.accept() {
                let mut buf = [0u8; 1024];
                let _ = stream.read(&mut buf);
                let req_str = String::from_utf8_lossy(&buf);

                if req_str.starts_with("HEAD") {
                    // Chunked, no Content-Length
                    let resp = "HTTP/1.1 200 OK\r\n\
                        Transfer-Encoding: chunked\r\n\
                        Content-Type: application/pdf\r\n\
                        Connection: close\r\n\r\n";
                    let _ = stream.write_all(resp.as_bytes());
                } else {
                    let resp = "HTTP/1.1 200 OK\r\nConnection: close\r\n\r\n";
                    let _ = stream.write_all(resp.as_bytes());
                }
            }
            thread::sleep(Duration::from_millis(5));
        }
    });

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(3))
        .build()
        .unwrap();
    let url = Url::parse(&format!("http://{}/document.pdf", addr)).unwrap();
    let file_info = probe_url(&client, &url, &reqwest::header::HeaderMap::new()).await.unwrap();

    assert_eq!(file_info.content_length, None, "Chunked response should have None content_length");
    assert_eq!(file_info.suggested_filename, "document.pdf");

    stop.store(true, Ordering::Relaxed);
    let _ = server_handle.join();
}

#[tokio::test]
async fn test_generic_probe_redirect_chain() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("Failed to bind listener");
    let addr = listener.local_addr().unwrap();
    let stop = Arc::new(AtomicBool::new(false));
    let stop_clone = stop.clone();

    let server_handle = thread::spawn(move || {
        listener.set_nonblocking(true).unwrap();
        while !stop_clone.load(Ordering::Relaxed) {
            if let Ok((mut stream, _)) = listener.accept() {
                let mut buf = [0u8; 1024];
                let _ = stream.read(&mut buf);
                let req_str = String::from_utf8_lossy(&buf);

                if req_str.contains("/hop1") {
                    let resp = format!(
                        "HTTP/1.1 302 Found\r\nLocation: http://{}/hop2\r\nConnection: close\r\n\r\n",
                        addr
                    );
                    let _ = stream.write_all(resp.as_bytes());
                } else if req_str.contains("/hop2") {
                    let resp = format!(
                        "HTTP/1.1 301 Moved Permanently\r\nLocation: http://{}/final_file.iso\r\nConnection: close\r\n\r\n",
                        addr
                    );
                    let _ = stream.write_all(resp.as_bytes());
                } else if req_str.contains("/final_file.iso") {
                    let resp = "HTTP/1.1 200 OK\r\n\
                        Content-Length: 4294967296\r\n\
                        Content-Type: application/x-iso9660-image\r\n\
                        Connection: close\r\n\r\n";
                    let _ = stream.write_all(resp.as_bytes());
                }
            }
            thread::sleep(Duration::from_millis(5));
        }
    });

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(3))
        .redirect(reqwest::redirect::Policy::limited(10))
        .build()
        .unwrap();
    let url = Url::parse(&format!("http://{}/hop1", addr)).unwrap();
    let file_info = probe_url(&client, &url, &reqwest::header::HeaderMap::new()).await.unwrap();

    assert_eq!(file_info.content_length, Some(4294967296));
    assert_eq!(file_info.suggested_filename, "final_file.iso");

    stop.store(true, Ordering::Relaxed);
    let _ = server_handle.join();
}

#[tokio::test]
async fn test_download_window_state_generic_probe_integration() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("Failed to bind listener");
    let addr = listener.local_addr().unwrap();
    let stop = Arc::new(AtomicBool::new(false));
    let stop_clone = stop.clone();

    let server_handle = thread::spawn(move || {
        listener.set_nonblocking(true).unwrap();
        while !stop_clone.load(Ordering::Relaxed) {
            if let Ok((mut stream, _)) = listener.accept() {
                let mut buf = [0u8; 1024];
                let _ = stream.read(&mut buf);
                let resp = "HTTP/1.1 200 OK\r\n\
                    Content-Length: 8388608\r\n\
                    Content-Type: application/zip\r\n\
                    Content-Disposition: attachment; filename=\"auto_probed_archive.zip\"\r\n\
                    Connection: close\r\n\r\n";
                let _ = stream.write_all(resp.as_bytes());
            }
            thread::sleep(Duration::from_millis(5));
        }
    });

    let url = format!("http://{}/download", addr);
    let mut wnd = DownloadWindowState::new(
        url.clone(),
        "download.bin".to_string(),
        "General".to_string(),
        "C:\\Downloads".to_string(),
    );

    // Initial state: probing started
    assert_eq!(wnd.stage, DownloadWindowStage::Info);
    assert_eq!(wnd.info_status, InfoLookupStatus::ReadingInfo);
    assert!(wnd.probe_rx.is_some(), "probe_rx must be spawned for generic HTTP");

    // Wait for background probe to complete (at most 2 seconds)
    let start = std::time::Instant::now();
    while wnd.probe_rx.is_some() && start.elapsed() < Duration::from_secs(2) {
        wnd.poll_info_rx();
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    assert_eq!(wnd.info_status, InfoLookupStatus::Success);
    assert_eq!(wnd.file_size, Some(8388608));
    assert_eq!(wnd.filename, "auto_probed_archive.zip");

    stop.store(true, Ordering::Relaxed);
    let _ = server_handle.join();
}
