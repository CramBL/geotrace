#!/usr/bin/env bash
set -euo pipefail
cd sdk/python/geotrace-py
uv sync --python 3.13 --reinstall-package geotrace-sdk
uv run pytest tests/ -v
for example in examples/*.py; do
    echo "── ${example} ──"
    uv run python "${example}"
done
