# Dev toolchain image for geotrace — two named stages.
#
# Stage 1  geotrace-dev     Rust toolchain + CI lint tools.
#                           Used by: just check / clippy / test / ci-extras / …
#
# Stage 2  geotrace-sdk-dev Extends stage 1 with cmake, cmocka, vcpkg.
#                           Used by: just test-c / test-cpp / test-install / …
#
# Build only what you need:
#
#   just build-image          # builds geotrace-dev  (default — used by most devs)
#   just build-sdk-image      # builds geotrace-sdk-dev  (C/C++ SDK work)
#   just build-images         # builds both
#
# Run any just recipe through its container without installing tools locally:
#
#   just test-c               # auto-uses geotrace-sdk-dev
#   just dev-shell            # bash inside geotrace-dev
#   just dev-shell-sdk        # bash inside geotrace-sdk-dev
#
# Note: the container does not include GPU drivers, so snapshot tests that
# require a hardware renderer must run natively or with a software renderer
# (LIBGL_ALWAYS_SOFTWARE=1 + WGPU_BACKEND=gl on Linux).

# ── Stage 1: Rust dev ─────────────────────────────────────────────────────────

FROM debian:bookworm-slim AS rust-dev

ENV DEBIAN_FRONTEND=noninteractive

RUN apt-get update && apt-get install -y --no-install-recommends \
    build-essential \
    ca-certificates \
    curl \
    git \
    libssl-dev \
    pkg-config \
    python3 \
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

# uv — Python package manager used by the Python SDK.
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

# Make the Rust toolchain and installed binaries readable by any user so that
# containers can be run with --user $(id -u):$(id -g) without permission errors.
RUN chmod -R a+rX /root/.cargo /root/.rustup

# Warm up the Rust compiler cache for the workspace dependencies.
# This layer is intentionally placed last so that changes to source files
# do not bust the tool-installation layers above.
COPY Cargo.toml Cargo.lock ./
COPY crates/ crates/
COPY sdk/ sdk/
COPY src/ src/

RUN cargo fetch

# ── Stage 2: C/C++ SDK dev ────────────────────────────────────────────────────
#
# Inherits everything from rust-dev and adds the tools needed to build, test,
# and package-manager-integrate the C and C++ SDKs.
#
# These tools are intentionally kept in a separate stage so that developers
# working only on the Rust codebase do not need to pull or build them.
# Docker layer sharing means the rust-dev layers are cached and reused.

FROM rust-dev AS sdk-dev

ENV DEBIAN_FRONTEND=noninteractive

RUN apt-get update && apt-get install -y --no-install-recommends \
    clang \
    clang-format \
    clang-tidy \
    cmake \
    libcmocka-dev \
    ninja-build \
    tar \
    unzip \
    zip \
    && rm -rf /var/lib/apt/lists/*

# vcpkg — placed last because the git clone + bootstrap is slow and rarely
# changes; keeping it as a late layer avoids busting the apt layer above.
RUN git clone --depth=1 https://github.com/microsoft/vcpkg /opt/vcpkg \
    && /opt/vcpkg/bootstrap-vcpkg.sh -disableMetrics
ENV VCPKG_ROOT=/opt/vcpkg
ENV PATH="${PATH}:/opt/vcpkg"
