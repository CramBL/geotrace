#!/usr/bin/env bash
set -euo pipefail

# Build the Python extension and run the gold dataset example from the
# repository root, mirroring gold-c.sh / gold-cpp.sh for the other SDKs.

REPO_ROOT=$(cd "$(dirname "$0")/.." && pwd)
cd "$REPO_ROOT/sdk/python/geotrace-py"

uv sync --python 3.13 --reinstall-package geotrace-sdk
uv run python examples/gold_dataset.py
