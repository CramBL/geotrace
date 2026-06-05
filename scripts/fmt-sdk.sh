#!/usr/bin/env bash
set -euo pipefail
find sdk/c sdk/cpp \( -name "*.c" -o -name "*.h" -o -name "*.cpp" -o -name "*.hpp" \) \
    -not -path "*/build/*" | sort | xargs clang-format -i
