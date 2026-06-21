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

# Verify the SDK version is consistent across all of its manifests, headers, and
# CMake files. Pass `--expect X.Y.Z` to also require a specific version.
[group("qa")]
check-versions *ARGS:
    {{ _QA }} versions check {{ ARGS }}

# Set every SDK version (crates, macro pin, Python, C/C++ headers, CMake) to
# VERSION. Tag the result `geotrace-sdk-vVERSION` to release.
[group("qa")]
bump-sdk VERSION:
    {{ _QA }} versions bump-sdk {{ VERSION }}

# Set the GUI app (workspace) version to VERSION. Tag `app/vVERSION` to release.
[group("qa")]
bump-app VERSION:
    {{ _QA }} versions bump-app {{ VERSION }}
