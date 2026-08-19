#!/usr/bin/env bash
# Renders every display equation of the reference material from its typst
# source to the PNG the reference window shows: black glyphs on a transparent
# page, which the window tints to the theme's text colour.
#
# Output is byte-identical for a given typst version: system fonts are ignored,
# so the glyphs come from the math font typst-cli embeds.
set -euo pipefail

# Twice the 96 pixels per inch the window lays the equation out at, so the
# glyphs stay sharp on a high-dpi display.
PPI=192

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

shopt -s nullglob
for source in "$repo_root"/crates/*/assets/equations/*.typ; do
    name="$(basename "$source" .typ)"
    if [ "$name" = "preamble" ]; then
        continue
    fi
    output="$(dirname "$source")/${name}.png"
    typst compile \
        --format png \
        --ppi "$PPI" \
        --ignore-system-fonts \
        "$source" \
        "$output"
    echo "${output#"$repo_root"/}"
done
