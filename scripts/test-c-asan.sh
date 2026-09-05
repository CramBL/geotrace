#!/usr/bin/env bash
set -euo pipefail
cmake -S sdk/c -B sdk/c/build/asan -GNinja \
      -DCMAKE_BUILD_TYPE=RelWithDebInfo -DCMAKE_C_COMPILER=clang \
      -DGEOTRACE_C_BUILD_TESTS=ON -DGEOTRACE_C_BUILD_EXAMPLES=ON \
      -DGEOTRACE_SANITIZE=asan
cmake --build sdk/c/build/asan
ctest --test-dir sdk/c/build/asan --output-on-failure
