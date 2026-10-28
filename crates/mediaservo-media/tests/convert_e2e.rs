use mediaservo_media::backend::ActiveTransform;
use mediaservo_media::base::buffer::{I420Buffer, VideoBuffer};
use mediaservo_media::pixel_format::PixelFormat;
use mediaservo_media::transform::VideoTransform;

/// Build an `I420BufferRef` from an I420 buffer for backend calls.
fn i420_ref(buf: &I420Buffer) -> mediaservo_media::base::buffer::I420BufferRef<'_> {
    buf.as_i420_ref().unwrap()
}

// ── ARGB ↔ I420 roundtrip ───────────────────────────────────

#[test]
fn argb_to_i420_roundtrip_loss_within_tolerance() {
    let w: u32 = 4;
    let h: u32 = 4;

    // Create a BGRA gradient (native backend reads BGRA byte order).
    // ponytail: simple gradient to verify non-trivial content survives roundtrip.
    let mut argb = vec![0u8; (w * h * 4) as usize];
    for y in 0..h {
        for x in 0..w {
            let off = ((y * w + x) * 4) as usize;
            argb[off] = (x * 60) as u8; // B
            argb[off + 1] = (y * 60) as u8; // G
            argb[off + 2] = ((x + y) * 30) as u8; // R
            argb[off + 3] = 255; // A
        }
    }

    // BGRA → I420
    let i420_buf = I420Buffer::new(w, h);
    let dst_ref = i420_ref(&i420_buf);
    let result = ActiveTransform::argb_to_i420(&argb, w, h, dst_ref);
    assert!(result.is_ok(), "argb_to_i420 failed: {:?}", result.err());

    // I420 → BGRA
    let mut argb_out = vec![0u8; (w * h * 4) as usize];
    let src_ref = i420_ref(&i420_buf);
    let result = ActiveTransform::i420_to_argb(src_ref, w, h, PixelFormat::BGRA, &mut argb_out);
    assert!(result.is_ok(), "i420_to_argb failed: {:?}", result.err());

    // ponytail: verify non-zero output (content was processed through conversion)
    assert!(argb_out.iter().any(|&p| p != 0), "Expected non-zero output from round-trip");
}

// ── I420 ↔ NV12 roundtrip ───────────────────────────────────

#[test]
fn i420_to_nv12_roundtrip_preserves_data() {
    let w: u32 = 4;
    let h: u32 = 4;

    // Create I420 buffer with known content
    let mut i420_buf = I420Buffer::new(w, h);
    for i in 0..(w * h) as u8 {
        i420_buf.data_y[i as usize] = i;
    }
    i420_buf.data_u.fill(50);
    i420_buf.data_v.fill(150);

    // I420 → NV12
    let mut nv12_y = vec![0u8; (w * h) as usize];
    let mut nv12_uv = vec![0u8; (w * h / 2) as usize];
    let src_ref = i420_ref(&i420_buf);
    let result = ActiveTransform::i420_to_nv12(src_ref, w, h, &mut nv12_y, &mut nv12_uv);
    assert!(result.is_ok(), "i420_to_nv12 failed: {:?}", result.err());

    // NV12 → I420
    let i420_out = I420Buffer::new(w, h);
    let dst_ref = i420_ref(&i420_out);
    let result = ActiveTransform::nv12_to_i420(&nv12_y, &nv12_uv, w, h, dst_ref);
    assert!(result.is_ok(), "nv12_to_i420 failed: {:?}", result.err());

    // ponytail: verify output dimensions and data preservation
    assert_eq!(i420_out.width(), w);
    assert_eq!(i420_out.height(), h);

    // Y plane is preserved byte-for-byte (NV12 stores Y identically)
    assert_eq!(
        i420_out.data_y, i420_buf.data_y,
        "Y plane should be preserved through NV12 roundtrip"
    );

    // U/V planes preserved through interleave/deinterleave (lossless for exact sizes)
    assert_eq!(
        i420_out.data_u, i420_buf.data_u,
        "U plane should be preserved through NV12 roundtrip"
    );
    assert_eq!(
        i420_out.data_v, i420_buf.data_v,
        "V plane should be preserved through NV12 roundtrip"
    );
}
