//! Deck record domain end-to-end test: CameraSource → Recorder → MP4 file.
//!
//! Requires the `backend-ffmpeg` feature (`cargo test -p mediaservo-deck --features backend-ffmpeg`).

#![cfg(feature = "backend-ffmpeg")]

use std::time::Duration;

use mediaservo_deck::record::{Container, RecordOptions, Recorder, VideoCodec};
use mediaservo_deck::source::{CameraSource, CaptureOptions, DeviceId};

#[tokio::test]
async fn records_mp4_from_camera_source() {
    let dir = std::env::temp_dir().join("deck-record-test");
    std::fs::create_dir_all(&dir).expect("create tmp dir");
    let path = dir.join("out.mp4");
    let _ = std::fs::remove_file(&path);

    let mut cam =
        CameraSource::open(DeviceId("stub:test-camera".into()), CaptureOptions::default())
            .expect("open camera");
    let opts = CaptureOptions { resolution: Some((320, 240)), framerate: Some(30), format: None };
    let mut frames = cam.start(&opts).expect("start camera");

    let mut recorder = Recorder::new(
        &path,
        RecordOptions {
            codec: VideoCodec::H264,
            container: Container::Mp4,
            fps: 30,
            keyframe_interval: 30,
        },
    )
    .unwrap_or_else(|e| panic!("create recorder: {e}"));
    let stop = recorder.stop_signal();

    // Async recording task: consume frames until stop() → flush + trailer
    let record_task = tokio::spawn(async move {
        recorder.record(&mut frames).await.expect("record ok");
    });

    // Feed ~2s of frames, then stop
    tokio::time::sleep(Duration::from_millis(1500)).await;
    cam.stop();
    stop.stop();

    // Wait for the recording task to finish (worker exits after flush + trailer)
    tokio::time::timeout(Duration::from_secs(10), record_task)
        .await
        .expect("record task finishes (no timeout)")
        .expect("record task panicked");

    // File should exist and be non-empty (MP4 header + several frames)
    let size = std::fs::metadata(&path).expect("file exists").len();
    assert!(size > 1_000, "mp4 size should be > 1KB, got {size} bytes");

    // With DECK_KEEP=1 keep the file for external verification (e.g. ffprobe)
    if std::env::var("DECK_KEEP").is_err() {
        let _ = std::fs::remove_file(&path);
    } else {
        eprintln!("kept: {path:?}");
    }
}

#[test]
fn recorder_rejects_missing_parent_dir() {
    match Recorder::new("/nonexistent-dir-xyz/out.mp4", RecordOptions::default()) {
        Err(mediaservo_deck::DeckError::NotFound(_)) => {}
        Err(e) => panic!("expected NotFound, got {e}"),
        Ok(_) => panic!("missing parent should fail"),
    }
}
