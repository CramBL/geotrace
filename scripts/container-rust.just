[group("container-rust")]
typos: (_native_or_dev "typos" "typos")

[group("container-rust")]
sort: (_dev "cargo" "sort" "--workspace" "." "sdk/python/geotrace-py" "sdk/rust/geotrace-sdk/fuzz" "crates/gt-logfile/fuzz")

[group("container-rust")]
sort-check: (_native_or_dev "cargo-sort" "cargo sort --workspace --check . sdk/python/geotrace-py sdk/rust/geotrace-sdk/fuzz crates/gt-logfile/fuzz")

[group("container-rust")]
shear *ARGS:
    # cargo-shear reads one project directory per run.
    just _native_or_dev "cargo-shear" "cargo shear --check-test-targets --deny-warnings {{ ARGS }} ."
    just _native_or_dev "cargo-shear" "cargo shear --check-test-targets --deny-warnings {{ ARGS }} sdk/python/geotrace-py"
    just _native_or_dev "cargo-shear" "cargo shear --check-test-targets --deny-warnings {{ ARGS }} sdk/rust/geotrace-sdk/fuzz"
    just _native_or_dev "cargo-shear" "cargo shear --check-test-targets --deny-warnings {{ ARGS }} crates/gt-logfile/fuzz"

# Runs the command directly when its binary is installed, otherwise inside the
# dev container. Keeps ci-essentials runnable without a container runtime.
[no-exit-message]
[private]
_native_or_dev bin *args:
    #!/usr/bin/env bash
    set -euo pipefail
    if command -v {{ bin }} >/dev/null 2>&1; then
        {{ args }}
    else
        just _dev "{{ args }}"
    fi

# msrv runs outside the dev image: cargo-msrv pulls many toolchains and is
# excluded from the image to keep it slim. Install it natively (see CI).
[group("native-checks")]
msrv:
    cargo msrv verify --manifest-path Cargo.toml

[group("native-checks")]
sdk-msrv:
    cargo msrv verify --manifest-path sdk/rust/geotrace-sdk/Cargo.toml

LYCHEE_CHECKED_FILE_GLOBS := "'*.rs' '*.md' '*.toml' '*.yml' '*.yaml'"

# The paths both lychee recipes drop from the file list: the captured responses,
# frozen at the day they were recorded and never edited for link rot, the
# committed fixtures beside them, and lychee.toml, whose regex patterns lychee
# reads as malformed links.
LYCHEE_SKIPPED_PATHS := '(^|/)tests/(captures|fixtures)/|^lychee\.toml$'

# Requests every URL the tracked files cite (network!), under the settings and
# exclusions of lychee.toml. Kept out of the dev image like msrv: install it
# natively with `cargo install lychee` (see CI).
[group("native-checks")]
lychee:
    git ls-files -- {{ LYCHEE_CHECKED_FILE_GLOBS }} | grep -v -E '{{ LYCHEE_SKIPPED_PATHS }}' | lychee --files-from -

# ci_extras.yml runs this on a pull request and on a push to trunk.
# Requests the URLs on lines added since BASE (network!), under the settings
# and exclusions of lychee.toml.
[group("native-checks")]
lychee-added BASE="origin/trunk":
    #!/usr/bin/env bash
    set -euo pipefail
    mapfile -t files < <(git diff --name-only "{{ BASE }}...HEAD" -- {{ LYCHEE_CHECKED_FILE_GLOBS }} | grep -v -E '{{ LYCHEE_SKIPPED_PATHS }}')
    if [ "${#files[@]}" -eq 0 ]; then
        exit 0
    fi
    added=$(git diff -U0 "{{ BASE }}...HEAD" -- "${files[@]}" | grep -E '^\+' | grep -v -E '^\+\+\+' || true)
    if [ -z "$added" ]; then
        exit 0
    fi
    printf '%s\n' "$added" | lychee -

[group("container-rust")]
python-sdk: (_dev "bash" "scripts/python-sdk.sh")
