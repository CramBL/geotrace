# Dev toolchain image for geotrace.
#
# Run any just recipe through the container without re-installing tools:
#
#   docker run --rm \
#     -v "$HOME/.cargo/registry:/root/.cargo/registry" \
#     -v "$HOME/.cargo/git:/root/.cargo/git" \
#     -v "$(pwd):/workspace" \
#     -w /workspace \
#     geotrace-dev \
#     just check
#
# The two ~/.cargo mounts share the package registry and git cache between the
# host and the container.  Both use the same Rust version (pinned in
# rust-toolchain.toml), so the compiled artifacts in target/ are compatible
# and can be shared too:
#
#   -v "$(pwd)/target:/workspace/target"
#
# Note: the container does not include GPU drivers, so tests that require a
# hardware renderer (egui/wgpu snapshot tests) must run natively or with a CI
# software renderer (LIBGL_ALWAYS_SOFTWARE=1 + WGPU_BACKEND=gl on Linux).

FROM debian:bookworm-slim AS base

ENV DEBIAN_FRONTEND=noninteractive

RUN apt-get update && apt-get install -y --no-install-recommends \
    build-essential \
    ca-certificates \
    curl \
    git \
    pkg-config \
    libssl-dev \
    python3 \
    python3-pip \
    && rm -rf /var/lib/apt/lists/*

# Install rustup without pinning a toolchain version here.
# rust-toolchain.toml is the single source of truth for the Rust version.
RUN curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs \
    | sh -s -- -y --default-toolchain none --no-modify-path
ENV PATH="/root/.cargo/bin:${PATH}"

# Copy the toolchain pin before any Rust commands so rustup installs the
# version declared in rust-toolchain.toml rather than whatever "stable"
# resolves to today.
WORKDIR /workspace
COPY rust-toolchain.toml ./
RUN rustup show

# Install the nightly toolchain required by cargo-pup.
# The exact version is pinned in justfile / pup.ron.
RUN rustup toolchain install nightly-2026-01-22 \
    --component rust-src,rustc-dev,llvm-tools-preview

# Install uv (Python package manager used by the Python SDK)
RUN curl -LsSf https://astral.sh/uv/install.sh | sh
ENV PATH="/root/.local/bin:${PATH}"

# Install just, cargo-nextest, and extra lint tools via cargo-binstall where
# available for speed; fall back to cargo install for the rest.
# cargo-binstall itself is bootstrapped via the official install script.
RUN curl -L --proto '=https' --tlsv1.2 -sSf \
    https://raw.githubusercontent.com/cargo-bins/cargo-binstall/main/install-from-binstall-release.sh \
    | bash

RUN cargo binstall --no-confirm \
    just \
    cargo-msrv \
    cargo-nextest \
    typos-cli \
    cargo-sort \
    cargo-shear \
    zizmor

# cargo-pup must be built against the nightly compiler; there is no pre-built
# binary distribution, so we compile it from source with the pinned nightly.
RUN cargo +nightly-2026-01-22 install cargo_pup

# Warm up the Rust compiler cache for the workspace dependencies.
# This layer is intentionally placed last so that changes to source files
# do not bust the tool-installation layers above.
COPY Cargo.toml Cargo.lock ./
COPY crates/ crates/
COPY geotrace-sdks/ geotrace-sdks/
COPY src/ src/

RUN cargo fetch
