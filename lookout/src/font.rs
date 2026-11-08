//! JetBrains Mono, pre-rendered by glyphs.sh and compiled in, so there's no font rasteriser to depend on

use std::sync::LazyLock;

use crate::draw::Canvas;

static UI: LazyLock<Font> = LazyLock::new(|| Font::new(include_bytes!("glyphs/ui.pgm")));
static TINY: LazyLock<Font> = LazyLock::new(|| Font::new(include_bytes!("glyphs/tiny.pgm")));

#[derive(Clone, Copy, PartialEq)]
pub enum Size {
    /// Everything in the bar and the sheet
    Ui,
    /// The sheet's section headings
    Tiny,
}

impl Size {
    pub fn font(self) -> &'static Font {
        match self {
            Size::Ui => &UI,
            Size::Tiny => &TINY,
        }
    }
}

/// How many characters a strip has: printable ASCII with the digits first, then "…", the order glyphs.sh renders
/// them in. Keep in step with it
const COUNT: i32 = 95;

/// Where `c` is in a strip, None for what isn't there, such as a space, which is left blank
fn index(c: char) -> Option<i32> {
    let n = c as i32;
    match c {
        '0'..='9' => Some(n - '0' as i32),
        // The rest of printable ASCII in order, closing up the gap the digits left
        '!'..='/' => Some(10 + n - '!' as i32),
        ':'..='~' => Some(10 + n - '!' as i32 - 10),
        '…' => Some(COUNT - 1),
        _ => None,
    }
}

/// One row of characters, all the same width since the font is monospace
pub struct Font {
    /// Black text on white, one byte per pixel
    pixels: &'static [u8],
    width: i32,
    pub advance: i32,
    pub height: i32,
    /// Rows from the top of the cell to the bottom of the digits
    pub baseline: i32,
    /// Rows from the top of the cell to the top of the digits
    pub top: i32,
}

impl Font {
    /// `pgm` is "P5 <width> <height> 255" separated by whitespace, one more whitespace byte, then the pixels
    fn new(pgm: &'static [u8]) -> Font {
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
        let (pixels, advance) = (&pgm[pos + 1..], width / COUNT);
        // Every strip starts with "0", whose inked rows are the digits' height
        let inked = |row: i32| pixels[(row * width) as usize..][..advance as usize].iter().any(|&p| p < 128);
        let baseline = (0..height).rev().find(|&row| inked(row)).map_or(height, |row| row + 1);
        let top = (0..height).find(|&row| inked(row)).unwrap_or(0);
        Font { pixels, width, advance, height, baseline, top }
    }

    /// Draws `c` with its cell's top left at (x, y) in `colour` (0xRRGGBB). Characters it doesn't have are left
    /// blank, and what falls outside the canvas is clipped
    pub fn draw(&self, canvas: &mut Canvas, x: i32, y: i32, c: char, colour: u32) {
        let Some(index) = index(c) else { return };
        let left = index * self.advance;
        for row in 0..self.height {
            let src = &self.pixels[(row * self.width + left) as usize..][..self.advance as usize];
            for (col, &ink) in src.iter().enumerate() {
                canvas.blend(x + col as i32, y + row, colour, 255 - ink as u32);
            }
        }
    }
}
