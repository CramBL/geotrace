#!/usr/bin/env bash
set -euo pipefail

# Build and run the C SDK gold dataset example from the repository root.
# The C SDK build is placed in a user-writable directory so it does not
# conflict with container-owned build directories under sdk/c/build/.

REPO_ROOT=$(cd "$(dirname "$0")/.." && pwd)
BUILD_DIR="${TMPDIR:-/tmp}/gtd_gold_c_build"

cargo build -p geotrace-c

LIB_DIR="$REPO_ROOT/target/debug"
cmake -G "Unix Makefiles" \
    -S "$REPO_ROOT/sdk/c" \
    -B "$BUILD_DIR" \
    -DCMAKE_BUILD_TYPE=Debug \
    -DGEOTRACE_C_BUILD_EXAMPLES=ON \
    -DGEOTRACE_C_LIB_DIR="$LIB_DIR"

cmake --build "$BUILD_DIR" --target gold_dataset

cd "$REPO_ROOT"
"$BUILD_DIR/examples/gold_dataset"
