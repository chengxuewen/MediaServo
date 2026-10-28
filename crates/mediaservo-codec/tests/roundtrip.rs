#![cfg(feature = "backend-ffmpeg")]
//! Roundtrip tests — I420 → H.264 encode → I420 decode.

use mediaservo_codec::codec::{CodecId, PixelFormat, VideoFormat};
use mediaservo_codec::config::{Bitrate, DecoderConfig, EncoderConfig, EncoderPreset};
use mediaservo_codec::decoder::VideoDecoder;
use mediaservo_codec::encoder::VideoEncoder;
use mediaservo_codec::factory::CodecFactory;
use mediaservo_codec::frame::{Plane, VideoFrame};

const W: u32 = 320;
const H: u32 = 256; // multiple of 32, avoids FFmpeg alignment padding

fn test_config() -> EncoderConfig {
    let fmt = VideoFormat { width: W, height: H, pixel_format: PixelFormat::Yuv420p };
    EncoderConfig::builder(CodecId::H264, fmt)
        .bitrate(Bitrate::Cbr(8000))
        .fps(30, 1)
        .preset(EncoderPreset::P3VeryFast)
        .gop(30)
        .build()
}

fn make_gradient_frame() -> VideoFrame {
    let w = W as usize;
    let h = H as usize;
    let y_size = w * h;
    let uv_size = (w / 2) * (h / 2);
    let mut y = vec![0u8; y_size];
    for row in 0..h {
        let val = (row * 256 / h) as u8;
        for col in 0..w {
            y[row * w + col] = val;
        }
    }
    VideoFrame {
        format: VideoFormat { width: W, height: H, pixel_format: PixelFormat::Yuv420p },
        planes: vec![
            Plane { data: y, stride: W },
            Plane { data: vec![128u8; uv_size], stride: W / 2 },
            Plane { data: vec![128u8; uv_size], stride: W / 2 },
        ],
        pts: 0,
        keyframe: true,
    }
}

fn encode_one(encoder: &mut Box<dyn VideoEncoder>, frame: &VideoFrame) -> Vec<Vec<u8>> {
    encoder.push_frame(frame).unwrap();
    let mut out = Vec::new();
    while let Some(pkt) = encoder.pull_packet().unwrap() {
        if !pkt.data.is_empty() {
            out.push(pkt.data);
        }
    }
    out
}

fn decode_all(decoder: &mut Box<dyn VideoDecoder>, packets: &[Vec<u8>]) -> Vec<VideoFrame> {
    for p in packets {
        decoder.push_packet(p).unwrap();
    }
    decoder.flush().unwrap();
    let mut out = Vec::new();
    while let Some(f) = decoder.pull_frame().unwrap() {
        out.push(f);
    }
    out
}

fn psnr(a: &[u8], b: &[u8], a_stride: u32, b_stride: u32, w: u32, h: u32) -> f64 {
    let a_stride = a_stride as usize;
    let b_stride = b_stride as usize;
    let w = w as usize;
    let h = h as usize;
    let mut mse = 0.0_f64;
    let mut count = 0u64;
    for row in 0..h {
        let ao = row * a_stride;
        let bo = row * b_stride;
        for col in 0..w {
            let av = a[ao + col] as f64;
            let bv = b[bo + col] as f64;
            mse += (av - bv).powi(2);
            count += 1;
        }
    }
    if mse < 0.0001 { 100.0 } else { 10.0 * (65025.0_f64 / (mse / count as f64)).log10() }
}

fn psnr_linear(a: &[u8], b: &[u8]) -> f64 {
    let n = a.len().min(b.len());
    let mse = a[..n].iter().zip(&b[..n]).map(|(x, y)| (*x as f64 - *y as f64).powi(2)).sum::<f64>()
        / n as f64;
    if mse < 0.0001 { 100.0 } else { 10.0 * (65025.0 / mse).log10() }
}

#[test]
fn roundtrip_works() {
    let factory = CodecFactory::new();
    let cfg = test_config();
    let mut encoder = factory.create_encoder(cfg.clone(), None).unwrap();
    encoder.configure(&cfg).unwrap();
    let mut decoder = factory.create_decoder(DecoderConfig { codec: CodecId::H264 }, None).unwrap();
    decoder.configure(&DecoderConfig { codec: CodecId::H264 }).unwrap();

    let frame = make_gradient_frame();
    let encoded = encode_one(&mut encoder, &frame);
    assert!(!encoded.is_empty(), "no packets");

    let decoded = decode_all(&mut decoder, &encoded);
    assert!(!decoded.is_empty(), "no frames");
    assert!(decoded[0].width() >= W && decoded[0].height() >= H, "size mismatch");
}

#[test]
fn roundtrip_psnr_above_35db() {
    let factory = CodecFactory::new();
    let cfg = test_config();
    let mut encoder = factory.create_encoder(cfg.clone(), None).unwrap();
    encoder.configure(&cfg).unwrap();
    let mut decoder = factory.create_decoder(DecoderConfig { codec: CodecId::H264 }, None).unwrap();
    decoder.configure(&DecoderConfig { codec: CodecId::H264 }).unwrap();

    let frame = make_gradient_frame();
    let orig_y = frame.plane_data(0).unwrap().to_vec();
    let encoded = encode_one(&mut encoder, &frame);
    let decoded = decode_all(&mut decoder, &encoded);

    let dec_y = decoded[0].plane_data(0).unwrap();
    let df = &decoded[0];
    let orig_stride = W;
    let dec_stride = df.plane_stride(0).unwrap_or(df.width()); // fallback to width

    let p = psnr(&orig_y, dec_y, orig_stride, dec_stride, W, H);
    eprintln!("decoded Y plane: {} bytes, stride={dec_stride}, w={W} h={H}", dec_y.len());
    eprintln!("first 10 orig: {:?}", &orig_y[..10.min(orig_y.len())]);
    eprintln!("first 10 dec:  {:?}", &dec_y[..10.min(dec_y.len())]);
    eprintln!("PSNR (strided): {p:.1} dB ({}x{})", df.width(), df.height());
    assert!(p > 35.0, "PSNR {p:.1} below 35 dB");
}
