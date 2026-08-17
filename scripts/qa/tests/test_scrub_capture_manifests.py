"""Tests for `qa.scrub_capture_manifests`: normalizing capture timestamps."""

import json
from pathlib import Path

import pytest

from qa.scrub_capture_manifests import SCRUBBED_VALUE, scrub_manifest

_NESTED = {
    "windows": [
        {"name": "kp-quiet", "captured_at": "2026-08-16T14:47:18.115465776+00:00", "samples": 9},
        {"name": "kp-storm", "captured_at": "2026-08-16T14:47:20.156744191+00:00", "samples": 17},
    ]
}


def _write(tmp_path: Path, manifest: dict[str, object]) -> Path:
    path = tmp_path / "capture.json"
    path.write_text(f"{json.dumps(manifest, indent=2)}\n", encoding="utf-8")
    return path


def test_every_nested_timestamp_is_scrubbed_and_the_rest_is_kept(tmp_path: Path) -> None:
    path = _write(tmp_path, _NESTED)

    assert scrub_manifest(path) == 2

    windows = json.loads(path.read_text(encoding="utf-8"))["windows"]
    assert [window["captured_at"] for window in windows] == [SCRUBBED_VALUE, SCRUBBED_VALUE]
    assert [window["samples"] for window in windows] == [9, 17]


def test_a_manifest_without_a_timestamp_fails(tmp_path: Path) -> None:
    path = _write(tmp_path, {"windows": [{"name": "kp-quiet", "samples": 9}]})

    with pytest.raises(ValueError, match="capture manifest schema changed"):
        scrub_manifest(path)


def test_scrubbing_twice_leaves_the_file_byte_identical(tmp_path: Path) -> None:
    path = _write(tmp_path, _NESTED)

    scrub_manifest(path)
    once = path.read_bytes()
    scrub_manifest(path)

    assert path.read_bytes() == once
