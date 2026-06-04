#!/usr/bin/env bash
set -euo pipefail
cmake -S sdk/cpp -B sdk/cpp/build/examples -GNinja \
      -DCMAKE_BUILD_TYPE=Release \
      -DGEOTRACE_CPP_BUILD_EXAMPLES=ON \
      -DGeoTraceC_DIR=sdk/c/build/examples
cmake --build sdk/cpp/build/examples
