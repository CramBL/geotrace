alias c := check
alias b := build
alias l := clippy
alias t := test
alias r := run
alias cie := ci-essentials
alias cix := ci-extras

import 'container.just'
import 'scripts/container-rust.just'
import 'scripts/fixtures.just'
import 'scripts/reference-assets.just'
import 'scripts/sdk.just'
mod qa 'scripts/qa/qa.just'
mod release 'scripts/release.just'

[default]
[private]
default:
    @just --list

[group("native")]
run *ARGS:
    cargo run {{ ARGS }}

# Formats every workspace: `fmt-check-all` is then green whichever one was edited.
[group("native")]
fmt:
    cargo fmt --all
    cargo fmt --manifest-path sdk/python/geotrace-py/Cargo.toml --all
    cargo fmt --manifest-path sdk/rust/geotrace-sdk/fuzz/Cargo.toml --all

[group("native")]
fmt-check:
    cargo fmt --all --check

[group("native")]
fmt-check-sdk:
    cargo fmt --manifest-path sdk/python/geotrace-py/Cargo.toml --all --check
    cargo fmt --manifest-path sdk/rust/geotrace-sdk/fuzz/Cargo.toml --all --check

[group("native")]
fmt-check-all: fmt-check fmt-check-sdk

[group("native")]
check *ARGS:
    cargo check {{ ARGS }}

[group("native")]
check-sdk:
    # geotrace-py declares abi3-py312. PYO3_BUILD_EXTENSION_MODULE=1 builds it as
    # an extension module, which pyo3 configures without an interpreter: the dev
    # image's Python is 3.11.
    PYO3_BUILD_EXTENSION_MODULE=1 cargo check --manifest-path sdk/python/geotrace-py/Cargo.toml
    cargo check --manifest-path sdk/rust/geotrace-sdk/fuzz/Cargo.toml

[group("native")]
check-all: check check-sdk

[group("native")]
build *ARGS:
    cargo build {{ ARGS }}

[group("native")]
clippy:
    cargo clippy --workspace --no-deps -- -D warnings
    cargo clippy --workspace --no-deps --tests -- -D warnings
    cargo clippy --workspace --no-deps --examples -- -D warnings
    cargo clippy --workspace --no-deps --benches -- -D warnings
    # The dist-only self-update code is feature-gated, so lint it explicitly too.
    cargo clippy --workspace --no-deps --features geotrace/self-update --tests -- -D warnings

# The SDK workspaces declare their own lints at `warn` level in their manifests:
# `-D warnings` is where CI turns those into errors.
[group("native")]
clippy-sdk:
    PYO3_BUILD_EXTENSION_MODULE=1 cargo clippy --manifest-path sdk/python/geotrace-py/Cargo.toml --workspace --no-deps --all-targets -- -D warnings
    cargo clippy --manifest-path sdk/rust/geotrace-sdk/fuzz/Cargo.toml --workspace --no-deps --all-targets -- -D warnings

[group("native")]
clippy-all: clippy clippy-sdk

# Every test CI runs. geotrace-py's test binary links libpython, which
# needs an interpreter at its abi3 minimum of 3.12: the dev image has 3.11.
# The fuzz targets set `test = false`.
[group("native")]
test *ARGS:
    cargo nextest run --workspace --features geotrace/self-update {{ ARGS }}

[group("native")]
test-backends:
    @echo "Running tests for pure backend"
    cargo nextest run -p gt-history --test integration --no-default-features --features backend-pure
    cargo nextest run -p gt-store --test log_attachments --no-default-features --features backend-pure
    @echo "Running tests for sys backend"
    cargo nextest run -p gt-history --test integration --no-default-features --features backend-sys
    cargo nextest run -p gt-store --test log_attachments --no-default-features --features backend-sys

[group("native")]
test-snapshots *ARGS:
    cargo nextest run --workspace --features geotrace/self-update -E "test(snapshot)" {{ ARGS }}

[group("native")]
examples:
    bash scripts/examples.sh


# Run the gt-snap live-API smoke test against the real Valhalla server
# (network!) - the on-demand drift check for the API boundary.
[group("native")]
snap-live-test:
    cargo nextest run --profile live --run-ignored all -p gt-snap

[group("native")]
sdk-doc:
    RUSTDOCFLAGS="-D warnings" cargo doc -p geotrace-sdk --no-deps

# Build every workspace's docs, failing on broken intra-doc links.
[group("native")]
doc:
    RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps
    RUSTDOCFLAGS="-D warnings" PYO3_BUILD_EXTENSION_MODULE=1 cargo doc --manifest-path sdk/python/geotrace-py/Cargo.toml --workspace --no-deps
    RUSTDOCFLAGS="-D warnings" cargo doc --manifest-path sdk/rust/geotrace-sdk/fuzz/Cargo.toml --workspace --no-deps

[group("ci")]
ci: build-images ci-essentials ci-extras ci-sdks

[group("ci")]
ci-essentials: fmt-check-all clippy-all check-all test examples qa::qa-lint qa::test qa::check-all qa::check-versions qa::check-app doc check-unit-bindings check-c-header osv-scanner sort-check shear typos qa::vale-added

[group("ci")]
ci-extras: msrv sdk-msrv sdk-doc lychee

[group("ci")]
ci-sdks: python-sdk build-c fmt-c lint-c test-c fmt-cpp lint-cpp test-cpp qa::generate-gold test-gold-all
