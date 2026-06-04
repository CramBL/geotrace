_QA := "uv run --project " + source_directory()

[group("qa")]
check-em-dash:
    {{ _QA }} check-em-dash

[group("qa")]
check-floating-comments:
    {{ _QA }} check-floating-comments

[group("qa")]
qa-lint:
    {{ _QA }} ruff check {{ source_directory() }}/src
    {{ _QA }} mypy --strict {{ source_directory() }}/src

[group("qa")]
generate-gold:
    {{ _QA }} generate-gold

[group("qa")]
test-gold: generate-gold
    cd {{ source_directory() }}/../.. && cargo run --example gold_dataset -p geotrace-sdk
    @echo "TODO: Run C SDK gold dataset example"
    @echo "TODO: Run C++ SDK gold dataset example"
    @echo "TODO: Run Python SDK gold dataset example"
