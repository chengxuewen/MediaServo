#![cfg(feature = "backend-yuv-sys")]

// mediaservo-media viewer — VideoFrame transform demo with egui (grid + single view)
// Usage: cargo run -p mediaservo-media --example viewer --features backend-native

// Note: all code between here and fn main requires backend-native.
// The empty-crate pattern is intentional — cargo needs a main fn.

// mediaservo-media viewer — VideoFrame transform demo with egui (grid + single view)
// Usage: cargo run -p mediaservo-media --example viewer --features backend-native

use eframe::egui;
use mediaservo_media::backend::NativeTransform;
use mediaservo_media::base::buffer::{I420Buffer, VideoBuffer};
use mediaservo_media::base::rotation::VideoRotation;
use mediaservo_media::pixel_format::PixelFormat;
use mediaservo_media::transform::VideoTransform;

// ── Helpers wrapping NativeTransform ───────────────────────

fn i420_scale(src: &I420Buffer, new_w: u32, new_h: u32) -> I420Buffer {
    let src_w = src.width();
    let src_h = src.height();
    let src_ref = src.as_i420_ref().unwrap();
    let dst = I420Buffer::new(new_w, new_h);
    let dst_ref = dst.as_i420_ref().unwrap();
    NativeTransform::scale(src_ref, src_w, src_h, dst_ref, new_w, new_h).unwrap();
    dst
}

fn i420_crop(src: &I420Buffer, x: u32, y: u32, w: u32, h: u32) -> I420Buffer {
    let src_ref = src.as_i420_ref().unwrap();
    let dst = I420Buffer::new(w, h);
    let dst_ref = dst.as_i420_ref().unwrap();
    NativeTransform::crop(src_ref, x, y, w, h, dst_ref).unwrap();
    dst
}

fn i420_rotate(src: &I420Buffer, rot: VideoRotation) -> I420Buffer {
    let w = src.width();
    let h = src.height();
    let (dst_w, dst_h) = match rot {
        VideoRotation::Rotation0 | VideoRotation::Rotation180 => (w, h),
        VideoRotation::Rotation90 | VideoRotation::Rotation270 => (h, w),
    };
    let src_ref = src.as_i420_ref().unwrap();
    let dst = I420Buffer::new(dst_w, dst_h);
    let dst_ref = dst.as_i420_ref().unwrap();
    NativeTransform::rotate(src_ref, w, h, rot, dst_ref).unwrap();
    dst
}

fn i420_nv12_roundtrip(src: &I420Buffer) -> I420Buffer {
    let w = src.width();
    let h = src.height();
    let src_ref = src.as_i420_ref().unwrap();
    let mut nv12_y = vec![0u8; (w * h) as usize];
    let mut nv12_uv = vec![0u8; (w * (h / 2)) as usize];
    NativeTransform::i420_to_nv12(src_ref, w, h, &mut nv12_y, &mut nv12_uv).unwrap();
    let dst = I420Buffer::new(w, h);
    let dst_ref = dst.as_i420_ref().unwrap();
    NativeTransform::nv12_to_i420(&nv12_y, &nv12_uv, w, h, dst_ref).unwrap();
    dst
}

// ponytail: direct plane fill for test pattern, no image crate → I420 roundtrip
fn generate_test_pattern(w: u32, h: u32) -> I420Buffer {
    let mut buf = I420Buffer::new(w, h);
    let y_stride = buf.stride_y as usize;
    let u_stride = buf.stride_u as usize;
    let v_stride = buf.stride_v as usize;

    // 8 vertical color bars
    for y in 0..h as usize {
        for x in 0..w as usize {
            let stripe = (x * 8 / w as usize).min(7);
            let vals = [16u8, 50, 90, 130, 170, 200, 220, 235];
            buf.data_y[y * y_stride + x] = vals[stripe];
        }
    }

    let half_w = (w / 2) as usize;
    let half_h = (h / 2) as usize;
    for y in 0..half_h {
        for x in 0..half_w {
            buf.data_u[y * u_stride + x] = ((x as f32 / half_w as f32) * 255.0) as u8;
            buf.data_v[y * v_stride + x] = ((y as f32 / half_h as f32) * 255.0) as u8;
        }
    }
    buf
}

fn resolve_asset_path() -> std::path::PathBuf {
    let exe_dir = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.to_path_buf()))
        .unwrap_or_else(|| ".".into());
    let path = exe_dir.join("../assets/images/color_card_1920x1080.bmp");
    if path.exists() {
        return path;
    }
    concat!(env!("CARGO_MANIFEST_DIR"), "/../assets/images/color_card_1920x1080.bmp").into()
}

fn load_i420() -> I420Buffer {
    if let Ok(dynamic) = image::open(resolve_asset_path()) {
        let rgba = dynamic.to_rgba8();
        let (w, h) = rgba.dimensions();
        // native.rs argb_to_i420 reads bytes as [B, G, R, A] — swap R<->B
        let bgra: Vec<u8> = rgba.pixels().flat_map(|p| [p[2], p[1], p[0], p[3]]).collect();
        let dst = I420Buffer::new(w, h);
        let dst_ref = dst.as_i420_ref().unwrap();
        if NativeTransform::argb_to_i420(&bgra, w, h, dst_ref).is_ok() {
            return dst;
        }
    }
    // ponytail: 320×240 is enough for a grid demo
    generate_test_pattern(320, 240)
}

// ── Texture helpers ────────────────────────────────────────

#[derive(PartialEq)]
enum Page {
    Grid,
    Single,
}

struct FrameVariant {
    label: String,
    rgba: Vec<u8>,
    width: u32,
    height: u32,
    texture: Option<egui::TextureHandle>,
}

impl FrameVariant {
    fn from_i420(label: &str, i420: &I420Buffer) -> Self {
        let w = i420.width();
        let h = i420.height();
        let mut rgba = vec![0u8; (w * h * 4) as usize];
        let src_ref = i420.as_i420_ref().unwrap();
        NativeTransform::i420_to_argb(src_ref, w, h, PixelFormat::RGBA, &mut rgba).unwrap();
        Self { label: label.into(), rgba, width: w, height: h, texture: None }
    }

    fn load_texture(&mut self, ctx: &egui::Context, id: &str, thumb_w: u32) {
        let (tw, th) = thumb_dims(self.width, self.height, thumb_w);
        let thumb = downscale_rgba(&self.rgba, self.width, self.height, tw, th);
        let size = [tw as usize, th as usize];
        let img = egui::ColorImage::from_rgba_unmultiplied(size, &thumb);
        self.texture = Some(ctx.load_texture(String::from(id), img, egui::TextureOptions::LINEAR));
    }
}

/// Nearest-neighbor downscale RGBA to target dimensions.
fn downscale_rgba(src: &[u8], sw: u32, sh: u32, tw: u32, th: u32) -> Vec<u8> {
    if tw >= sw && th >= sh {
        return src.to_vec();
    }
    let tw = tw.max(1);
    let th = th.max(1);
    let mut dst = vec![0u8; (tw * th * 4) as usize];
    let xr = sw as f32 / tw as f32;
    let yr = sh as f32 / th as f32;
    for y in 0..th {
        for x in 0..tw {
            let sx = ((x as f32 * xr) as usize).min(sw as usize - 1);
            let sy = ((y as f32 * yr) as usize).min(sh as usize - 1);
            let si = (sy * sw as usize + sx) * 4;
            let di = (y as usize * tw as usize + x as usize) * 4;
            dst[di..di + 4].copy_from_slice(&src[si..si + 4]);
        }
    }
    dst
}

fn thumb_dims(w: u32, h: u32, thumb_w: u32) -> (u32, u32) {
    let max_dim = w.max(h);
    if max_dim <= thumb_w {
        (w, h)
    } else {
        let r = thumb_w as f32 / max_dim as f32;
        ((w as f32 * r) as u32, (h as f32 * r) as u32)
    }
}

// ── Single-view state ──────────────────────────────────────

struct SingleViewState {
    selected_label: String,
    selected_rgba: Vec<u8>,
    selected_w: u32,
    selected_h: u32,
    orig_i420: I420Buffer,
    texture: Option<egui::TextureHandle>,
}

// ── App ────────────────────────────────────────────────────

struct App {
    page: Page,
    variants: Vec<FrameVariant>,
    single: Option<SingleViewState>,
    thumbnail_w: u32,
    status: String,
    loaded: bool,
}

impl Default for App {
    fn default() -> Self {
        let mut app = Self {
            page: Page::Grid,
            variants: vec![],
            single: None,
            thumbnail_w: 240,
            status: String::new(),
            loaded: false,
        };
        app.build_variants();
        app
    }
}

impl App {
    fn i420_to_frame(_label: &str, i420: &I420Buffer) -> (Vec<u8>, u32, u32) {
        let w = i420.width();
        let h = i420.height();
        let mut rgba = vec![0u8; (w * h * 4) as usize];
        let src_ref = i420.as_i420_ref().unwrap();
        NativeTransform::i420_to_argb(src_ref, w, h, PixelFormat::RGBA, &mut rgba).unwrap();
        (rgba, w, h)
    }

    fn build_variants(&mut self) {
        let i420 = load_i420();
        let w = i420.width();
        let h = i420.height();

        let nv12_rt = i420_nv12_roundtrip(&i420);

        self.variants = vec![
            FrameVariant::from_i420(&format!("Original {w}×{h} I420"), &i420),
            FrameVariant::from_i420("I420→NV12→I420 round-trip", &nv12_rt),
            FrameVariant::from_i420("Scale 50%", &i420_scale(&i420, w / 2, h / 2)),
            FrameVariant::from_i420("Scale 25%", &i420_scale(&i420, w / 4, h / 4)),
            {
                let cx = ((w - 960).min(w)) / 2 & !1;
                let cy = ((h - 540).min(h)) / 2 & !1;
                let crop_w = w.min(960);
                let crop_h = h.min(540);
                FrameVariant::from_i420("Crop Center", &i420_crop(&i420, cx, cy, crop_w, crop_h))
            },
            FrameVariant::from_i420("Rotate 90°", &i420_rotate(&i420, VideoRotation::Rotation90)),
            FrameVariant::from_i420("Rotate 180°", &i420_rotate(&i420, VideoRotation::Rotation180)),
            FrameVariant::from_i420("Rotate 270°", &i420_rotate(&i420, VideoRotation::Rotation270)),
            {
                let half = i420_scale(&i420, w / 2, h / 2);
                let cx = ((half.width() - w / 4).min(half.width())) / 2 & !1;
                let cy = ((half.height() - h / 4).min(half.height())) / 2 & !1;
                let crop_w = half.width().min(w / 4);
                let crop_h = half.height().min(h / 4);
                FrameVariant::from_i420("Scale→Crop", &i420_crop(&half, cx, cy, crop_w, crop_h))
            },
            {
                let r90 = i420_rotate(&i420, VideoRotation::Rotation90);
                FrameVariant::from_i420(
                    "Rot90→Scale 50%",
                    &i420_scale(&r90, r90.width() / 2, r90.height() / 2),
                )
            },
        ];

        // Init single view with original
        let (rgba, fw, fh) = Self::i420_to_frame("Original", &i420);
        self.single = Some(SingleViewState {
            selected_label: format!("Original {w}×{h} I420"),
            selected_rgba: rgba,
            selected_w: fw,
            selected_h: fh,
            orig_i420: i420,
            texture: None,
        });

        self.loaded = true;
        self.status = format!("{} variants processed", self.variants.len());
    }

    fn apply_single(&mut self, label: &str, f: impl FnOnce(&I420Buffer) -> I420Buffer) {
        if let Some(ref s) = self.single {
            let i420 = f(&s.orig_i420);
            let (rgba, w, h) = Self::i420_to_frame(label, &i420);
            self.single = Some(SingleViewState {
                selected_label: label.into(),
                selected_rgba: rgba,
                selected_w: w,
                selected_h: h,
                orig_i420: s.orig_i420.clone(),
                texture: None,
            });
            self.status = format!("{label}: {w}×{h}");
        }
    }

    fn show_grid_page(&mut self, ctx: &egui::Context, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.label("Thumbnail:");
            for &tw in &[120u32, 180, 240, 320, 480] {
                if ui.selectable_label(self.thumbnail_w == tw, tw.to_string()).clicked() {
                    self.thumbnail_w = tw;
                    for v in &mut self.variants {
                        v.texture = None;
                    }
                }
            }
            ui.separator();
            ui.label(format!("{} variants", self.variants.len()));
        });
        ui.separator();

        let tw = self.thumbnail_w as f32;
        let avail = ui.available_width();
        let cols = (avail / (tw + 12.0)).floor().max(1.0) as usize;
        let variants_len = self.variants.len();

        egui::ScrollArea::vertical().show(ui, |ui| {
            egui::Grid::new("variant_grid")
                .striped(false)
                .min_col_width(tw)
                .spacing([6.0, 6.0])
                .show(ui, |ui| {
                    for (i, v) in self.variants.iter_mut().enumerate() {
                        if v.texture.is_none() {
                            v.load_texture(ctx, &format!("g{i}"), self.thumbnail_w);
                        }
                        ui.vertical(|ui| {
                            ui.set_width(tw);
                            if let Some(ref tex) = v.texture {
                                let scale = tw / v.width.max(v.height) as f32;
                                ui.image(egui::ImageSource::Texture(
                                    egui::load::SizedTexture::new(
                                        tex.id(),
                                        egui::Vec2::new(
                                            v.width as f32 * scale,
                                            v.height as f32 * scale,
                                        ),
                                    ),
                                ));
                            }
                            ui.label(egui::RichText::new(&v.label).size(11.0));
                            ui.label(
                                egui::RichText::new(format!("{}×{}", v.width, v.height))
                                    .size(10.0)
                                    .color(egui::Color32::GRAY),
                            );
                        });
                        if (i + 1) % cols == 0 && i + 1 < variants_len {
                            ui.end_row();
                        }
                    }
                });
        });
    }

    fn show_single_page(&mut self, ctx: &egui::Context, ui: &mut egui::Ui) {
        if let Some(ref mut s) = self.single {
            if s.texture.is_none() {
                const MAX_TEX: u32 = 1280;
                let (tw, th) = if s.selected_w > MAX_TEX || s.selected_h > MAX_TEX {
                    let m = s.selected_w.max(s.selected_h);
                    let r = MAX_TEX as f32 / m as f32;
                    ((s.selected_w as f32 * r) as u32, (s.selected_h as f32 * r) as u32)
                } else {
                    (s.selected_w, s.selected_h)
                };
                let thumb = downscale_rgba(&s.selected_rgba, s.selected_w, s.selected_h, tw, th);
                let size = [tw as usize, th as usize];
                let img = egui::ColorImage::from_rgba_unmultiplied(size, &thumb);
                s.texture = Some(ctx.load_texture(
                    String::from("single-tex"),
                    img,
                    egui::TextureOptions::LINEAR,
                ));
            }

            ui.label(format!("{}  |  {}×{}", s.selected_label, s.selected_w, s.selected_h));
            ui.separator();

            if let Some(ref tex) = s.texture {
                let available = ui.available_size();
                let scale = (available.x / s.selected_w as f32)
                    .min(available.y / s.selected_h as f32)
                    .min(1.0);
                ui.image(egui::ImageSource::Texture(egui::load::SizedTexture::new(
                    tex.id(),
                    egui::Vec2::new(s.selected_w as f32 * scale, s.selected_h as f32 * scale),
                )));
            }
        }
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        egui::TopBottomPanel::top("header").show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.heading("mediaservo-media");
                ui.separator();

                ui.selectable_value(&mut self.page, Page::Grid, "Grid View");
                ui.selectable_value(&mut self.page, Page::Single, "Single View");

                ui.separator();
                if ui.button("Reload").clicked() {
                    self.variants.clear();
                    self.single = None;
                    self.loaded = false;
                    self.build_variants();
                }
                ui.label(format!("|  {}", self.status));
            });
        });

        egui::TopBottomPanel::bottom("status").show(ctx, |ui| {
            ui.label("Pipeline: BMP/Pattern → RGBA → I420 → [Scale | Crop | Rotate | NV12] → RGBA → Display");
        });

        egui::CentralPanel::default().show(ctx, |ui| {
            if !self.loaded {
                ui.centered_and_justified(|ui| {
                    ui.spinner();
                });
                return;
            }

            match self.page {
                Page::Grid => self.show_grid_page(ctx, ui),
                Page::Single => {
                    egui::SidePanel::right("controls")
                        .resizable(false)
                        .default_width(200.0)
                        .show_inside(ui, |ui| {
                            ui.heading("Transform");
                            ui.separator();

                            if ui.button("Original").clicked() {
                                if let Some(ref s) = self.single {
                                    let (rgba, w, h) =
                                        Self::i420_to_frame("Original", &s.orig_i420);
                                    let ow = s.orig_i420.width();
                                    let oh = s.orig_i420.height();
                                    self.single = Some(SingleViewState {
                                        selected_label: format!("Original {ow}×{oh} I420"),
                                        selected_rgba: rgba,
                                        selected_w: w,
                                        selected_h: h,
                                        orig_i420: s.orig_i420.clone(),
                                        texture: None,
                                    });
                                }
                            }
                            ui.separator();

                            ui.label("Format Convert");
                            for (label, cb) in [(
                                "I420 → NV12 → I420",
                                Box::new(|i: &I420Buffer| i420_nv12_roundtrip(i))
                                    as Box<dyn Fn(&I420Buffer) -> I420Buffer>,
                            )]
                            .iter()
                            {
                                if ui.button(*label).clicked() {
                                    self.apply_single(label, |i| cb(i));
                                }
                            }
                            ui.separator();

                            ui.label("Scale");
                            if ui.button("50%").clicked() {
                                self.apply_single("Scale 50%", |i| {
                                    i420_scale(i, i.width() / 2, i.height() / 2)
                                });
                            }
                            if ui.button("25%").clicked() {
                                self.apply_single("Scale 25%", |i| {
                                    i420_scale(i, i.width() / 4, i.height() / 4)
                                });
                            }
                            ui.separator();

                            ui.label("Crop");
                            if ui.button("Center Crop").clicked() {
                                self.apply_single("Crop Center", |i| {
                                    let cw = i.width() / 2;
                                    let ch = i.height() / 2;
                                    let cx = ((i.width() - cw) / 2) & !1;
                                    let cy = ((i.height() - ch) / 2) & !1;
                                    i420_crop(i, cx, cy, cw, ch)
                                });
                            }
                            ui.separator();

                            ui.label("Rotate");
                            for &deg in &[90u32, 180, 270] {
                                let rot = match deg {
                                    90 => VideoRotation::Rotation90,
                                    180 => VideoRotation::Rotation180,
                                    270 => VideoRotation::Rotation270,
                                    _ => unreachable!(),
                                };
                                if ui.button(format!("{deg}°")).clicked() {
                                    self.apply_single(&format!("Rotate {deg}°"), |i| {
                                        i420_rotate(i, rot)
                                    });
                                }
                            }
                            ui.separator();

                            ui.label("Pipeline");
                            if ui.button("Scale→Crop").clicked() {
                                self.apply_single("Scale→Crop", |i| {
                                    let half = i420_scale(i, i.width() / 2, i.height() / 2);
                                    let cw = half.width() / 2;
                                    let ch = half.height() / 2;
                                    let cx = ((half.width() - cw) / 2) & !1;
                                    let cy = ((half.height() - ch) / 2) & !1;
                                    i420_crop(&half, cx, cy, cw, ch)
                                });
                            }
                            if ui.button("Rotate→Scale").clicked() {
                                self.apply_single("Rot→Scale", |i| {
                                    let r = i420_rotate(i, VideoRotation::Rotation90);
                                    i420_scale(&r, r.width() / 2, r.height() / 2)
                                });
                            }
                        });

                    self.show_single_page(ctx, ui);
                }
            }
        });
    }
}

fn main() -> Result<(), eframe::Error> {
    eframe::run_native(
        "mediaservo-media Video Frame Viewer",
        eframe::NativeOptions {
            viewport: egui::ViewportBuilder::default()
                .with_inner_size([1200.0, 800.0])
                .with_title("mediaservo-media Video Frame Viewer"),
            ..Default::default()
        },
        Box::new(|_cc| Ok(Box::new(App::default()))),
    )
}

// Stub main for when backend-yuv-sys is not enabled
#[cfg(not(feature = "backend-yuv-sys"))]
fn main() {
    eprintln!("viewer example requires --features backend-yuv-sys");
}
