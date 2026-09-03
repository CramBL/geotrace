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

# msrv runs natively rather than via `_dev`: cargo-msrv pulls many toolchains,
# kept out of the dev image to keep it slim. Install it natively (see CI).
[group("native-checks")]
msrv:
    cargo msrv verify --manifest-path Cargo.toml

[group("native-checks")]
sdk-msrv:
    cargo msrv verify --manifest-path sdk/rust/geotrace-sdk/Cargo.toml

# Requests every URL the tracked files cite (network!), under the settings and
# exclusions of lychee.toml. Kept out of the dev image like msrv: install it
# natively with `cargo install lychee` (see CI).
[group("native-checks")]
lychee:
    git ls-files -- '*.rs' '*.md' '*.toml' '*.yml' '*.yaml' | lychee --files-from -

[group("container-rust")]
python-sdk: (_dev "bash" "scripts/python-sdk.sh")
