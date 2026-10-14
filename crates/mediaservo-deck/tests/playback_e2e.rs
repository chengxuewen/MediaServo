//! Deck playback domain test: recorded artifact → Player demux/decode.
//!
//! Requires the `backend-ffmpeg` feature.

#![cfg(feature = "backend-ffmpeg")]

use std::time::Duration;

use mediaservo_deck::playback::Player;
use mediaservo_deck::record::{Container, RecordOptions, Recorder, VideoCodec};
use mediaservo_deck::source::{CameraSource, CaptureOptions, DeviceId};

/// Record an MP4, then play it back and verify decoded frame count/dimensions.
#[tokio::test]
async fn records_then_plays_back() {
    let dir = std::env::temp_dir().join("deck-playback-test");
    std::fs::create_dir_all(&dir).expect("mkdir");
    let path = dir.join("rec.mp4");
    let _ = std::fs::remove_file(&path);

    // ── Record ~1s ──────────────────────────────────
    let mut cam =
        CameraSource::open(DeviceId("stub:test-camera".into()), CaptureOptions::default())
            .expect("open cam");
    let opts = CaptureOptions { resolution: Some((320, 240)), framerate: Some(30), format: None };
    let mut cam_frames = cam.start(&opts).expect("start cam");

    let mut recorder = Recorder::new(
        &path,
        RecordOptions {
            codec: VideoCodec::H264,
            container: Container::Mp4,
            fps: 30,
            keyframe_interval: 30,
        },
    )
    .unwrap_or_else(|e| panic!("recorder: {e}"));
    let stop = recorder.stop_signal();
    let rec_task = tokio::spawn(async move {
        recorder.record(&mut cam_frames).await.expect("record ok");
    });

    tokio::time::sleep(Duration::from_millis(1200)).await;
    cam.stop();
    stop.stop();
    tokio::time::timeout(Duration::from_secs(10), rec_task)
        .await
        .expect("record finishes")
        .expect("record ok");
    drop(cam);

    // ── Playback ────────────────────────────────────
    let mut player = Player::open(&path).unwrap_or_else(|e| panic!("open: {e}"));
    let dur = player.duration_secs().expect("duration");
    assert!((0.5..=5.0).contains(&dur), "duration {dur}s should be ~1s");

    let mut count = 0u32;
    let mut last_w = 0;
    let mut last_h = 0;
    while let Some(frame) = player.next_frame().expect("decode frame") {
        if count == 0 {
            last_w = frame.format.width;
            last_h = frame.format.height;
            assert_eq!(last_w, 320);
            assert_eq!(last_h, 240);
        }
        assert_eq!(frame.format.width, last_w);
        assert_eq!(frame.format.height, last_h);
        count += 1;
        if count > 120 {
            break; // Safety guard: 120-frame cap
        }
    }
    assert!(count >= 15, "expected >=15 decoded frames, got {count}");
    eprintln!("decoded {count} frames @{last_w}x{last_h}");

    if std::env::var("DECK_KEEP").is_err() {
        let _ = std::fs::remove_file(&path);
    } else {
        eprintln!("kept: {path:?}");
    }
}

#[test]
fn open_missing_file_errors() {
    match Player::open("/nonexistent-deck-file.mp4") {
        Err(mediaservo_deck::DeckError::NotFound(_)) => {}
        Err(e) => panic!("expected NotFound, got {e}"),
        Ok(_) => panic!("missing file should fail"),
    }
}
