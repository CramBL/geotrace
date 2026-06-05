#!/usr/bin/env bash
set -euo pipefail

for example in sdk/rust/geotrace-sdk/examples/*.rs; do
    name=$(basename "$example" .rs)
    echo "── ${name} ──"
    cargo run -p geotrace-sdk --example "${name}"
done
