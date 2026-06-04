#!/usr/bin/env bash
set -euo pipefail

mapfile -t FILES < <(find sdk/cpp -name "*.cpp" -o -name "*.hpp" | grep -v '/build/')
clang-format -i "${FILES[@]}"
