#!/usr/bin/env bash
set -euo pipefail
C_INSTALL=/tmp/gtd-sdk-asan
cmake -S sdk/c -B sdk/c/build/asan-for-cpp -GNinja \
      -DCMAKE_BUILD_TYPE=RelWithDebInfo -DCMAKE_INSTALL_PREFIX="$C_INSTALL"
cmake --build sdk/c/build/asan-for-cpp
cmake --install sdk/c/build/asan-for-cpp
cmake -S sdk/cpp -B sdk/cpp/build/asan -GNinja \
      -DCMAKE_BUILD_TYPE=RelWithDebInfo -DBUILD_TESTING=ON \
      -DCMAKE_CXX_COMPILER=clang++ \
      -DGEOTRACE_CPP_BUILD_EXAMPLES=ON \
      -DCMAKE_CXX_STANDARD=17 \
      -DGEOTRACE_SANITIZE=asan \
      -DGeoTraceC_DIR="$C_INSTALL/lib/cmake/GeoTraceC"
cmake --build sdk/cpp/build/asan
ctest --test-dir sdk/cpp/build/asan --output-on-failure
