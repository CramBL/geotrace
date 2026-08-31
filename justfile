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

# The cargo workspaces in the repository. Adding a fourth is one line here:
# every gate below reads this list. `geotrace-py` and the fuzz targets are
# isolated by design. A PyO3 cdylib needs unwinding, and the root's
# `panic = "abort"` release profile takes it away. A cargo-fuzz target stays
# out of every `--workspace` build.
WORKSPACE_MANIFESTS := "./Cargo.toml sdk/python/geotrace-py/Cargo.toml sdk/rust/geotrace-sdk/fuzz/Cargo.toml"
WORKSPACE_DIRS := replace(WORKSPACE_MANIFESTS, "/Cargo.toml", "")
WORKSPACE_LOCKFILES := replace(WORKSPACE_MANIFESTS, "Cargo.toml", "Cargo.lock")

[default]
[private]
default:
    @just --list

[group("native")]
run *ARGS:
    cargo run {{ ARGS }}

# Run `cargo SUBCOMMAND` over every workspace. FLAGS goes after the manifest
# path: a trailing `-- -D warnings` still ends up last.
# PYO3_BUILD_EXTENSION_MODULE builds geotrace-py the way maturin ships it, as an
# abi3 extension module: pyo3 then needs neither an interpreter at its abi3
# minimum (3.12, above the dev image's 3.11) nor a libpython to link against.
[private]
_cargo-in-every-workspace SUBCOMMAND *FLAGS:
    #!/usr/bin/env bash
    set -euo pipefail
    export PYO3_BUILD_EXTENSION_MODULE=1
    for manifest in {{ WORKSPACE_MANIFESTS }}; do
        cargo {{ SUBCOMMAND }} --manifest-path "$manifest" {{ FLAGS }}
    done

[group("native")]
fmt: (_cargo-in-every-workspace "fmt" "--all")

[group("native")]
fmt-check: (_cargo-in-every-workspace "fmt" "--all --check")

# Compiles every workspace. A single crate is `cargo check -p <crate>`.
[group("native")]
check: (_cargo-in-every-workspace "check")

[group("native")]
build *ARGS:
    cargo build {{ ARGS }}

[group("native")]
clippy:
    #!/usr/bin/env bash
    set -euo pipefail
    # The isolated workspaces declare their own lints at `warn` level in their
    # manifests: `-D warnings` is where CI turns those into errors.
    for targets in "" --tests --examples --benches; do
        just _cargo-in-every-workspace clippy "--workspace --no-deps $targets -- -D warnings"
    done
    # The dist-only self-update code is feature-gated, so lint it explicitly too.
    cargo clippy --workspace --no-deps --features geotrace/self-update --tests -- -D warnings

[group("native")]
test *ARGS:
    cargo nextest run --workspace --features geotrace/self-update {{ ARGS }}

[group("native")]
test-all-backends:
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

# Build all workspace docs, failing on broken intra-doc links.
[group("native")]
doc:
    #!/usr/bin/env bash
    set -euo pipefail
    export RUSTDOCFLAGS="-D warnings"
    just _cargo-in-every-workspace doc "--workspace --no-deps"

[group("ci")]
ci: build-images ci-essentials ci-extras ci-sdks

[group("ci")]
ci-essentials: fmt-check clippy check test examples qa::qa-lint qa::test qa::check-all qa::check-versions qa::check-app doc check-unit-bindings osv-scanner sort-check shear typos

[group("ci")]
ci-extras: msrv sdk-msrv sdk-doc lychee

[group("ci")]
ci-sdks: python-sdk build-c fmt-c lint-c test-c fmt-cpp lint-cpp test-cpp qa::generate-gold test-gold-all
