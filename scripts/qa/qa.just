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
