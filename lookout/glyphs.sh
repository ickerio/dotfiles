#!/bin/bash
# Renders the characters lookout shows into src/glyphs/, which get compiled in. Run again after changing a font or size.
# Each is one row of characters as a PGM (raw 8-bit grayscale, black text on white)
set -e
cd "$(dirname "$0")"
mkdir -p src/glyphs

render() {
    pango-view --no-display --backend=ft2 --pixels --margin=0 --font="$2" -o "src/glyphs/$1.pgm" --text="$3"
}

# Printable ASCII with the digits first, since font.rs finds the baseline from "0". Keep in step with index() in src/font.rs
ascii="0123456789"
for code in $(seq 33 126); do
    c=$(printf "\\$(printf %03o "$code")")
    [[ $c == [0-9] ]] || ascii+=$c
done

# Then an ellipsis for cut text. Keep in step with index() and COUNT in src/font.rs
ascii+="…"

# The bar and the sheet, and the sheet's section headings
render ui   "JetBrains Mono NL 13px"        "$ascii"
render tiny "JetBrains Mono NL Medium 10px" "$ascii"
