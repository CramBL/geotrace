#!/usr/bin/env bash
set -euo pipefail
cmake -S sdk/cpp -B sdk/cpp/build/test -GNinja \
      -DCMAKE_BUILD_TYPE=Debug -DBUILD_TESTING=ON \
      -DGeoTraceC_DIR=sdk/c/build/test
cmake --build sdk/cpp/build/test
ctest --test-dir sdk/cpp/build/test --output-on-failure
