#!/usr/bin/env bash
set -euo pipefail
PREFIX=/tmp/gtd-sdk
cmake -S sdk/c   -B sdk/c/build/install   -GNinja \
      -DCMAKE_BUILD_TYPE=Release -DCMAKE_INSTALL_PREFIX="$PREFIX"
cmake --build  sdk/c/build/install
cmake --install sdk/c/build/install
cmake -S sdk/cpp -B sdk/cpp/build/install -GNinja \
      -DCMAKE_BUILD_TYPE=Release -DCMAKE_INSTALL_PREFIX="$PREFIX" \
      -DGeoTraceC_DIR="$PREFIX/lib/cmake/GeoTraceC"
cmake --build  sdk/cpp/build/install
cmake --install sdk/cpp/build/install
cmake -S sdk/integration-tests/cmake-find-package \
      -B sdk/integration-tests/cmake-find-package/build -GNinja \
      -DCMAKE_BUILD_TYPE=Release \
      -DCMAKE_PREFIX_PATH="$PREFIX"
cmake --build sdk/integration-tests/cmake-find-package/build
sdk/integration-tests/cmake-find-package/build/smoke_c
sdk/integration-tests/cmake-find-package/build/smoke_cpp
pkg-config --cflags --libs geotrace-c \
    || echo "pkg-config check skipped - geotrace-c.pc not on PKG_CONFIG_PATH"
