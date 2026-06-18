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
    curl \
    git \
    libssl-dev \
    pkg-config \
    python3 \
    && rm -rf /var/lib/apt/lists/*

# Debian bookworm ships CMake 3.25, but vcpkg (cloned at HEAD in stage 2) now
# requires >= 3.26. Install a pinned modern 3.x release from Kitware instead.
# Staying on the 3.x line avoids CMake 4.0 rejecting the bundled libhdf5's old
# `cmake_minimum_required`. /usr/local/bin precedes /usr/bin on PATH.
ENV CMAKE_VERSION=3.31.12
RUN arch="$(uname -m)" \
    && curl -fsSL "https://github.com/Kitware/CMake/releases/download/v${CMAKE_VERSION}/cmake-${CMAKE_VERSION}-linux-${arch}.tar.gz" \
       | tar -xz --strip-components=1 -C /usr/local \
    && cmake --version

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

# cargo-pup (architecture lints) and cargo-msrv are intentionally NOT installed
# here. cargo-pup needs a pinned nightly with rustc-dev/llvm-tools and is
# compiled from source; cargo-msrv downloads many toolchains at runtime. Both
# bloat the image, so the `pup`, `msrv`, and `sdk-msrv` recipes run natively
# (see scripts/container-rust.just) with the tools installed via cached CI
# actions instead.

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
    cargo-nextest \
    typos-cli \
    cargo-sort \
    cargo-shear \
    zizmor

# Make the Rust toolchain and installed binaries writable by any user so that
# containers can be run with --user $(id -u):$(id -g).
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
