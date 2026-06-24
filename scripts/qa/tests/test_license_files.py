"""The SDK LICENSE files must be real files, not symlinks.

A symlinked LICENSE makes maturin record `License-File: LICENSE` in the sdist
metadata while shipping only a dangling symlink, which PyPI rejects at upload.
"""

import pytest

from qa._check import repo_root

_CANONICAL = "sdk/rust/geotrace-sdk/LICENSE"
_DISTRIBUTED = [
    "sdk/python/geotrace-py/LICENSE",
    "sdk/c/LICENSE",
    "sdk/cpp/LICENSE",
]


@pytest.mark.parametrize("rel", _DISTRIBUTED)
def test_distributed_license_is_a_real_file(rel: str) -> None:
    path = repo_root() / rel
    assert not path.is_symlink(), f"{rel} is a symlink; package it as a real file"
    assert path.is_file(), f"{rel} is missing"


def test_distributed_licenses_match_the_canonical_text() -> None:
    root = repo_root()
    canonical = (root / _CANONICAL).read_text(encoding="utf-8")
    for rel in _DISTRIBUTED:
        text = (root / rel).read_text(encoding="utf-8")
        assert text == canonical, f"{rel} drifted from {_CANONICAL}"
