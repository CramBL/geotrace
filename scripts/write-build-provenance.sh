#!/usr/bin/env bash
# Writes the commit hash and committer timestamp that build.rs embeds in the SDK
# build. The output file stays out of .gitignore: cargo drops ignored files from
# the package, and a published crate has to carry it.
set -euo pipefail

out="sdk/rust/geotrace-sdk/build_provenance.txt"
git rev-parse HEAD > "$out"
TZ=UTC git show --no-patch --format=%cd --date=iso-strict-local HEAD >> "$out"
cat "$out"
