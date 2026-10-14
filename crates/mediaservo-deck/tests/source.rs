//! Deck capture domain (source) tests.

use mediaservo_deck::DeckError;
use mediaservo_deck::source::DeviceId;
use mediaservo_deck::source::{CameraSource, CaptureOptions, MediaDeviceKind, MediaDevices};

fn open_cam() -> CameraSource {
    CameraSource::open(DeviceId("stub:test-camera".into()), CaptureOptions::default())
        .unwrap_or_else(|e| panic!("open failed: {e}"))
}

#[test]
fn enumerate_returns_stub_camera() {
    let cams = MediaDevices::enumerate(MediaDeviceKind::Camera);
    assert_eq!(cams, vec![DeviceId("stub:test-camera".into())]);
    assert!(MediaDevices::enumerate(MediaDeviceKind::Audio).is_empty());
    assert!(MediaDevices::enumerate(MediaDeviceKind::Screen).is_empty());
}

#[test]
fn open_unknown_device_errors() {
    match CameraSource::open(DeviceId("nope".into()), CaptureOptions::default()) {
        Err(DeckError::NotFound(_)) => {}
        Err(e) => panic!("expected NotFound, got {e}"),
        Ok(_) => panic!("unknown device should fail"),
    }
}

#[test]
fn open_known_device_ok() {
    match CameraSource::open(DeviceId("stub:test-camera".into()), CaptureOptions::default()) {
        Ok(_) => {}
        Err(e) => panic!("known device should open, got {e}"),
    }
}

#[tokio::test]
async fn start_produces_i420_frames() {
    let mut src = open_cam();
    let opts = CaptureOptions { resolution: Some((640, 360)), framerate: Some(30), format: None };
    let mut frames = src.start(&opts).unwrap_or_else(|e| panic!("start failed: {e}"));

    // Sample a few frames: expect an I420 3-plane frame with matching dimensions
    let first = tokio::time::timeout(std::time::Duration::from_secs(2), frames.recv())
        .await
        .expect("first frame arrives (no timeout)")
        .expect("stream not closed");
    assert_eq!(first.format.width, 640);
    assert_eq!(first.format.height, 360);
    assert_eq!(first.planes.len(), 3);
    assert_eq!(first.planes[0].stride, 640);
    assert_eq!(first.planes[1].stride, 320);

    // One more frame to verify continuous frame production
    let second = tokio::time::timeout(std::time::Duration::from_secs(2), frames.recv())
        .await
        .expect("second frame arrives")
        .expect("stream not closed");
    assert_eq!(second.format.width, 640);

    src.stop();
}

#[tokio::test]
async fn start_twice_errors() {
    let mut src = open_cam();
    let opts = CaptureOptions { resolution: Some((320, 240)), framerate: Some(15), format: None };
    src.start(&opts).unwrap_or_else(|e| panic!("first start failed: {e}"));
    match src.start(&opts) {
        Err(DeckError::InvalidState(_)) => {}
        Err(e) => panic!("expected InvalidState, got {e}"),
        Ok(_) => panic!("second start should fail"),
    }
    src.stop();
}
