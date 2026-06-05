#!/usr/bin/env bash
set -euo pipefail

# Build and run the C++ SDK gold dataset example from the repository root.

REPO_ROOT=$(cd "$(dirname "$0")/.." && pwd)
C_INSTALL="${TMPDIR:-/tmp}/gtd_gold_c_sdk"
C_BUILD="${TMPDIR:-/tmp}/gtd_gold_cpp_c_build"
CPP_BUILD="${TMPDIR:-/tmp}/gtd_gold_cpp_build"

cargo build -p geotrace-c

LIB_DIR="$REPO_ROOT/target/debug"

# Install the C SDK to a stable prefix.
cmake -G "Unix Makefiles" \
    -S "$REPO_ROOT/sdk/c" \
    -B "$C_BUILD" \
    -DCMAKE_BUILD_TYPE=Debug \
    -DCMAKE_INSTALL_PREFIX="$C_INSTALL" \
    -DGEOTRACE_C_LIB_DIR="$LIB_DIR"
cmake --install "$C_BUILD"

# If the C++ cmake cache references a different library, start fresh.
if [ -f "$CPP_BUILD/CMakeCache.txt" ]; then
    if ! grep -q "GEOTRACE_C_LIBRARY:FILEPATH=$C_INSTALL" "$CPP_BUILD/CMakeCache.txt"; then
        rm -rf "$CPP_BUILD"
    fi
fi

cmake -G "Unix Makefiles" \
    -S "$REPO_ROOT/sdk/cpp" \
    -B "$CPP_BUILD" \
    -DCMAKE_BUILD_TYPE=Debug \
    -DGEOTRACE_CPP_BUILD_EXAMPLES=ON \
    -DCMAKE_PREFIX_PATH="$C_INSTALL"

cmake --build "$CPP_BUILD" --target gold_dataset

LD_LIBRARY_PATH="$C_INSTALL/lib${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}" \
    "$CPP_BUILD/examples/gold_dataset"
