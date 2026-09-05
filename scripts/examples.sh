#!/usr/bin/env bash
set -euo pipefail

for example in sdk/rust/geotrace-sdk/examples/*.rs; do
    name=$(basename "$example" .rs)
    echo "── ${name} ──"
    cargo run -p geotrace-sdk --example "${name}"
done

git diff --exit-code --stat -- tests/fixtures/demo_trip/demo_trip.gtd tests/fixtures/gold_dataset/gold.gtd \
    || { echo "error: an example rewrote a committed fixture, commit the new bytes" >&2; exit 1; }
