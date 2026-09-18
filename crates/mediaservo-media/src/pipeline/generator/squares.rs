// ── Squares Pattern ─────────────────────────────────────

use rand::Rng;
use rand::SeedableRng;

use super::FramePattern;

/// A single randomly placed, colored square with fixed direction velocity.
#[derive(Clone, Debug)]
pub struct Square {
    pub x: u32,
    pub y: u32,
    pub size: u32,
    pub y_val: u8,
    pub u_val: u8,
    pub v_val: u8,
    pub dx: i32,
    pub dy: i32,
}

/// Strategy for assigning colors to squares.
#[derive(Clone, Debug, Default)]
pub enum ColorStrategy {
    /// New random colors for all squares on every frame.
    RandomPerFrame,
    /// Each square keeps its initial random color (existing behavior).
    #[default]
    RandomPerSquare,
    /// Explicit list of (Y, U, V) tuples, cycled per square.
    Fixed(Vec<(u8, u8, u8)>),
}

/// Configuration for the squares pattern.
pub struct SquaresConfig {
    pub count: u32,
    pub min_size: u32,
    pub max_size: u32,
    /// Fixed-direction velocity (pixels per frame per axis).
    /// Bounces off edges like classic webrtc-kit/opencv test patterns.
    /// 0 = static; n > 0 = max speed magnitude per axis.
    pub motion_speed: u32,
    pub color_strategy: ColorStrategy,
}

impl Default for SquaresConfig {
    fn default() -> Self {
        Self {
            count: 10,
            min_size: 8,
            max_size: 63,
            motion_speed: 0,
            color_strategy: ColorStrategy::default(),
        }
    }
}

/// Generates a pattern of colored squares drawn on a YUV background.
pub struct SquaresPattern {
    squares: Vec<Square>,
    config: SquaresConfig,
    rng: rand::rngs::StdRng,
    fixed_colors: Vec<(u8, u8, u8)>,
    fixed_index: usize,
}

impl SquaresPattern {
    /// Create a new pattern with the given configuration.
    pub fn with_config(width: u32, height: u32, config: SquaresConfig) -> Self {
        let mut rng = rand::rngs::StdRng::from_entropy();
        let squares = Self::generate_squares(&mut rng, &config, width, height);
        let fixed_colors = match &config.color_strategy {
            ColorStrategy::Fixed(colors) => colors.clone(),
            _ => Vec::new(),
        };

        Self { squares, config, rng, fixed_colors, fixed_index: 0 }
    }

    /// Convenience constructor: random-per-square colors, no motion.
    pub fn new(width: u32, height: u32, num_squares: u32) -> Self {
        let config = SquaresConfig { count: num_squares, ..Default::default() };
        Self::with_config(width, height, config)
    }

    fn generate_squares(
        rng: &mut rand::rngs::StdRng,
        config: &SquaresConfig,
        width: u32,
        height: u32,
    ) -> Vec<Square> {
        let mut rng_clone = rng.clone();
        let mut squares = Vec::with_capacity(config.count as usize);
        for i in 0..config.count as usize {
            let size = rng_clone.gen_range(config.min_size..=config.max_size);
            let max_x = width.saturating_sub(size);
            let max_y = height.saturating_sub(size);
            let x = if max_x > 0 { rng_clone.gen_range(0..max_x) } else { 0 };
            let y = if max_y > 0 { rng_clone.gen_range(0..max_y) } else { 0 };

            let (y_val, u_val, v_val) = match &config.color_strategy {
                ColorStrategy::Fixed(colors) if !colors.is_empty() => {
                    let c = colors[i % colors.len()];
                    (c.0, c.1, c.2)
                }
                _ => (
                    rng_clone.gen_range(60u8..200),
                    rng_clone.gen_range(80u8..176),
                    rng_clone.gen_range(80u8..176),
                ),
            };

            let dx = if config.motion_speed > 0 {
                rng_clone.gen_range(1..=config.motion_speed) as i32
                    * if rng_clone.gen_bool(0.5) { 1 } else { -1 }
            } else {
                0
            };
            let dy = if config.motion_speed > 0 {
                rng_clone.gen_range(1..=config.motion_speed) as i32
                    * if rng_clone.gen_bool(0.5) { 1 } else { -1 }
            } else {
                0
            };

            squares.push(Square { x, y, size, y_val, u_val, v_val, dx, dy });
        }
        squares
    }
}

impl FramePattern for SquaresPattern {
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
        // 1. Background fill
        y.fill(128); // medium gray
        u.fill(128); // gray chroma = no color bias
        v.fill(128);

        // 2. Move by fixed velocity, bounce off edges
        if self.config.motion_speed > 0 {
            for sq in &mut self.squares {
                let max_x = width.saturating_sub(sq.size) as i32;
                let max_y = height.saturating_sub(sq.size) as i32;

                let mut new_x = sq.x as i32 + sq.dx;
                let mut new_y = sq.y as i32 + sq.dy;

                // Bounce off left/right edges
                if new_x < 0 {
                    new_x = -new_x;
                    sq.dx = -sq.dx;
                } else if new_x > max_x {
                    new_x = 2 * max_x - new_x;
                    sq.dx = -sq.dx;
                }
                // Bounce off top/bottom edges
                if new_y < 0 {
                    new_y = -new_y;
                    sq.dy = -sq.dy;
                } else if new_y > max_y {
                    new_y = 2 * max_y - new_y;
                    sq.dy = -sq.dy;
                }

                sq.x = new_x.clamp(0, max_x) as u32;
                sq.y = new_y.clamp(0, max_y) as u32;
            }
        }

        // 3. Regenerate colors if RandomPerFrame
        if matches!(self.config.color_strategy, ColorStrategy::RandomPerFrame) {
            for sq in &mut self.squares {
                sq.y_val = self.rng.gen_range(60u8..200);
                sq.u_val = self.rng.gen_range(80u8..176);
                sq.v_val = self.rng.gen_range(80u8..176);
            }
        }

        // 4. If Fixed color strategy with cycling
        if matches!(self.config.color_strategy, ColorStrategy::Fixed(_))
            && !self.fixed_colors.is_empty()
        {
            for (i, sq) in self.squares.iter_mut().enumerate() {
                let c = self.fixed_colors[(self.fixed_index + i) % self.fixed_colors.len()];
                sq.y_val = c.0;
                sq.u_val = c.1;
                sq.v_val = c.2;
            }
            self.fixed_index = self.fixed_index.wrapping_add(1);
        }

        let half_w = width / 2;
        let half_h = height / 2;

        // 5. Draw squares
        for sq in &self.squares {
            let sx = sq.x;
            let sy = sq.y;
            let sz = sq.size;
            let end_x = (sx + sz).min(width);
            let end_y = (sy + sz).min(height);

            // ponytail: nearest-neighbor fill — simple nested loops
            for row in sy..end_y {
                let y_off = row as usize * stride_y;
                for col in sx..end_x {
                    y[y_off + col as usize] = sq.y_val;
                }
            }

            // UV planes are subsampled 2:1 both dimensions
            let ux = (sx / 2).min(half_w.saturating_sub(1));
            let uy = (sy / 2).min(half_h.saturating_sub(1));
            let u_end_x = ((end_x / 2).min(half_w)).max(ux + 1);
            let u_end_y = ((end_y / 2).min(half_h)).max(uy + 1);

            for row in uy..u_end_y {
                let u_off = row as usize * stride_u;
                let v_off = row as usize * stride_v;
                for col in ux..u_end_x {
                    u[u_off + col as usize] = sq.u_val;
                    v[v_off + col as usize] = sq.v_val;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn squares_pattern_creates_correct_number_of_squares() {
        let config = SquaresConfig { count: 5, ..Default::default() };
        let pattern = SquaresPattern::with_config(64, 48, config);
        assert_eq!(pattern.squares.len(), 5);
    }

    #[test]
    fn squares_pattern_with_fixed_colors() {
        let config = SquaresConfig {
            count: 3,
            color_strategy: ColorStrategy::Fixed(vec![
                (200, 100, 100),
                (100, 200, 100),
                (100, 100, 200),
            ]),
            ..Default::default()
        };
        let pattern = SquaresPattern::with_config(32, 32, config);
        assert_eq!(pattern.squares.len(), 3);
    }

    #[test]
    fn motion_keeps_squares_in_bounds() {
        let config = SquaresConfig {
            count: 1,
            min_size: 8,
            max_size: 8,
            motion_speed: 5,
            ..Default::default()
        };
        let w = 32u32;
        let h = 32u32;
        let mut pattern = SquaresPattern::with_config(w, h, config);
        let mut y = vec![0u8; (w * h) as usize];
        let mut u = vec![128u8; ((w / 2) * (h / 2)) as usize];
        let mut v = vec![128u8; ((w / 2) * (h / 2)) as usize];

        // Draw many frames and verify squares stay in bounds
        for _ in 0..100 {
            pattern.draw(
                &mut y,
                &mut u,
                &mut v,
                w as usize,
                (w / 2) as usize,
                (w / 2) as usize,
                w,
                h,
            );
            for sq in &pattern.squares {
                assert!(
                    sq.x + sq.size <= w,
                    "square x out of bounds: {} + {} > {}",
                    sq.x,
                    sq.size,
                    w
                );
                assert!(
                    sq.y + sq.size <= h,
                    "square y out of bounds: {} + {} > {}",
                    sq.y,
                    sq.size,
                    h
                );
            }
        }
    }
}
