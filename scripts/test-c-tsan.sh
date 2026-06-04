#!/usr/bin/env bash
set -euo pipefail
cmake -S sdk/c -B sdk/c/build/tsan -GNinja \
      -DCMAKE_BUILD_TYPE=Debug -DBUILD_TESTING=ON \
      -DGEOTRACE_SANITIZE=tsan
cmake --build sdk/c/build/tsan
env TSAN_OPTIONS=second_deadlock_stack=1 \
    ctest --test-dir sdk/c/build/tsan --output-on-failure
