// ── Bitmap Font ─────────────────────────────────────────

/// Character-cell width in pixels.
pub struct GlyphWidth(pub u32);

/// Character-cell height in pixels.
pub struct GlyphHeight(pub u32);

impl GlyphWidth {
    /// Default glyph width: 6 pixels.
    pub const DEFAULT: u32 = 6;
}

impl GlyphHeight {
    /// Default glyph height: 10 pixels.
    pub const DEFAULT: u32 = 10;
}

/// Stored bitmap for a single glyph — 10 rows of 6 usable bits each.
/// Bits 5–0 map to columns left→right (MSB = leftmost pixel).
/// Bits 7–6 are unused.
#[derive(Clone, Copy)]
pub struct Glyph(&'static [u8; 10]);

/// A set of glyphs with char→index lookup.
///
/// `N` is the number of glyphs in the set.
pub struct GlyphSet<const N: usize> {
    glyphs: [Glyph; N],
}

impl<const N: usize> GlyphSet<N> {
    /// Create a new glyph set from static glyph data and a lookup function.
    pub fn new(glyphs: [Glyph; N]) -> Self {
        Self { glyphs }
    }

    /// Look up the glyph index for a character.
    /// The default lookup supports: '0'..='9', ':', '.', '-'
    pub fn index_of(ch: char) -> Option<usize> {
        match ch {
            '0'..='9' => Some((ch as usize) - ('0' as usize)),
            ':' => Some(10),
            '.' => Some(11),
            '-' => Some(12),
            _ => None,
        }
    }

    /// Get a reference to the glyph at the given index.
    pub fn get(&self, index: usize) -> Option<&Glyph> {
        self.glyphs.get(index)
    }
}

/// Renders glyphs into a Y-plane buffer with scaling support.
pub struct BitmapFont {
    glyphs: GlyphSet<13>,
    scale_x: u32,
    scale_y: u32,
}

/// Standard glyph data: digits 0-9, ':', '.', '-'.
static GLYPH_DATA: [Glyph; 13] = [
    Glyph(&[0x1E, 0x33, 0x33, 0x33, 0x33, 0x33, 0x33, 0x33, 0x1E, 0x00]), // 0
    Glyph(&[0x0C, 0x1C, 0x0C, 0x0C, 0x0C, 0x0C, 0x0C, 0x0C, 0x3F, 0x00]), // 1
    Glyph(&[0x1E, 0x33, 0x01, 0x03, 0x06, 0x0C, 0x18, 0x30, 0x3F, 0x00]), // 2
    Glyph(&[0x1E, 0x33, 0x01, 0x03, 0x0E, 0x03, 0x01, 0x33, 0x1E, 0x00]), // 3
    Glyph(&[0x03, 0x07, 0x0F, 0x1B, 0x33, 0x3F, 0x03, 0x03, 0x03, 0x00]), // 4
    Glyph(&[0x3F, 0x30, 0x30, 0x3E, 0x03, 0x03, 0x03, 0x33, 0x1E, 0x00]), // 5
    Glyph(&[0x0E, 0x18, 0x30, 0x3E, 0x33, 0x33, 0x33, 0x33, 0x1E, 0x00]), // 6
    Glyph(&[0x3F, 0x03, 0x03, 0x06, 0x0C, 0x0C, 0x0C, 0x0C, 0x0C, 0x00]), // 7
    Glyph(&[0x1E, 0x33, 0x33, 0x33, 0x1E, 0x33, 0x33, 0x33, 0x1E, 0x00]), // 8
    Glyph(&[0x1E, 0x33, 0x33, 0x33, 0x1F, 0x03, 0x06, 0x0C, 0x18, 0x00]), // 9
    Glyph(&[0x00, 0x0C, 0x0C, 0x00, 0x00, 0x0C, 0x0C, 0x00, 0x00, 0x00]), // :
    Glyph(&[0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x0C, 0x0C, 0x00]), // .
    Glyph(&[0x00, 0x00, 0x00, 0x00, 0x3F, 0x00, 0x00, 0x00, 0x00, 0x00]), // -
];

impl Default for BitmapFont {
    fn default() -> Self {
        Self { glyphs: GlyphSet::new(GLYPH_DATA), scale_x: 1, scale_y: 1 }
    }
}

impl BitmapFont {
    /// Create a new BitmapFont with the standard 13-glyph set.
    pub fn new() -> Self {
        Self::default()
    }

    /// Create with unscaled glyphs (scale_x=1, scale_y=1).
    pub fn with_scale(scale_x: u32, scale_y: u32) -> Self {
        Self { glyphs: GlyphSet::new(GLYPH_DATA), scale_x, scale_y }
    }

    /// The current scale factor for X (horizontal).
    pub fn scale_x(&self) -> u32 {
        self.scale_x
    }

    /// The current scale factor for Y (vertical).
    pub fn scale_y(&self) -> u32 {
        self.scale_y
    }

    /// Scaled glyph width in pixels.
    pub fn glyph_width_px(&self) -> u32 {
        GlyphWidth::DEFAULT * self.scale_x
    }

    /// Scaled glyph height in pixels.
    pub fn glyph_height_px(&self) -> u32 {
        GlyphHeight::DEFAULT * self.scale_y
    }

    /// Render one scaled glyph into the Y plane at absolute pixel coordinates `(ox, oy)`.
    /// Pixels outside the frame are silently clipped.
    #[allow(clippy::too_many_arguments)] // 字形栅格化内部函数，参数打包 = 独立重构
    pub fn draw_glyph(
        &self,
        y: &mut [u8],
        stride_y: usize,
        ch: char,
        ox: u32,
        oy: u32,
        width: u32,
        height: u32,
    ) {
        let Some(index) = GlyphSet::<13>::index_of(ch) else {
            return;
        };
        let Some(glyph) = self.glyphs.get(index) else {
            return;
        };

        let glyph_w = GlyphWidth::DEFAULT;
        let glyph_h = GlyphHeight::DEFAULT;

        for row in 0..glyph_h {
            let byte = glyph.0[row as usize];
            let src_py = oy + row * self.scale_y;
            if src_py >= height {
                break;
            }
            for col in 0..glyph_w {
                // ponytail: bit test — glyph_w-1-col maps col 0 to MSB of 6-bit field
                let bit = (byte >> (glyph_w - 1 - col)) & 1;
                if bit == 1 {
                    let src_px = ox + col * self.scale_x;
                    // Scale: repeat pixel scale_x times horizontally, scale_y times vertically
                    for sy in 0..self.scale_y {
                        let py = src_py + sy;
                        if py >= height {
                            break;
                        }
                        for sx in 0..self.scale_x {
                            let px = src_px + sx;
                            if px >= width {
                                break;
                            }
                            y[py as usize * stride_y + px as usize] = 240; // white
                        }
                    }
                }
            }
        }
    }

    /// Render a string of glyphs into the Y plane starting at `(ox, oy)`.
    /// Returns the x-offset after the last glyph drawn (for chaining).
    #[allow(clippy::too_many_arguments)] // 同上：字形渲染链参数，独立重构
    pub fn draw_text(
        &self,
        y: &mut [u8],
        stride_y: usize,
        text: &str,
        ox: u32,
        oy: u32,
        width: u32,
        height: u32,
    ) -> u32 {
        let mut cursor_x = ox;
        let step = self.glyph_width_px();
        for ch in text.chars() {
            self.draw_glyph(y, stride_y, ch, cursor_x, oy, width, height);
            cursor_x += step;
            if cursor_x >= width {
                break;
            }
        }
        cursor_x
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn glyph_set_index_lookup() {
        assert_eq!(GlyphSet::<13>::index_of('0'), Some(0));
        assert_eq!(GlyphSet::<13>::index_of('9'), Some(9));
        assert_eq!(GlyphSet::<13>::index_of(':'), Some(10));
        assert_eq!(GlyphSet::<13>::index_of('.'), Some(11));
        assert_eq!(GlyphSet::<13>::index_of('-'), Some(12));
        assert_eq!(GlyphSet::<13>::index_of('A'), None);
    }

    #[test]
    fn scaled_draw_glyph_output_is_correct_size() {
        let font = BitmapFont::with_scale(2, 3);
        assert_eq!(font.glyph_width_px(), 12); // 6 * 2
        assert_eq!(font.glyph_height_px(), 30); // 10 * 3
    }

    #[test]
    fn draw_text_returns_new_cursor() {
        let font = BitmapFont::new();
        let mut y = vec![0u8; 200 * 200];
        let cursor = font.draw_text(&mut y, 200, "123", 0, 0, 200, 200);
        assert_eq!(cursor, 18); // 3 chars × 6px
    }
}
