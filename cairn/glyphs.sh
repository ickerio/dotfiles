#!/bin/bash
# Renders the characters cairn shows into src/glyphs/, which get compiled in. Run again after changing a font or size.
# Each is one row of characters as a PGM (raw 8-bit grayscale, black text on white)
set -e
cd "$(dirname "$0")"
mkdir -p src/glyphs

render() {
    pango-view --no-display --backend=ft2 --pixels --margin=0 --font="$2" -o "src/glyphs/$1.pgm" --text="$3"
}

# Keep these in step with CLOCK_CHARS, TEXT_CHARS and SMALL_CHARS in src/font.rs
render clock "JetBrains Mono NL Light 82px" "0123456789:"
render text  "JetBrains Mono NL 20px"       "0123456789:./+-ABCEFGIMPRTUVW"
render small "JetBrains Mono NL 14px"       "0123456789()"
