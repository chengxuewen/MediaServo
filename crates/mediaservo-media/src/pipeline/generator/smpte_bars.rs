// ── SMPTE Color Bars Pattern ────────────────────────────

use super::FramePattern;

/// Standard SMPTE 75% color bars with grayscale gradient below.
///
/// Top 2/3 of the frame: 7 equal-width color bars.
/// Bottom 1/3: grayscale ramp from Y=16 (black) to Y=235 (white).
pub struct SmpteBarsPattern;

impl SmpteBarsPattern {
    /// Create a new SMPTE bars pattern (stateless).
    pub fn new() -> Self {
        Self
    }
}

impl Default for SmpteBarsPattern {
    fn default() -> Self {
        Self::new()
    }
}

/// SMPTE 75% color bar (Y, U, V) values.
const BAR_COLORS: [(u8, u8, u8); 7] = [
    (235, 128, 128), // White
    (210, 16, 146),  // Yellow
    (170, 166, 16),  // Cyan
    (145, 54, 34),   // Green
    (106, 202, 222), // Magenta
    (81, 90, 240),   // Red
    (41, 240, 110),  // Blue
];

impl FramePattern for SmpteBarsPattern {
    #[allow(clippy::too_many_arguments)]
    fn draw(
        &mut self,
        y: &mut [u8],
        u: &mut [u8],
        v: &mut [u8],
        stride_y: usize,
        stride_u: usize,
        stride_v: usize,
        width: u32,
        height: u32,
    ) {
        let bar_height = if height > 0 { (height * 2) / 3 } else { 0 };
        let bar_width = if width >= 7 { width / 7 } else { 1 };
        let gradient_top = bar_height;
        let gradient_h = height.saturating_sub(gradient_top);

        // ── Draw color bars (top 2/3) ─────────────────
        for bar_idx in 0..7u32 {
            let (bar_y, bar_u, bar_v) = BAR_COLORS[bar_idx as usize];
            let start_x = bar_idx * bar_width;
            let end_x = if bar_idx == 6 { width } else { ((bar_idx + 1) * bar_width).min(width) };

            // Y plane
            for row in 0..bar_height {
                let y_off = row as usize * stride_y;
                for px in start_x..end_x {
                    y[y_off + px as usize] = bar_y;
                }
            }

            // UV planes (subsampled)
            let half_start_x = (start_x / 2).min((width / 2).saturating_sub(1));
            let half_end_x = ((end_x / 2).min(width / 2)).max(half_start_x + 1);
            let half_h = bar_height / 2;
            for row in 0..half_h {
                let u_off = row as usize * stride_u;
                let v_off = row as usize * stride_v;
                for px in half_start_x..half_end_x {
                    u[u_off + px as usize] = bar_u;
                    v[v_off + px as usize] = bar_v;
                }
            }
        }

        // ── Draw grayscale gradient (bottom 1/3) ──────
        if gradient_h > 0 {
            for row in 0..gradient_h {
                // Linear interpolation: Y from 16 (top of gradient) to 235 (bottom)
                let t = if gradient_h > 1 { row as f32 / (gradient_h - 1) as f32 } else { 0.0 };
                let gray = (16.0 + t * (235.0 - 16.0)) as u8;
                let py = gradient_top + row;
                let y_off = py as usize * stride_y;
                for px in 0..width {
                    y[y_off + px as usize] = gray;
                }

                // UV planes: neutral gray
                let half_py = py / 2;
                if half_py < height / 2 {
                    let u_off = half_py as usize * stride_u;
                    let v_off = half_py as usize * stride_v;
                    for px in 0..(width / 2) {
                        u[u_off + px as usize] = 128;
                        v[v_off + px as usize] = 128;
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn smpte_bars_produce_color_bars_and_gradient() {
        let mut pattern = SmpteBarsPattern::new();
        let w = 128;
        let h = 72;
        let mut y = vec![0u8; (w * h) as usize];
        let mut u = vec![0u8; ((w / 2) * (h / 2)) as usize];
        let mut v = vec![0u8; ((w / 2) * (h / 2)) as usize];

        pattern.draw(&mut y, &mut u, &mut v, w as usize, (w / 2) as usize, (w / 2) as usize, w, h);

        // Bar area (top 2/3) should have white bar pixel at start
        let bar_height = (h * 2) / 3;
        // First bar (white): Y=235
        assert!(bar_height > 0);
        let first_bar_pixel = y[0]; // row 0, col 0
        assert_eq!(first_bar_pixel, 235, "first bar should be white (Y=235)");

        // Gradient area (bottom 1/3) should start dark
        if bar_height < h {
            let grad_y_off = bar_height as usize * w as usize;
            assert!(
                y[grad_y_off] <= 30,
                "gradient top should be near black, got {}",
                y[grad_y_off]
            );
        }

        // Last row of gradient should be near white
        let last_row = (h - 1) as usize * w as usize;
        assert!(y[last_row] >= 220, "gradient bottom should be near white, got {}", y[last_row]);
    }

    #[test]
    fn smpte_bars_seven_colors_present() {
        let mut pattern = SmpteBarsPattern::new();
        let w = 700;
        let h = 30;
        let mut y = vec![0u8; (w * h) as usize];
        let mut u = vec![0u8; ((w / 2) * (h / 2)) as usize];
        let mut v = vec![0u8; ((w / 2) * (h / 2)) as usize];

        pattern.draw(&mut y, &mut u, &mut v, w as usize, (w / 2) as usize, (w / 2) as usize, w, h);

        let bar_w = w / 7;
        let expected_y_values: [u8; 7] = [235, 210, 170, 145, 106, 81, 41];
        for (i, &expected) in expected_y_values.iter().enumerate() {
            let px = (i as u32 * bar_w) as usize;
            assert_eq!(
                y[px], expected,
                "bar {} at x={}: expected Y={}, got Y={}",
                i, px, expected, y[px]
            );
        }
    }
}
