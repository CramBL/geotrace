alias c := check
alias b := build
alias l := clippy
alias t := test
alias r := run
alias cie := ci-essentials
alias cix := ci-extras

import 'container.just'
import 'scripts/container-rust.just'
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

[group("native")]
fmt:
    cargo fmt --all

[group("native")]
fmt-check:
    cargo fmt --all --check

[group("native")]
check *ARGS:
    cargo check {{ ARGS }}

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

[group("native")]
test *ARGS:
    GEOTRACE_OFFLINE=1 cargo nextest run --workspace --features geotrace/self-update {{ ARGS }}

[group("native")]
test-all-backends:
    @echo "Running tests for pure backend"
    GEOTRACE_OFFLINE=1 cargo nextest run -p gt-history --test integration --no-default-features --features backend-pure
    @echo "Running tests for sys backend"
    GEOTRACE_OFFLINE=1 cargo nextest run -p gt-history --test integration --no-default-features --features backend-sys

[group("native")]
test-snapshots *ARGS:
    GEOTRACE_OFFLINE=1 cargo nextest run --workspace --features geotrace/self-update -E "test(snapshot)" {{ ARGS }}

[group("native")]
examples:
    bash scripts/examples.sh

# Generate the minimal.gtd fixture used by the C SDK tests.
[group("native")]
gen-fixture:
    cargo run -p geotrace-c --bin gen_fixture

# Refresh the gt-jam interference dataset fixtures from gpsjam.org (network!).
# Fixtures are frozen once committed - review the resulting diff like code.
# Name days to capture only those (additive); no args captures all.
[group("native")]
jam-fixtures *DAYS:
    cargo run -p gt-jam --example fetch_jam_fixtures -- {{ DAYS }}

# Refresh the gt-snap live-API fixtures from the Valhalla server (network!).
# Fixtures are frozen once committed - review the resulting diff like code.
# Name scenarios to capture only those (additive); no args captures all.
[group("native")]
snap-fixtures *SCENARIOS:
    cargo run -p gt-snap --example fetch_snap_fixtures -- {{ SCENARIOS }}

# Run the gt-snap live-API smoke test against the real Valhalla server
# (network!) - the on-demand drift check for the API boundary.
[group("native")]
snap-live-test:
    cargo nextest run --profile live -p gt-snap

[group("native")]
sdk-doc:
    RUSTDOCFLAGS="-D warnings" cargo doc -p geotrace-sdk --no-deps

# Build all workspace docs, failing on broken intra-doc links.
[group("native")]
doc:
    RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps

[group("utils")]
setup-pup:
    rustup component add --toolchain nightly-2026-01-22 rust-src rustc-dev llvm-tools-preview
    cargo +nightly-2026-01-22 install cargo_pup

[group("ci")]
ci: build-images ci-essentials ci-extras ci-sdks

[group("ci")]
ci-essentials: fmt-check clippy check test examples qa::qa-lint qa::test qa::check-all qa::check-versions qa::check-app doc check-unit-bindings

[group("ci")]
ci-extras: osv-scanner sort-check shear typos pup msrv sdk-msrv sdk-doc

[group("ci")]
ci-sdks: python-sdk build-c fmt-c lint-c test-c fmt-cpp lint-cpp test-cpp qa::generate-gold test-gold-all
