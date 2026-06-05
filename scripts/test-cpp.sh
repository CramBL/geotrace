#!/usr/bin/env bash
set -euo pipefail
C_INSTALL=/tmp/gtd-sdk-test
cmake -S sdk/c -B sdk/c/build/test-for-cpp -GNinja \
      -DCMAKE_BUILD_TYPE=Debug -DCMAKE_INSTALL_PREFIX="$C_INSTALL"
cmake --build sdk/c/build/test-for-cpp
cmake --install sdk/c/build/test-for-cpp
cmake -S sdk/cpp -B sdk/cpp/build/test -GNinja \
      -DCMAKE_BUILD_TYPE=Debug -DBUILD_TESTING=ON \
      -DCMAKE_CXX_STANDARD=17 \
      -DGeoTraceC_DIR="$C_INSTALL/lib/cmake/GeoTraceC"
cmake --build sdk/cpp/build/test
ctest --test-dir sdk/cpp/build/test --output-on-failure
