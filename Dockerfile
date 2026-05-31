# Dev toolchain image for geotrace.
#
# Run any just recipe through the container without re-installing tools:
#
#   docker run --rm \
#     -v "$HOME/.cargo/registry:/usr/local/cargo/registry" \
#     -v "$HOME/.cargo/git:/usr/local/cargo/git" \
#     -v "$(pwd):/workspace" \
#     -w /workspace \
#     geotrace-dev \
#     just check
#
# The two ~/.cargo mounts share the package registry and git cache between the
# host and the container.  As long as both use the same stable toolchain version
# the compiled artifacts in target/ are compatible, so the target/ directory can
# also be bind-mounted to share incremental build state:
#
#   -v "$(pwd)/target:/workspace/target"
#
# Note: the container does not include GPU drivers, so tests that require a
# hardware renderer (egui/wgpu snapshot tests) must run natively or with a CI
# software renderer (LIBGL_ALWAYS_SOFTWARE=1 + WGPU_BACKEND=gl on Linux).

FROM debian:bookworm-slim AS base

ENV DEBIAN_FRONTEND=noninteractive

RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates \
    curl \
    git \
    pkg-config \
    libssl-dev \
    python3 \
    python3-pip \
    && rm -rf /var/lib/apt/lists/*

# Install Rust stable via rustup
RUN curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs \
    | sh -s -- -y --default-toolchain stable --no-modify-path
ENV PATH="/root/.cargo/bin:${PATH}"

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
WORKDIR /workspace
COPY Cargo.toml Cargo.lock ./
COPY crates/ crates/
COPY geotrace-sdks/ geotrace-sdks/
COPY src/ src/

RUN cargo fetch
