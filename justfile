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

[group("native")]
test *ARGS:
    cargo nextest run --workspace {{ ARGS }}

[group("native")]
examples:
    bash scripts/examples.sh

# Generate the minimal.gtd fixture used by the C SDK tests.
[group("native")]
gen-fixture:
    cargo run -p geotrace-c --bin gen_fixture

[group("native")]
sdk-doc:
    RUSTDOCFLAGS="-D warnings" cargo doc -p geotrace-sdk --no-deps

[group("test-gold")]
generate-gold:
    just qa::generate-gold

[group("test-gold")]
test-gold:
    just qa::test-gold

[group("utils")]
setup-pup:
    rustup component add --toolchain nightly-2026-01-22 rust-src rustc-dev llvm-tools-preview
    cargo +nightly-2026-01-22 install cargo_pup

[group("ci")]
ci: build-images ci-essentials ci-extras python-sdk osv-scanner

[group("ci")]
ci-essentials: fmt-check clippy check test examples qa::qa-lint qa::check-em-dash qa::check-floating-comments

[group("ci")]
ci-extras: sort-check shear typos pup msrv sdk-msrv sdk-doc
