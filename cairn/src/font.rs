//! JetBrains Mono, pre-rendered by glyphs.sh and compiled in, since there's no font rasteriser without dependencies

use std::sync::LazyLock;

/// The characters in each strip, in order, each starting with 0. Keep these in step with glyphs.sh
const CLOCK_CHARS: &str = "0123456789:";
const TEXT_CHARS: &str = "0123456789:./+-ABCEFGIMPRTUVW";
const SMALL_CHARS: &str = "0123456789()";

static CLOCK: LazyLock<Font> = LazyLock::new(|| Font::new(CLOCK_CHARS, include_bytes!("glyphs/clock.pgm")));
static TEXT: LazyLock<Font> = LazyLock::new(|| Font::new(TEXT_CHARS, include_bytes!("glyphs/text.pgm")));
static SMALL: LazyLock<Font> = LazyLock::new(|| Font::new(SMALL_CHARS, include_bytes!("glyphs/small.pgm")));

#[derive(Clone, Copy, PartialEq)]
pub enum Size {
    Clock,
    Text,
    Small,
}

impl Size {
    pub fn font(self) -> &'static Font {
        match self {
            Size::Clock => &CLOCK,
            Size::Text => &TEXT,
            Size::Small => &SMALL,
        }
    }
}

/// One row of characters, all the same width since the font is monospace
pub struct Font {
    chars: &'static str,
    /// Black text on white, one byte per pixel
    pixels: &'static [u8],
    width: i32,
    pub advance: i32,
    pub height: i32,
    /// Rows from the top to the bottom of the digits
    pub baseline: i32,
}

impl Font {
    /// `pgm` is "P5 <width> <height> 255" separated by whitespace, one more whitespace byte, then the pixels
    fn new(chars: &'static str, pgm: &'static [u8]) -> Font {
        let mut pos = 0;
        let mut fields = [0; 4];
        for field in &mut fields {
            while pgm[pos].is_ascii_whitespace() {
                pos += 1;
            }
            let start = pos;
            while !pgm[pos].is_ascii_whitespace() {
                pos += 1;
            }
            *field = std::str::from_utf8(&pgm[start..pos]).ok().and_then(|f| f.parse().ok()).unwrap_or(0);
        }
        let [_, width, height, _] = fields;
        let (pixels, advance) = (&pgm[pos + 1..], width / chars.chars().count() as i32);
        // Every strip starts with "0", whose lowest inked row is the baseline
        let inked = |row: i32| pixels[(row * width) as usize..][..advance as usize].iter().any(|&p| p < 128);
        let baseline = (0..height).rev().find(|&row| inked(row)).map_or(height, |row| row + 1);
        Font { chars, pixels, width, advance, height, baseline }
    }

    /// Draws `c` with its top left at (x, y) into `px`, premultiplied ARGB `w` pixels wide, in `grey`.
    /// The text is sized to fit so nothing is clipped
    pub fn draw(&self, px: &mut [u32], w: i32, x: i32, y: i32, c: char, grey: u8) {
        let Some(index) = self.chars.chars().position(|ch| ch == c) else { return };
        let left = index as i32 * self.advance;
        for row in 0..self.height {
            let src = &self.pixels[(row * self.width + left) as usize..][..self.advance as usize];
            let dst = &mut px[((y + row) * w + x) as usize..][..self.advance as usize];
            for (out, &ink) in dst.iter_mut().zip(src) {
                let alpha = 255 - ink as u32;
                let v = grey as u32 * alpha / 255;
                *out = alpha << 24 | v << 16 | v << 8 | v;
            }
        }
    }
}
