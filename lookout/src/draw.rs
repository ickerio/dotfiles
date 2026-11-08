//! Drawing into a premultiplied ARGB pixel buffer: filled boxes, the status icons, and text. Shapes are
//! anti-aliased by sampling each pixel on a grid, which at these sizes is cheap and exact enough

use crate::font::Size;

/// A pixel buffer `w` by `h`, rows top to bottom
pub struct Canvas<'a> {
    pub px: &'a mut [u32],
    pub w: i32,
    pub h: i32,
    /// The grey (0-255) that black stands for: every colour is drawn lifted towards white by it, as if it were seen
    /// through on that grey rather than black, so a design made on black keeps every contrast on a lighter surface.
    /// 0 draws colours as they are
    pub raise: u32,
}

/// Opaque 0xRRGGBB as premultiplied ARGB
pub fn opaque(colour: u32) -> u32 {
    0xff00_0000 | colour
}

impl<'a> Canvas<'a> {
    pub fn new(px: &'a mut [u32], w: i32, h: i32) -> Canvas<'a> {
        Canvas { px, w, h, raise: 0 }
    }

    /// `colour` (0xRRGGBB) as drawn on this canvas: each channel lifted by `raise` the way white light over that grey
    /// would, c + (255 - c) * raise / 255 seen from the other side, so black becomes the grey and white stays white
    fn raised(&self, colour: u32) -> u32 {
        if self.raise == 0 {
            return colour;
        }
        let p = self.raise;
        let ch = |shift: u32| (p + (255 - p) * (colour >> shift & 0xff) / 255) << shift;
        ch(16) | ch(8) | ch(0)
    }

    /// Source over: `colour` (0xRRGGBB) at `alpha` (0-255) onto the pixel at (x, y)
    pub fn blend(&mut self, x: i32, y: i32, colour: u32, alpha: u32) {
        if x < 0 || y < 0 || x >= self.w || y >= self.h || alpha == 0 {
            return;
        }
        let colour = self.raised(colour);
        let out = &mut self.px[(y * self.w + x) as usize];
        if alpha >= 255 {
            *out = opaque(colour);
            return;
        }
        let under = |shift: u32| (*out >> shift & 0xff) * (255 - alpha) / 255;
        let over = |shift: u32| (colour >> shift & 0xff) * alpha / 255;
        *out = (alpha + under(24)) << 24 | (over(16) + under(16)) << 16 | (over(8) + under(8)) << 8 | (over(0) + under(0));
    }

    /// Replaces a box with premultiplied ARGB `argb`, clipped to the canvas
    pub fn fill(&mut self, x: i32, y: i32, w: i32, h: i32, argb: u32) {
        let (x0, y0, x1, y1) = (x.max(0), y.max(0), (x + w).min(self.w), (y + h).min(self.h));
        if x0 >= x1 {
            return;
        }
        // Opaque colours are raised; clear stays clear
        let argb = if argb >> 24 == 0xff { 0xff00_0000 | self.raised(argb & 0xff_ffff) } else { argb };
        for row in y0..y1 {
            self.px[(row * self.w + x0) as usize..(row * self.w + x1) as usize].fill(argb);
        }
    }

    /// An opaque box with corners rounded to `r`. Only the corners are sampled, the rest is filled straight
    pub fn round_rect(&mut self, x: i32, y: i32, w: i32, h: i32, r: f32, colour: u32) {
        let (fx, fy, fw, fh) = (x as f32, y as f32, w as f32, h as f32);
        let k = r.ceil() as i32;
        self.fill(x + k, y, w - 2 * k, h, opaque(colour));
        self.fill(x, y + k, k, h - 2 * k, opaque(colour));
        self.fill(x + w - k, y + k, k, h - 2 * k, opaque(colour));
        for (cx, cy) in [(x, y), (x + w - k, y), (x, y + h - k), (x + w - k, y + h - k)] {
            self.shape(cx, cy, k, k, colour, |px, py| {
                let qx = (px - fx - fw / 2.0).abs() - (fw / 2.0 - r);
                let qy = (py - fy - fh / 2.0).abs() - (fh / 2.0 - r);
                (qx.max(0.0).powi(2) + qy.max(0.0).powi(2)).sqrt() + qx.max(qy).min(0.0) - r <= 0.0
            });
        }
    }

    /// Fills the pixels in the box whose samples `inside` says are in the shape, each by how many of its samples
    /// are, so edges are smooth
    pub fn shape(&mut self, x: i32, y: i32, w: i32, h: i32, colour: u32, inside: impl Fn(f32, f32) -> bool) {
        const N: i32 = 4;
        for py in y..y + h {
            for px in x..x + w {
                let mut hits = 0;
                for sy in 0..N {
                    for sx in 0..N {
                        let (fx, fy) = (px as f32 + (sx as f32 + 0.5) / N as f32, py as f32 + (sy as f32 + 0.5) / N as f32);
                        hits += inside(fx, fy) as u32;
                    }
                }
                self.blend(px, py, colour, hits * 255 / (N * N) as u32);
            }
        }
    }

    /// Draws `s` with its cells' tops at `y`, returns where it ended
    pub fn text(&mut self, x: i32, y: i32, size: Size, s: &str, colour: u32) -> i32 {
        let font = size.font();
        let mut x = x;
        for c in s.chars() {
            font.draw(self, x, y, c, colour);
            x += font.advance;
        }
        x
    }
}

/// Width of `s` in `size`
pub fn width(size: Size, s: &str) -> i32 {
    s.chars().count() as i32 * size.font().advance
}
