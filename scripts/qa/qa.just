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
check-raw-colors:
    {{ _QA }} check-raw-colors

[group("qa")]
qa-lint:
    {{ _QA }} ruff check {{ source_directory() }}/src
    {{ _QA }} mypy --strict {{ source_directory() }}/src

[group("qa")]
generate-gold:
    {{ _QA }} generate-gold

# Run the QA Python unit tests.
[group("qa")]
test *ARGS:
    {{ _QA }} pytest {{ source_directory() }}/tests {{ ARGS }}

# Verify the SDK version is consistent across all of its manifests, headers, and
# CMake files. Pass `--expect X.Y.Z` to also require that version and a matching
# CHANGELOG_SDK.md `## [X.Y.Z]` section.
[group("qa")]
check-versions *ARGS:
    {{ _QA }} versions check {{ ARGS }}

# Verify the GUI app (workspace) version. Pass `--expect X.Y.Z` to also require
# that version and a matching CHANGELOG.md release section.
[group("qa")]
check-app *ARGS:
    {{ _QA }} versions check-app {{ ARGS }}

# Set every SDK version (crates, macro pin, Python, C/C++ headers, CMake) to
# VERSION and promote CHANGELOG_SDK.md's [unreleased] section to it. Tag the
# result `geotrace-sdk-vVERSION` to release.
[group("qa")]
bump-sdk VERSION:
    {{ _QA }} versions bump-sdk {{ VERSION }}

# Set the GUI app (workspace) version to VERSION and promote CHANGELOG.md's
# unreleased section to it. Tag `app/vVERSION` to release.
[group("qa")]
bump-app VERSION:
    {{ _QA }} versions bump-app {{ VERSION }}
