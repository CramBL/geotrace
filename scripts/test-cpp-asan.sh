#!/usr/bin/env bash
set -euo pipefail
cmake -S sdk/cpp -B sdk/cpp/build/asan -GNinja \
      -DCMAKE_BUILD_TYPE=Debug -DBUILD_TESTING=ON \
      -DGEOTRACE_SANITIZE=asan \
      -DGeoTraceC_DIR=sdk/c/build/asan
cmake --build sdk/cpp/build/asan
env ASAN_OPTIONS=detect_leaks=1 UBSAN_OPTIONS=print_stacktrace=1 \
    ctest --test-dir sdk/cpp/build/asan --output-on-failure
