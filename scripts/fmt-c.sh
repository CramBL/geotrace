#!/usr/bin/env bash
set -euo pipefail

mapfile -t FILES < <(find sdk/c -name "*.c" -o -name "*.h" | grep -v '/build/')
clang-format -i "${FILES[@]}"
