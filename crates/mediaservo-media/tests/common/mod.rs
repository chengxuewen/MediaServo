//! Shared test utilities for integration tests.

use mediaservo_media::base::buffer::I420Buffer;
use mediaservo_media::base::frame::BoxVideoFrame;
use mediaservo_media::base::rotation::VideoRotation;

/// Create a known test frame: 64×64 I420 with a colored square in the center.
pub fn create_test_frame(width: u32, height: u32) -> BoxVideoFrame {
    let mut buf = I420Buffer::new(width, height);
    // Fill the center 16×16 square with a known color
    let cx = width / 2;
    let cy = height / 2;
    let half = 8u32;
    // Y plane: bright white center
    let y_plane = &mut buf.data_y;
    let u_plane = &mut buf.data_u;
    let v_plane = &mut buf.data_v;
    let stride_y = buf.stride_y as usize;
    let stride_u = buf.stride_u as usize;
    for y in (cy - half)..(cy + half) {
        for x in (cx - half)..(cx + half) {
            let yi = y as usize * stride_y + x as usize;
            if yi < y_plane.len() {
                y_plane[yi] = 235;
            }
        }
    }
    // U plane: blue-ish (low U, high V would be red but U/V are half-res)
    for y in (cy / 2 - half / 2)..(cy / 2 + half / 2) {
        for x in (cx / 2 - half / 2)..(cx / 2 + half / 2) {
            let ui = y as usize * stride_u + x as usize;
            if ui < u_plane.len() {
                u_plane[ui] = 100;
            }
        }
    }
    // V plane: blue-ish
    for y in (cy / 2 - half / 2)..(cy / 2 + half / 2) {
        for x in (cx / 2 - half / 2)..(cx / 2 + half / 2) {
            let vi = y as usize * stride_u + x as usize;
            if vi < v_plane.len() {
                v_plane[vi] = 180;
            }
        }
    }

    mediaservo_media::base::frame::VideoFrame::new(
        Box::new(buf) as Box<dyn mediaservo_media::base::buffer::VideoBuffer>
    )
    .with_rotation(VideoRotation::Rotation0)
    .with_timestamp(0)
}
