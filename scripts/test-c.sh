#!/usr/bin/env bash
set -euo pipefail
cmake -S sdk/c -B sdk/c/build/test -GNinja \
      -DCMAKE_BUILD_TYPE=Debug -DGEOTRACE_C_BUILD_TESTS=ON
cmake --build sdk/c/build/test
ctest --test-dir sdk/c/build/test --output-on-failure
