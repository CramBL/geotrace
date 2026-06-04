#!/usr/bin/env bash
set -euo pipefail
cargo test -p gt-arch
cargo pup check
