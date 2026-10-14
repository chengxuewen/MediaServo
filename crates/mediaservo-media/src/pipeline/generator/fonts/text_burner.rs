// ── TextBurner ──────────────────────────────────────────

use super::bitmap_font::BitmapFont;

/// Where to anchor the text within the frame.
#[derive(Debug, Clone, Copy)]
pub enum Anchor {
    /// Top-left corner with a small margin (4, 4).
    TopLeft,
    /// Bottom-left corner with a small margin.
    BottomLeft,
    /// Explicit pixel position.
    Position { x: u32, y: u32 },
}

/// Renders text into a Y-plane buffer with optional background fill.
pub struct TextBurner {
    font: BitmapFont,
    bg_fill: bool,
    anchor: Anchor,
}

impl TextBurner {
    /// Create a new TextBurner with the given font, background fill mode, and anchor.
    pub fn new(font: BitmapFont, bg_fill: bool, anchor: Anchor) -> Self {
        Self { font, bg_fill, anchor }
    }

    /// Render `text` into the Y-plane buffer at the anchor position.
    ///
    /// When `bg_fill` is true, the text background rectangle is filled with Y=80
    /// (dark gray) before drawing the glyphs.
    pub fn burn(&self, y: &mut [u8], stride_y: usize, text: &str, width: u32, height: u32) {
        let (ox, oy) = self.resolve_anchor(width, height);

        if self.bg_fill && !text.is_empty() {
            // Fill background rect: (ox, oy) → (ox + text_width, oy + glyph_height)
            let tw = (text.len() as u32) * self.font.glyph_width_px();
            let th = self.font.glyph_height_px();
            let end_x = (ox + tw).min(width);
            let end_y = (oy + th).min(height);
            for row in oy..end_y {
                let off = row as usize * stride_y;
                for col in ox..end_x {
                    y[off + col as usize] = 80; // dark gray
                }
            }
        }

        self.font.draw_text(y, stride_y, text, ox, oy, width, height);
    }

    /// Get the anchor position.
    pub fn anchor(&self) -> Anchor {
        self.anchor
    }

    /// Get a reference to the underlying font.
    pub fn font(&self) -> &BitmapFont {
        &self.font
    }

    fn resolve_anchor(&self, width: u32, height: u32) -> (u32, u32) {
        const MARGIN: u32 = 4;

        match self.anchor {
            Anchor::TopLeft => (MARGIN, MARGIN),
            Anchor::BottomLeft => {
                let y = height.saturating_sub(self.font.glyph_height_px() + MARGIN);
                (MARGIN, y)
            }
            Anchor::Position { x, y } => {
                (x.min(width.saturating_sub(1)), y.min(height.saturating_sub(1)))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn top_left_anchor_positions_at_margin() {
        let font = BitmapFont::new();
        let burner = TextBurner::new(font, false, Anchor::TopLeft);
        let (x, y) = burner.resolve_anchor(100, 60);
        assert_eq!(x, 4);
        assert_eq!(y, 4);
    }

    #[test]
    fn bottom_left_anchor_positions_near_bottom() {
        let font = BitmapFont::new();
        let burner = TextBurner::new(font, false, Anchor::BottomLeft);
        let (_, y) = burner.resolve_anchor(100, 60);
        // glyph height 10, margin 4 → y = 60 - 14 = 46
        assert_eq!(y, 46);
    }

    #[test]
    fn position_anchor_respects_bounds() {
        let font = BitmapFont::new();
        let burner = TextBurner::new(font, false, Anchor::Position { x: 999, y: 999 });
        let (x, y) = burner.resolve_anchor(100, 60);
        assert_eq!(x, 99);
        assert_eq!(y, 59);
    }

    #[test]
    fn bg_fill_paints_dark_gray() {
        let font = BitmapFont::new();
        let burner = TextBurner::new(font, true, Anchor::TopLeft);
        let mut y = vec![0u8; 20 * 20];
        burner.burn(&mut y, 20, "12", 20, 20);
        // The bg fill should have set Y=80 in the text region
        // Glyph width = 6, scale=1, so 2 chars = 12px wide, 10px tall
        // Margin is 4,4, so region is (4,4) to (16,14)
        assert_eq!(y[4 * 20 + 4], 80); // bg fill
        // The glyph pixel should be white (240) where it overlaps
        // Check that some pixels are white where glyph drew
        let white_count = y.iter().filter(|&&v| v == 240).count();
        assert!(white_count > 0, "expected white glyph pixels");
    }
}
