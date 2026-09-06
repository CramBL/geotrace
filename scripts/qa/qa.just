[private]
_QA := "uv run --project " + source_directory()

[group("qa")]
check-all:
    {{ _QA }} check-all

[group("qa")]
check-fixed-width-integers:
    {{ _QA }} check-fixed-width-integers

[group("qa")]
check-floating-comments:
    {{ _QA }} check-floating-comments

[group("qa")]
check-narrative-comments:
    {{ _QA }} check-narrative-comments

[group("qa")]
check-no-network:
    {{ _QA }} check-no-network

[group("qa")]
check-raw-colors:
    {{ _QA }} check-raw-colors

[group("qa")]
qa-lint:
    {{ _QA }} ruff check {{ source_directory() }}/src
    {{ _QA }} mypy --strict {{ source_directory() }}/src

# Normalize the capture timestamps in the fixture manifests, so a fresh
# capture diffs against the committed one on served data alone.
[group("qa")]
scrub-capture-manifests *MANIFESTS:
    {{ _QA }} scrub-capture-manifests {{ MANIFESTS }}

[group("qa")]
generate-gold:
    {{ _QA }} generate-gold

# Write the .Z fixtures the gt-ionex compress decoder tests read, checking each
# generated stream against the system gzip before it lands.
[group("qa")]
generate-unix-compress-fixtures:
    {{ _QA }} generate-unix-compress-fixtures

# Download the style packages .vale.ini pins into .vale/styles/.
[group("qa")]
vale-sync:
    {{ _QA }} lint-prose sync

# Lint the tracked Markdown files.
[group("qa")]
vale-docs:
    {{ _QA }} lint-prose docs

# Lint the comments of the tracked Rust, Python, C and C++ files.
[group("qa")]
vale-source:
    {{ _QA }} lint-prose source

# Lint the comments of the justfiles, CMake files and workflow files.
[group("qa")]
vale-scripts:
    {{ _QA }} lint-prose scripts

# Lint every commit message in RANGE.
[group("qa")]
vale-commits RANGE="origin/trunk..HEAD":
    {{ _QA }} lint-prose commits {{ RANGE }}

# The gate: alerts on lines added since BASE, as GitHub annotations, failing on an error.
[group("qa")]
vale-added BASE="origin/trunk":
    {{ _QA }} lint-prose added {{ BASE }}

# Every surface, the whole backlog.
[group("qa")]
vale: vale-docs vale-source vale-scripts vale-commits

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
