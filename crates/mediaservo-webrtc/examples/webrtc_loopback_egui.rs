//! P2P video loopback demo with egui visualization.
//!
//! Creates two PeerConnections, exchanges SDP in-process,
//! sends test-pattern frames, displays both sender and receiver previews,
//! and shows FPS stats.
//!
//! Run with:
//! ```bash
//! cargo run -p mediaservo-webrtc --example webrtc_loopback_egui
//! ```
//!
//! Use stub backend (no native libwebrtc): `--no-default-features`

use eframe::egui;
use egui::ColorImage;
use mediaservo_webrtc::TrackWriteBackend;
use mediaservo_webrtc::factory::RTCPeerConnectionFactory;
use mediaservo_webrtc::peer_connection::*;
use mediaservo_webrtc::track::{TrackKind, TrackRef};
use mediaservo_webrtc::traits::PeerConnectionApi;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

// ── Constants ──

const W: u32 = 640;
const H: u32 = 360;

// ── P2P State ──

#[derive(Debug, Clone, Copy, PartialEq)]
enum P2pState {
    Idle,
    Connecting,
    Connected,
    Error,
}

// ── Shared Pipeline ──

struct Pipeline {
    sender_frame: Mutex<Option<(Vec<u8>, u32, u32)>>,
    receiver_frame: Mutex<Option<(Vec<u8>, u32, u32)>>,
    sender_count: AtomicU64,
    receiver_count: AtomicU64,
    status: Mutex<String>,
    p2p_state: Mutex<P2pState>,
    stop_requested: AtomicBool,
}

// ── Entry ──

fn main() -> Result<(), eframe::Error> {
    let pipeline = Arc::new(Pipeline {
        sender_frame: Mutex::new(None),
        receiver_frame: Mutex::new(None),
        sender_count: AtomicU64::new(0),
        receiver_count: AtomicU64::new(0),
        status: Mutex::new("Starting P2P...".into()),
        p2p_state: Mutex::new(P2pState::Connecting),
        stop_requested: AtomicBool::new(false),
    });

    // Spawn P2P thread
    {
        let p = pipeline.clone();
        std::thread::spawn(move || run_p2p(p));
    }

    let p_clone = pipeline.clone();
    eframe::run_native(
        "mediaservo-webrtc P2P Loopback",
        eframe::NativeOptions {
            viewport: egui::ViewportBuilder::default().with_inner_size([960.0, 540.0]),
            ..Default::default()
        },
        Box::new(move |_cc| {
            Ok(Box::new(LoopbackApp { p: p_clone, sender_tex: None, receiver_tex: None }))
        }),
    )
}

// ── egui App ──

struct LoopbackApp {
    p: Arc<Pipeline>,
    sender_tex: Option<egui::TextureHandle>,
    receiver_tex: Option<egui::TextureHandle>,
}

impl eframe::App for LoopbackApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        ctx.request_repaint_after(std::time::Duration::from_millis(33));

        // Top bar: FPS stats
        egui::TopBottomPanel::top("fps_bar").show(ctx, |ui| {
            ui.horizontal(|ui| {
                let sent = self.p.sender_count.load(Ordering::Relaxed);
                let recv = self.p.receiver_count.load(Ordering::Relaxed);
                ui.label(format!("Sent: {sent} frames | Received: {recv} frames"));
                ui.separator();
                ui.label(self.p.status.lock().unwrap().as_str());
            });
        });

        // Center: video previews side by side
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.columns(2, |cols| {
                // Left: Sender preview
                cols[0].vertical_centered(|ui| {
                    ui.heading("Sender");
                    video_preview(ui, &mut self.sender_tex, &self.p.sender_frame);
                });

                // Right: Receiver preview
                cols[1].vertical_centered(|ui| {
                    ui.heading("Receiver");
                    video_preview(ui, &mut self.receiver_tex, &self.p.receiver_frame);
                });
            });
        });

        // Bottom bar: controls
        egui::TopBottomPanel::bottom("controls").show(ctx, |ui| {
            ui.horizontal(|ui| {
                let state = *self.p.p2p_state.lock().unwrap();
                ui.label(format!("P2P State: {state:?}"));
                ui.separator();
                if ui.button("Stop").clicked() {
                    self.p.stop_requested.store(true, Ordering::Relaxed);
                }
            });
        });
    }
}

fn video_preview(
    ui: &mut egui::Ui,
    tex: &mut Option<egui::TextureHandle>,
    frame_lock: &Mutex<Option<(Vec<u8>, u32, u32)>>,
) {
    if let Some((rgba, w, h)) = frame_lock.lock().unwrap().as_ref() {
        let img = ColorImage::from_rgba_unmultiplied([*w as usize, *h as usize], rgba);
        let handle = tex.get_or_insert_with(|| {
            ui.ctx().load_texture("video-preview", img.clone(), Default::default())
        });
        handle.set(img, Default::default());
        ui.image(egui::load::SizedTexture::new(handle.id(), [*w as f32, *h as f32]));
    } else {
        ui.label("No video");
    }
}

// ── P2P Thread ──

fn run_p2p(p: Arc<Pipeline>) {
    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async {
        *p.status.lock().unwrap() = "Creating PeerConnections...".into();
        let factory = RTCPeerConnectionFactory::new();
        // ponytail: use the SAME factory for both PC and video track —
        // libwebrtc requires tracks to be created by the PC's factory.

        let pc_sender =
            factory.create_peer_connection(RTCConfiguration::default()).await.expect("sender pc");
        let pc_receiver =
            factory.create_peer_connection(RTCConfiguration::default()).await.expect("receiver pc");

        // ── Create video track with real VideoTrackSource ──
        // add_track internally creates the real backend's TrackSender (webrtc-sys: VideoTrackSource + media_track)
        // and the frame write is retrieved via get_track (the T0 fix: replaces the deleted create_video_track_custom/accept_video_track)
        pc_sender.add_track("loopback-video", TrackKind::Video).expect("add track");
        let track_sender = match pc_sender.get_track("loopback-video").unwrap() {
            TrackRef::Sender(s) => s,
            _ => unreachable!("add_track registered a Sender"),
        };

        // on_track callback — fires when remote track arrives after SDP negotiation
        let log_status = p.clone();
        let (frame_tx, frame_rx) = std::sync::mpsc::channel::<(Vec<u8>, u32, u32)>();

        pc_receiver.on_track(move |receiver| {
            *log_status.status.lock().unwrap() = "Receiver: remote track connected!".into();

            // Register a FrameSink that sends decoded I420 frames via channel
            struct ChannelSink {
                tx: std::sync::mpsc::Sender<(Vec<u8>, u32, u32)>,
            }
            impl mediaservo_webrtc::track::FrameSink for ChannelSink {
                fn on_frame(&self, data: &[u8], width: u32, height: u32) {
                    self.tx.send((data.to_vec(), width, height)).ok();
                }
            }
            // register the FrameSink via receiver.track (TrackRef::Receiver)
            if let TrackRef::Receiver(tr) = &receiver.track {
                tr.set_frame_sink(Box::new(ChannelSink { tx: frame_tx.clone() }));
            }
        });

        // SDP exchange
        *p.status.lock().unwrap() = "Exchanging SDP...".into();
        let offer = pc_sender
            .create_offer(&RTCOfferOptions {
                offer_to_receive_video: false,
                offer_to_receive_audio: false,
                ..Default::default()
            })
            .await
            .expect("offer");
        pc_sender.set_local_description(&offer).await.expect("set local");
        pc_receiver.set_remote_description(&offer).await.expect("set remote");

        let answer = pc_receiver.create_answer(&RTCAnswerOptions::default()).await.expect("answer");
        pc_receiver.set_local_description(&answer).await.expect("set local");
        pc_sender.set_remote_description(&answer).await.expect("set remote");
        let s = format!("{:?}", pc_sender.connection_state());
        let r = format!("{:?}", pc_receiver.connection_state());
        *p.p2p_state.lock().unwrap() = P2pState::Connected;
        *p.status.lock().unwrap() = format!("S:{} | R:{}", s, r);

        // Spawn receiver frame decoder thread
        let p_rcv = p.clone();
        std::thread::spawn(move || {
            while let Ok((i420, w, h)) = frame_rx.recv() {
                let rgba = i420_to_rgba(&i420, w, h);
                let mut f = p_rcv.receiver_frame.lock().unwrap();
                *f = Some((rgba, w, h));
                drop(f);
                p_rcv.receiver_count.fetch_add(1, Ordering::Relaxed);
            }
        });
        // Frame generation loop
        let mut frame_idx: u64 = 0;
        let frame_interval = std::time::Duration::from_millis(33);

        while !p.stop_requested.load(Ordering::Relaxed) {
            let i420 = generate_test_frame(W, H, frame_idx);
            let rgba = i420_to_rgba(&i420, W, H);
            *p.sender_frame.lock().unwrap() = Some((rgba, W, H));
            p.sender_count.fetch_add(1, Ordering::Relaxed);

            // Write raw I420 frame to the webrtc track
            // write frames through the TrackSender's public methods (private backend types never exposed)
            let _ = track_sender.write_raw_i420(&i420, W, H).await;

            frame_idx += 1;
            tokio::time::sleep(frame_interval).await;
        }

        pc_sender.close().await;
        pc_receiver.close().await;
        *p.p2p_state.lock().unwrap() = P2pState::Idle;
        *p.status.lock().unwrap() = "Stopped".into();
    });
}

// ── Frame Utils ──

fn generate_test_frame(width: u32, height: u32, frame_index: u64) -> Vec<u8> {
    let y_size = (width * height) as usize;
    let uv_size = (width * height / 4) as usize;
    let total = y_size + 2 * uv_size;
    let mut frame = vec![0u8; total];

    let bar_width = width / 4;
    let shift = (frame_index % 100) as u32;

    // Y plane: moving color bars
    for y in 0..height {
        for x in 0..width {
            let bar = ((x + shift) / bar_width) % 4;
            let y_val: u8 = match bar {
                0 => 16,
                1 => 128,
                2 => 200,
                _ => 235,
            };
            frame[(y * width + x) as usize] = y_val;
        }
    }

    // U and V planes: gray (128)
    let uv_base = y_size;
    for i in 0..2 * uv_size {
        frame[uv_base + i] = 128;
    }

    // Diagonal line that moves
    let line_y = ((frame_index * 2) % height as u64) as usize;
    for x in 0..width {
        frame[line_y * width as usize + x as usize] = 200;
    }

    frame
}

fn i420_to_rgba(i420: &[u8], width: u32, height: u32) -> Vec<u8> {
    let y_size = (width * height) as usize;
    let mut rgba = vec![0u8; 4 * y_size];

    let u_plane = y_size;
    let v_plane = y_size + y_size / 4;

    for y in 0..height as usize {
        for x in 0..width as usize {
            let idx = y * width as usize + x;
            let yy = i420[idx] as f32;
            let u = i420[u_plane + y / 2 * width as usize / 2 + x / 2] as f32 - 128.0;
            let v = i420[v_plane + y / 2 * width as usize / 2 + x / 2] as f32 - 128.0;

            let r = (yy + 1.402 * v).clamp(0.0, 255.0) as u8;
            let g = (yy - 0.344 * u - 0.714 * v).clamp(0.0, 255.0) as u8;
            let b = (yy + 1.772 * u).clamp(0.0, 255.0) as u8;

            let rgba_idx = idx * 4;
            rgba[rgba_idx] = r;
            rgba[rgba_idx + 1] = g;
            rgba[rgba_idx + 2] = b;
            rgba[rgba_idx + 3] = 255;
        }
    }
    rgba
}
