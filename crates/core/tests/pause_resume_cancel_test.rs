use std::sync::Arc;
use std::time::Instant;
use tokio::sync::mpsc;
use rustget_core::{db::Database, queue::QueueManager, downloader::DownloadOptions};

#[tokio::test]
async fn test_pause_resume_cancel_timings() {
    let db = Database::open_in_memory().unwrap();
    let (tx, mut rx) = mpsc::channel(64);
    tokio::spawn(async move { while rx.recv().await.is_some() {} });

    let queue = Arc::new(QueueManager::new(db, 4, 0, tx));

    let opt = DownloadOptions {
        url: "https://example.com/rickroll_1080p.mp4".to_string(),
        ..Default::default()
    };

    let id = queue.add_download(opt, None, 0).await.unwrap();

    // 1. Measure Pause timing
    let t0 = Instant::now();
    queue.pause(&id).await.unwrap();
    let pause_duration = t0.elapsed();

    let paused_rec = queue.db().get_download(&id).await.unwrap().unwrap();
    assert_eq!(paused_rec.status, "paused");

    // 2. Measure Resume timing
    let t1 = Instant::now();
    queue.resume(&id).await.unwrap();
    let resume_duration = t1.elapsed();

    let resumed_rec = queue.db().get_download(&id).await.unwrap().unwrap();
    assert_eq!(resumed_rec.status, "queued");

    // 3. Measure Cancel / Remove timing
    let t2 = Instant::now();
    queue.remove(&id, true).await.unwrap();
    let cancel_duration = t2.elapsed();

    let deleted_rec = queue.db().get_download(&id).await.unwrap();
    assert!(deleted_rec.is_none());

    println!("\n=== CosmoDownloader Rust Core Queue Timings ===");
    println!("⏱️ Queue Pause Time:  {:?} ({:.3} ms)", pause_duration, pause_duration.as_secs_f64() * 1000.0);
    println!("⏱️ Queue Resume Time: {:?} ({:.3} ms)", resume_duration, resume_duration.as_secs_f64() * 1000.0);
    println!("⏱️ Queue Cancel Time: {:?} ({:.3} ms)", cancel_duration, cancel_duration.as_secs_f64() * 1000.0);
    println!("==============================================\n");
}

#[tokio::test]
async fn test_download_3_times_and_cancel() {
    let db = Database::open_in_memory().unwrap();
    let (tx, mut rx) = mpsc::channel(64);
    tokio::spawn(async move { while rx.recv().await.is_some() {} });

    let queue = Arc::new(QueueManager::new(db, 4, 0, tx));

    let urls = [
        "https://example.com/rickroll_1080p.mp4",
        "https://example.com/rickroll_720p.mp4",
        "https://example.com/rickroll_audio.mp3",
    ];

    println!("\n=== 3x Download and Cancel Verification (Rust Core) ===");
    for (i, url) in urls.iter().enumerate() {
        let opt = DownloadOptions {
            url: url.to_string(),
            ..Default::default()
        };

        // Add download
        let t_add = Instant::now();
        let id = queue.add_download(opt, None, 0).await.unwrap();
        let add_dur = t_add.elapsed();

        // Verify it was registered
        let rec = queue.db().get_download(&id).await.unwrap().expect("Record must exist");
        assert_eq!(rec.status, "queued");

        // Cancel and remove download
        let t_cancel = Instant::now();
        queue.remove(&id, true).await.unwrap();
        let cancel_dur = t_cancel.elapsed();

        // Verify completely deleted and cancelled
        let rec_after = queue.db().get_download(&id).await.unwrap();
        assert!(rec_after.is_none(), "Cancelled record must be deleted");

        println!(
            "📥 [{}/3] Download '{}': Added in {:.3} ms -> Cancelled in {:.3} ms ({:?})",
            i + 1,
            url,
            add_dur.as_secs_f64() * 1000.0,
            cancel_dur.as_secs_f64() * 1000.0,
            cancel_dur
        );
    }
    println!("✅ All 3 downloads initiated and cancelled successfully with zero leakage!\n");
}
