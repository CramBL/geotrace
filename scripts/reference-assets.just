# Generation recipes for the reference material's committed assets.

# Render the reference material's display equations from their typst sources (needs typst-cli).
[group("reference-assets")]
generate-reference-equations:
    bash scripts/generate-reference-equations.sh

# Render the TEC reference material's illustration from the committed storm fixture.
[group("reference-assets")]
generate-reference-tec-map:
    cargo nextest run -p gt-map --run-ignored all -E 'test(generate_tec_reference_illustration)'

# Render the TEC reference material's plot illustration from the committed node series.
[group("reference-assets")]
generate-reference-tec-plot:
    cargo nextest run -p gt-plot --run-ignored all -E 'test(generate_tec_reference_plot)'

# Render the interference reference material's illustration from the committed world day.
[group("reference-assets")]
generate-reference-interference-map:
    cargo nextest run -p gt-map --run-ignored all -E 'test(generate_interference_reference_illustration)'
