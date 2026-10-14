// ── TimestampOverlay ────────────────────────────────────

use chrono::{LocalResult, TimeZone, Utc};

use super::fonts::TextBurner;

/// Format for the timestamp overlay text.
#[derive(Clone, Copy, Debug)]
pub enum TimestampFormat {
    /// YYYY-MM-DD HH:MM:SS.mmm (wall clock from µs timestamp).
    DateTime,
    /// Frame counter: zero-padded 6 digits.
    FrameCount,
    /// Both DateTime and FrameCount on separate lines.
    Combined,
}

/// Renders a timestamp or frame counter as a bitmap-font overlay into I420 planes.
///
/// Uses a [`TextBurner`] to render text into the Y plane, and fills corresponding
/// U/V chroma regions with 128 (no color) to avoid colorful artifacts under text.
pub struct TimestampOverlay {
    burner: TextBurner,
    format: TimestampFormat,
}

impl TimestampOverlay {
    /// Create a new overlay with the given text burner and format.
    pub fn new(burner: TextBurner, format: TimestampFormat) -> Self {
        Self { burner, format }
    }

    /// Render the timestamp overlay into I420 plane buffers.
    ///
    /// * `ts_us` — microseconds since Unix epoch (used when format is `DateTime` or `Combined`).
    /// * `frame_id` — monotonic frame counter (used when format is `FrameCount` or `Combined`).
    #[allow(clippy::too_many_arguments)]
    pub fn burn_i420(
        &self,
        y: &mut [u8],
        u: &mut [u8],
        v: &mut [u8],
        stride_y: usize,
        stride_u: usize,
        stride_v: usize,
        ts_us: i64,
        frame_id: u32,
        width: u32,
        height: u32,
    ) {
        let text = self.format_text(ts_us, frame_id);
        if text.is_empty() {
            return;
        }

        // Compute text region for UV clearing
        let lines: Vec<&str> = text.split('\n').collect();
        let font = self.burner.font();
        let glyph_h = font.glyph_height_px();
        let line_count = lines.len() as u32;
        let max_line_width = lines.iter().map(|l| l.len()).max().unwrap_or(0) as u32;
        let text_w = max_line_width * font.glyph_width_px();
        let text_h = line_count * glyph_h;

        // Resolve anchor position (shared anchor point)
        let (ox, oy) = match self.burner.anchor() {
            super::fonts::Anchor::TopLeft => (4u32, 4u32),
            super::fonts::Anchor::BottomLeft => {
                let y_pos = height.saturating_sub(text_h + 4);
                (4u32, y_pos)
            }
            super::fonts::Anchor::Position { x, y } => (x, y),
        };

        // Clear U/V planes in text region to 128 (no color)
        let u_end_x = ((ox + text_w) / 2).min((width / 2).max(1));
        let u_end_y = ((oy + text_h) / 2).min((height / 2).max(1));
        let u_start_x = (ox / 2).min(u_end_x.saturating_sub(1));
        let u_start_y = (oy / 2).min(u_end_y.saturating_sub(1));

        for row in u_start_y..u_end_y {
            let u_off = row as usize * stride_u;
            let v_off = row as usize * stride_v;
            for px in u_start_x..u_end_x {
                u[u_off + px as usize] = 128;
                v[v_off + px as usize] = 128;
            }
        }

        // Draw each line
        for (i, line) in lines.iter().enumerate() {
            let line_oy = oy + i as u32 * glyph_h;
            self.burner.font().draw_text(y, stride_y, line, ox, line_oy, width, height);
        }
    }

    fn format_text(&self, ts_us: i64, frame_id: u32) -> String {
        match self.format {
            TimestampFormat::DateTime => Self::format_datetime(ts_us),
            TimestampFormat::FrameCount => format!("{:06}", frame_id % 1_000_000),
            TimestampFormat::Combined => {
                let dt = Self::format_datetime(ts_us);
                let fc = format!("{:06}", frame_id % 1_000_000);
                format!("{}\n{}", dt, fc)
            }
        }
    }

    fn format_datetime(ts_us: i64) -> String {
        let secs = ts_us / 1_000_000;
        let nsecs = ((ts_us % 1_000_000) * 1000) as u32;
        match Utc.timestamp_opt(secs, nsecs) {
            LocalResult::Single(dt) => dt.format("%Y-%m-%d %H:%M:%S%.3f").to_string(),
            _ => String::from("invalid ts"),
        }
    }

    /// Get the timestamp format.
    pub fn format(&self) -> TimestampFormat {
        self.format
    }
}

#[cfg(test)]
mod tests {
    use super::super::fonts::{Anchor, BitmapFont};
    use super::*;

    #[test]
    fn datetime_format_produces_iso_style_string() {
        let font = BitmapFont::new();
        let burner = TextBurner::new(font, false, Anchor::TopLeft);
        let overlay = TimestampOverlay::new(burner, TimestampFormat::DateTime);
        // 2025-07-21T12:00:00 UTC in µs since epoch
        // 20290 days * 86400 + 43200 = 1753099200 seconds
        let ts = 1_753_099_200_000_000i64; // µs
        let text = overlay.format_text(ts, 0);
        assert!(text.starts_with("2025-07-21"));
        assert!(text.contains("12:00:00.000"));
    }

    #[test]
    fn frame_count_zero_pads_six_digits() {
        let font = BitmapFont::new();
        let burner = TextBurner::new(font, false, Anchor::TopLeft);
        let overlay = TimestampOverlay::new(burner, TimestampFormat::FrameCount);
        assert_eq!(overlay.format_text(0, 0), "000000");
        assert_eq!(overlay.format_text(0, 42), "000042");
        assert_eq!(overlay.format_text(0, 999999), "999999");
    }

    #[test]
    fn combined_includes_both_lines() {
        let font = BitmapFont::new();
        let burner = TextBurner::new(font, false, Anchor::TopLeft);
        let overlay = TimestampOverlay::new(burner, TimestampFormat::Combined);
        let ts = 1_753_113_600_000_000i64; // µs
        let text = overlay.format_text(ts, 42);
        assert!(text.contains('\n'), "combined should have two lines");
        assert!(text.contains("000042"), "should contain frame count");
    }

    #[test]
    fn burn_i420_clears_uv_to_128() {
        let font = BitmapFont::new();
        let burner = TextBurner::new(font, false, Anchor::TopLeft);
        let overlay = TimestampOverlay::new(burner, TimestampFormat::FrameCount);
        let w = 64u32;
        let h = 32u32;
        let mut y = vec![16u8; (w * h) as usize];
        let mut u = vec![100u8; ((w / 2) * (h / 2)) as usize];
        let mut v = vec![200u8; ((w / 2) * (h / 2)) as usize];

        overlay.burn_i420(
            &mut y,
            &mut u,
            &mut v,
            w as usize,
            (w / 2) as usize,
            (w / 2) as usize,
            0,
            0,
            w,
            h,
        );

        // U/V in the text region should be cleared to 128
        // Text at anchor (4,4), "000000" = 6 chars × 6px = 36px wide, 10px tall
        // UV starts at (4/2, 4/2) = (2, 2), ends around ((4+36)/2, (4+10)/2) = (20, 7)
        assert_eq!(u[2 * (w / 2) as usize + 2], 128, "U should be cleared to 128 in text region");
        assert_eq!(v[2 * (w / 2) as usize + 2], 128, "V should be cleared to 128 in text region");
    }
}
