use rustget_core::youtube::query_video_formats;

#[tokio::test]
async fn test_shown_vs_actual_sizes_3_videos() {
    let videos = [
        ("jpurmr0Sovs", "Short 720p HD Upload Test", 1_327_704u64),
        ("C0DPdy98e4c", "TEST VIDEO", 265_198u64),
        ("dQw4w9WgXcQ", "Rick Astley - Never Gonna Give You Up (Official Video) (4K Remaster)", 243_743_176u64),
    ];

    for (vid, _title, actual_bytes) in videos {
        let url = format!("https://www.youtube.com/watch?v={}", vid);
        let formats = query_video_formats(&url).await.expect("Failed to query formats");
        println!("\n=== Video: {} ({}) ===", formats.title, vid);
        println!("is_live: {}", formats.is_live);
        println!("Qualities found ({}):", formats.qualities.len());
        for q in &formats.qualities {
            println!("  - {}: size={:?} formatted={}", q.label, q.size, q.size_formatted);
        }
        if let Some(ref a) = formats.audio {
            println!("Audio: size={:?} formatted={}", a.size, a.size_formatted);
        }
        if let Some(ref b) = formats.best {
            println!("Best/Auto: label='{}' size={:?} formatted={}", b.label, b.size, b.size_formatted);

            if let Some(shown_bytes) = b.size {
                let diff = if actual_bytes > shown_bytes {
                    actual_bytes - shown_bytes
                } else {
                    shown_bytes - actual_bytes
                };
                let pct = (diff as f64 / actual_bytes as f64) * 100.0;
                println!("Comparison: shown={} bytes, actual={} bytes, diff={:.2}%", shown_bytes, actual_bytes, pct);
                if b.is_approx {
                    assert!(pct <= 10.0, "Estimate must be within 10%, was {:.2}%", pct);
                } else {
                    assert!(pct <= 1.0, "Exact size must be within 1%, was {:.2}%", pct);
                }
            }
        }
    }
}
