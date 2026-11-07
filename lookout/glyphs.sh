#!/bin/bash
# Renders the characters lookout shows into src/glyphs/, which get compiled in. Run again after changing a font or size.
# Each is one row of characters as a PGM (raw 8-bit grayscale, black text on white)
set -e
cd "$(dirname "$0")"
mkdir -p src/glyphs

render() {
    pango-view --no-display --backend=ft2 --pixels --margin=0 --font="$2" -o "src/glyphs/$1.pgm" --text="$3"
}

# Printable ASCII with the digits first, since font.rs finds the baseline from "0". Keep in step with ascii() in src/font.rs
ascii="0123456789"
for code in $(seq 33 126); do
    c=$(printf "\\$(printf %03o "$code")")
    [[ $c == [0-9] ]] || ascii+=$c
done

# Keep CLOCK_CHARS in step with src/font.rs
render clock "JetBrains Mono NL Light 82px" "0123456789:"
render text  "JetBrains Mono NL 20px"       "$ascii"
render small "JetBrains Mono NL 14px"       "$ascii"
