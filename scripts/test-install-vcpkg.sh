#!/usr/bin/env bash
set -euo pipefail
vcpkg install geotrace-c \
    --overlay-ports=sdk/integration-tests/vcpkg-port \
    --triplet x64-linux
cmake -S sdk/integration-tests/vcpkg \
      -B sdk/integration-tests/vcpkg/build -GNinja \
      -DCMAKE_BUILD_TYPE=Release \
      -DCMAKE_TOOLCHAIN_FILE="$VCPKG_ROOT/scripts/buildsystems/vcpkg.cmake" \
      -DVCPKG_TARGET_TRIPLET=x64-linux
cmake --build sdk/integration-tests/vcpkg/build
sdk/integration-tests/vcpkg/build/smoke_c
