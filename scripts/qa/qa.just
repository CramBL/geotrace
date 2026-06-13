[private]
_QA := "uv run --project " + source_directory()

[group("qa")]
check-all:
    {{ _QA }} check-all

[group("qa")]
check-em-dash:
    {{ _QA }} check-em-dash

[group("qa")]
check-floating-comments:
    {{ _QA }} check-floating-comments

[group("qa")]
check-narrative-comments:
    {{ _QA }} check-narrative-comments

[group("qa")]
qa-lint:
    {{ _QA }} ruff check {{ source_directory() }}/src
    {{ _QA }} mypy --strict {{ source_directory() }}/src

[group("qa")]
generate-gold:
    {{ _QA }} generate-gold
