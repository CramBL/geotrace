#!/usr/bin/env bash
set -euo pipefail

# GEOTRACE_C_LIB_DIR may be set by CI to point at a pre-built cargo output dir.
# When unset the CMakeLists.txt falls back to sdk/rust/geotrace-c/target/release.
LIB_ARG=${GEOTRACE_C_LIB_DIR:+-DGEOTRACE_C_LIB_DIR="$GEOTRACE_C_LIB_DIR"}

# Install the C SDK to a temp prefix so find_package(GeoTraceC) works for C++.
INSTALL_PREFIX=$(mktemp -d)
trap 'rm -rf "$INSTALL_PREFIX"' EXIT

cmake -S sdk/c -B sdk/c/build/lint-c-for-cpp -GNinja \
    -DCMAKE_BUILD_TYPE=Debug \
    -DCMAKE_INSTALL_PREFIX="$INSTALL_PREFIX" \
    $LIB_ARG
cmake --install sdk/c/build/lint-c-for-cpp

cmake -S sdk/cpp -B sdk/cpp/build/lint -GNinja \
    -DCMAKE_BUILD_TYPE=Debug \
    -DBUILD_TESTING=ON \
    -DGEOTRACE_CPP_BUILD_EXAMPLES=ON \
    -DCMAKE_PREFIX_PATH="$INSTALL_PREFIX"

echo "==> clang-format (C++ SDK)"
find sdk/cpp \( -name "*.cpp" -o -name "*.hpp" \) -not -path '*/build/*' \
    | sort | xargs clang-format --dry-run --Werror

echo "==> clang-tidy (C++ SDK)"
find sdk/cpp -name "*.cpp" -not -path '*/build/*' \
    | sort | xargs clang-tidy -p sdk/cpp/build/lint
