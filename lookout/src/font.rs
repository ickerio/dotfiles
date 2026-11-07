//! JetBrains Mono, pre-rendered by glyphs.sh and compiled in, so there's no font rasteriser to depend on

use std::sync::LazyLock;

/// The clock's characters, in order and starting with 0. Keep in step with glyphs.sh
const CLOCK_CHARS: &str = "0123456789:";

static CLOCK: LazyLock<Font> = LazyLock::new(|| Font::new(CLOCK_CHARS.chars().collect(), include_bytes!("glyphs/clock.pgm")));
static TEXT: LazyLock<Font> = LazyLock::new(|| Font::new(ascii(), include_bytes!("glyphs/text.pgm")));
static SMALL: LazyLock<Font> = LazyLock::new(|| Font::new(ascii(), include_bytes!("glyphs/small.pgm")));
static MARK: LazyLock<Font> = LazyLock::new(marks);

/// Agent markers: small squares of 1-bit pixels like the wallpaper's dithered mountain, as dense as the agent is
/// done. Solid wants you, a checkerboard is working, sparse dots are idle, an outline is done and already seen
pub const SOLID: char = 's';
pub const CHECKER: char = 'c';
pub const SPARSE: char = 'p';
pub const OUTLINE: char = 'o';
/// Width of a marker's cell, the marker and the gap after it
pub const MARK_ADVANCE: i32 = 20;
/// The marker is CELLS by CELLS dots of DOT pixels, the size of the wallpaper's dither
const CELLS: i32 = 5;
const DOT: i32 = 2;


/// Printable ASCII with the digits first, the order glyphs.sh renders it in
fn ascii() -> Vec<char> {
    let digits = '0'..='9';
    digits.clone().chain(('!'..='~').filter(|c| !c.is_ascii_digit())).collect()
}

#[derive(Clone, Copy, PartialEq)]
pub enum Size {
    Clock,
    Text,
    Small,
    /// An agent marker, as tall as Text and centred on its digits
    Mark,
}

impl Size {
    pub fn font(self) -> &'static Font {
        match self {
            Size::Clock => &CLOCK,
            Size::Text => &TEXT,
            Size::Small => &SMALL,
            Size::Mark => &MARK,
        }
    }
}

/// One row of characters, all the same width since the font is monospace
pub struct Font {
    chars: Vec<char>,
    /// Black text on white, one byte per pixel
    pixels: &'static [u8],
    width: i32,
    pub advance: i32,
    pub height: i32,
    /// Rows from the top to the bottom of the digits
    pub baseline: i32,
    /// Rows above the top of the digits
    pub top: i32,
    /// Where each character's ink is in its cell: left, top, right and bottom, the last two exclusive. None for
    /// blank ones. So panels can be fitted to what's drawn rather than to cells
    ink: Vec<Option<[i32; 4]>>,
}

impl Font {
    /// `pgm` is "P5 <width> <height> 255" separated by whitespace, one more whitespace byte, then the pixels
    fn new(chars: Vec<char>, pgm: &'static [u8]) -> Font {
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
        let (pixels, advance) = (&pgm[pos + 1..], width / chars.len() as i32);
        // Every strip starts with "0", whose lowest inked row is the baseline
        let inked = |row: i32| pixels[(row * width) as usize..][..advance as usize].iter().any(|&p| p < 128);
        let baseline = (0..height).rev().find(|&row| inked(row)).map_or(height, |row| row + 1);
        let top = (0..height).find(|&row| inked(row)).unwrap_or(0);
        let ink = measure(chars.len(), pixels, width, advance, height);
        Font { chars, pixels, width, advance, height, baseline, top, ink }
    }

    /// Where `c`'s ink is in its cell, None if it has none
    pub fn ink(&self, c: char) -> Option<[i32; 4]> {
        let index = self.chars.iter().position(|&ch| ch == c)?;
        self.ink[index]
    }

    /// Draws `c` with its top left at (x, y) over what's already in `px`, premultiplied ARGB `w` pixels wide, in
    /// `colour` (0xRRGGBB). Characters it doesn't have are left blank. The text is sized to fit so nothing is clipped
    pub fn draw(&self, px: &mut [u32], w: i32, x: i32, y: i32, c: char, colour: u32) {
        let Some(index) = self.chars.iter().position(|&ch| ch == c) else { return };
        let left = index as i32 * self.advance;
        let channels = [colour >> 16 & 0xff, colour >> 8 & 0xff, colour & 0xff];
        for row in 0..self.height {
            let src = &self.pixels[(row * self.width + left) as usize..][..self.advance as usize];
            let dst = &mut px[((y + row) * w + x) as usize..][..self.advance as usize];
            for (out, &ink) in dst.iter_mut().zip(src) {
                let alpha = 255 - ink as u32;
                if alpha == 0 {
                    continue;
                }
                // Source over: the glyph's coverage of the colour, plus whatever shows through the rest
                let under = |shift: u32| (*out >> shift & 0xff) * (255 - alpha) / 255;
                let [r, g, b] = channels.map(|c| c * alpha / 255);
                *out = (alpha + under(24)) << 24 | (r + under(16)) << 16 | (g + under(8)) << 8 | (b + under(0));
            }
        }
    }
}


/// Draws the markers once, as a strip like the rendered fonts: black on white, one cell each, crisp-edged
fn marks() -> Font {
    let text = &*TEXT;
    let (height, advance) = (text.height, MARK_ADVANCE);
    let patterns: [(char, fn(i32, i32) -> bool); 4] = [
        (SOLID, |_, _| true),
        (CHECKER, |x, y| (x + y) % 2 == 0),
        (SPARSE, |x, y| (x == 1 || x == 3) && (y == 1 || y == 3)),
        (OUTLINE, |x, y| x == 0 || y == 0 || x == CELLS - 1 || y == CELLS - 1),
    ];
    let width = advance * patterns.len() as i32;
    let size = CELLS * DOT;
    // Centred on the digits, at the left of the cell
    let top = (text.top + text.baseline) / 2 - size / 2;
    let mut pixels = vec![255u8; (width * height) as usize];
    for (i, (_, on)) in patterns.iter().enumerate() {
        for y in 0..size {
            for x in 0..size {
                if on(x / DOT, y / DOT) {
                    pixels[((top + y) * width + i as i32 * advance + x) as usize] = 0;
                }
            }
        }
    }
    let pixels: &'static [u8] = Box::leak(pixels.into_boxed_slice());
    let chars: Vec<char> = patterns.iter().map(|(c, _)| *c).collect();
    let ink = measure(chars.len(), pixels, width, advance, height);
    Font { chars, pixels, width, advance, height, baseline: text.baseline, top: text.top, ink }
}

/// The ink rectangle of each of `count` cells in a strip, see Font::ink
fn measure(count: usize, pixels: &[u8], width: i32, advance: i32, height: i32) -> Vec<Option<[i32; 4]>> {
    (0..count as i32)
        .map(|i| {
            let inked = |x: i32, y: i32| pixels[(y * width + i * advance + x) as usize] < 128;
            let cols: Vec<i32> = (0..advance).filter(|&x| (0..height).any(|y| inked(x, y))).collect();
            let rows: Vec<i32> = (0..height).filter(|&y| (0..advance).any(|x| inked(x, y))).collect();
            Some([*cols.first()?, *rows.first()?, cols.last()? + 1, rows.last()? + 1])
        })
        .collect()
}
