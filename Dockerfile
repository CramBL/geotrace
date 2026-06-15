# Stage 1  geotrace-dev     Rust toolchain + CI lint tools + cmake (the default
#                           history backend builds libhdf5 from source).
#                           Used by: just check / clippy / test / ci-extras / ...
#
# Stage 2  geotrace-sdk-dev Extends stage 1 with clang, criterion, vcpkg.
#                           Used by: just test-c / test-cpp / test-install / ...

### Stage 1: Rust dev ###
FROM debian:bookworm-slim AS rust-dev

ENV DEBIAN_FRONTEND=noninteractive
RUN apt-get update && apt-get install -y --no-install-recommends \
    build-essential \
    ca-certificates \
    cmake \
    curl \
    git \
    libssl-dev \
    pkg-config \
    python3 \
    && rm -rf /var/lib/apt/lists/*

# Install rustup into a shared location so it's accessible by non-root users.
ENV RUSTUP_HOME=/usr/local/rustup \
    CARGO_HOME=/usr/local/cargo \
    PATH=/usr/local/cargo/bin:${PATH}

RUN curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs \
    | sh -s -- -y --default-toolchain none --no-modify-path

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

# Python package manager used by the Python SDK.
RUN curl -LsSf https://astral.sh/uv/install.sh | UV_INSTALL_DIR=/usr/local/bin sh

# Install just, cargo-nextest, and extra lint tools via cargo-binstall where
# available for speed.
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

# Make the Rust toolchain and installed binaries writable by any user so that
# containers can be run with --user $(id -u):$(id -g) and still allow tools
# like cargo-msrv to install temporary toolchains.
RUN chmod -R a+rwX "$CARGO_HOME" "$RUSTUP_HOME"

### Stage 2: C/C++ SDK dev ###
#
# Inherits everything from rust-dev and adds the tools needed to build, test,
# and package-manager-integrate the C and C++ SDKs.
FROM rust-dev AS sdk-dev

ENV DEBIAN_FRONTEND=noninteractive

# cmake is already provided by the Rust dev stage (the default history backend
# builds libhdf5), so it is not repeated here.
RUN apt-get update && apt-get install -y --no-install-recommends \
    clang \
    clang-format \
    clang-tidy \
    libcriterion-dev \
    ninja-build \
    tar \
    unzip \
    zip \
    && rm -rf /var/lib/apt/lists/*

RUN git clone --depth=1 https://github.com/microsoft/vcpkg /opt/vcpkg \
    && /opt/vcpkg/bootstrap-vcpkg.sh -disableMetrics
ENV VCPKG_ROOT=/opt/vcpkg
ENV PATH="${PATH}:/opt/vcpkg"

# Ensure non-root users can write to vcpkg buildtrees, downloads, and packages at runtime
RUN chmod -R a+rwX /opt/vcpkg
