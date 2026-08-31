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

# cargo-msrv is intentionally NOT installed here: it downloads many toolchains
# at runtime and would bloat the image, so the `msrv` and `sdk-msrv` recipes
# run natively (see scripts/container-rust.just) with the tool installed via
# cached CI actions instead.

# Python package manager used by the Python SDK.
RUN curl -LsSf https://astral.sh/uv/install.sh | UV_INSTALL_DIR=/usr/local/bin sh

# Install just, cargo-nextest, and extra lint tools via cargo-binstall where
# available for speed.
# cargo-binstall itself is bootstrapped via the official install script.
RUN curl -L --proto '=https' --tlsv1.2 -sSf \
    https://raw.githubusercontent.com/cargo-bins/cargo-binstall/main/install-from-binstall-release.sh \
    | bash

# --locked pins the `cargo install` source fallback to each crate's shipped
# Cargo.lock, whose dependency versions are known to compile.
# Binstall still finds a prebuilt binary when GitHub is slow:
# --maximum-resolution-timeout raises its 15s default to 60s.
RUN cargo binstall --no-confirm --locked --maximum-resolution-timeout 60 \
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

# cmake is absent from the package list below because the Rust dev stage already
# installs it (the default history backend builds libhdf5).
#
# clang-format 14, the version bookworm packages, reformats
# sdk/cpp/include/geotrace/geotrace.hpp: it ignores
# `AllowShortFunctionsOnASingleLine: Inline` for a function body inside the
# `#else` branch of a preprocessor conditional. This major matches the one
# .github/workflows/ci_sdk.yml pins as LLVM_CLANG_MAJOR.
ENV LLVM_VERSION=22
RUN curl -fsSL https://apt.llvm.org/llvm-snapshot.gpg.key \
      -o /usr/share/keyrings/apt-llvm-org.asc \
    && echo "deb [signed-by=/usr/share/keyrings/apt-llvm-org.asc] https://apt.llvm.org/bookworm/ llvm-toolchain-bookworm-${LLVM_VERSION} main" \
       > /etc/apt/sources.list.d/apt-llvm-org.list \
    && apt-get update && apt-get install -y --no-install-recommends \
    "clang-${LLVM_VERSION}" \
    "clang-format-${LLVM_VERSION}" \
    "clang-tidy-${LLVM_VERSION}" \
    libcriterion-dev \
    ninja-build \
    tar \
    unzip \
    zip \
    && rm -rf /var/lib/apt/lists/*

# apt.llvm.org installs only versioned binary names, which the lint and format
# scripts do not use.
RUN set -e; for tool in clang clang++ clang-format clang-tidy; do \
        update-alternatives --install "/usr/bin/${tool}" "${tool}" \
            "/usr/bin/${tool}-${LLVM_VERSION}" 100; \
    done

RUN git clone --depth=1 https://github.com/microsoft/vcpkg /opt/vcpkg \
    && /opt/vcpkg/bootstrap-vcpkg.sh -disableMetrics
ENV VCPKG_ROOT=/opt/vcpkg
ENV PATH="${PATH}:/opt/vcpkg"

# Ensure non-root users can write to vcpkg buildtrees, downloads, and packages at runtime
RUN chmod -R a+rwX /opt/vcpkg
