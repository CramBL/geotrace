#!/usr/bin/env bash
set -euo pipefail
cmake -S sdk/c -B sdk/c/build/asan -GNinja \
      -DCMAKE_BUILD_TYPE=Debug -DBUILD_TESTING=ON \
      -DGEOTRACE_SANITIZE=asan
cmake --build sdk/c/build/asan
env ASAN_OPTIONS=detect_leaks=1 UBSAN_OPTIONS=print_stacktrace=1 \
    ctest --test-dir sdk/c/build/asan --output-on-failure
