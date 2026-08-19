"""Write the `.Z` fixtures `gt_ionex::unix_compress`'s decoder tests read.

Every stream is run through the reference encoder in `qa.unix_compress` and
checked against `gzip -d` before it is written, so a committed fixture is one
an independent implementation reads back as the bytes it stands for.

The four cover the stream shapes CDDIS serves and the ones a decoder gets
wrong: the default 16-bit block-mode stream over a whole IONEX capture, a
12-bit one whose table fills and restarts several times, one without block
mode, where no restart code exists, and a repeated byte expanding past the
decoder's output limit.
"""

import sys
from collections.abc import Callable
from dataclasses import dataclass
from pathlib import Path

from qa._check import repo_root
from qa.unix_compress import compress, decompressed_by_gzip

FIXTURE_DIR = Path("crates/gt-ionex/tests/fixtures")

OUTPUT_DIR = FIXTURE_DIR / "unix_compress"

# The capture the file-derived fixtures hold, and how much of it the partial
# ones do. The Rust tests slice the same capture to this.
CAPTURE = "JPLG0920.24I"
HEAD_BYTES = 65536

# Longer than `gt_ionex::unix_compress::MAX_DECOMPRESSED_BYTES`, which the
# decoder stops at.
PAST_OUTPUT_LIMIT_BYTES = 64 * 1024 * 1024 + 1024


@dataclass(frozen=True)
class _Fixture:
    output: str
    max_code_width: int
    block_mode: bool
    data: Callable[[Path], bytes]


def _capture(root: Path) -> bytes:
    return (root / FIXTURE_DIR / CAPTURE).read_bytes()


def _capture_head(root: Path) -> bytes:
    return _capture(root)[:HEAD_BYTES]


def _run_past_the_output_limit(_root: Path) -> bytes:
    return b"a" * PAST_OUTPUT_LIMIT_BYTES


_FIXTURES = (
    _Fixture(
        output=f"{CAPTURE}.Z",
        max_code_width=16,
        block_mode=True,
        data=_capture,
    ),
    _Fixture(
        output=f"{CAPTURE}.head.12bit.Z",
        max_code_width=12,
        block_mode=True,
        data=_capture_head,
    ),
    _Fixture(
        output=f"{CAPTURE}.head.no-block.Z",
        max_code_width=16,
        block_mode=False,
        data=_capture_head,
    ),
    _Fixture(
        output="past-output-limit.Z",
        max_code_width=16,
        block_mode=True,
        data=_run_past_the_output_limit,
    ),
)


def _write(root: Path, fixture: _Fixture) -> None:
    data = fixture.data(root)
    compressed = compress(data, fixture.max_code_width, fixture.block_mode)

    reference = decompressed_by_gzip(compressed)
    if reference != data:
        raise ValueError(
            f"{fixture.output}: gzip read back {len(reference)} bytes of the {len(data)} "
            "encoded, so what this encoder writes is not the format"
        )

    path = root / OUTPUT_DIR / fixture.output
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_bytes(compressed)
    print(f"{path}: {len(data)} bytes as {len(compressed)}, gzip agrees")


def main() -> None:
    root = repo_root()
    failed = False
    for fixture in _FIXTURES:
        try:
            _write(root, fixture)
        except (OSError, ValueError) as error:
            print(f"error: {error}", file=sys.stderr)
            failed = True

    if failed:
        sys.exit(1)
