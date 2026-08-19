"""The reference encoder writes streams the system `gzip` reads back.

`gt_ionex::unix_compress` is tested against fixtures this encoder wrote, so
what those fixtures stand for rests on an implementation nobody here wrote
agreeing with it.
"""

import pytest

from qa.unix_compress import compress, decompressed_by_gzip

# Enough of a repeated pattern to fill a 12-bit table several times over, and
# enough distinct bytes to grow the code width.
_REPEATED = b"the quick brown fox " * 4000
_COUNTING = bytes(range(256)) * 400


@pytest.mark.parametrize("max_code_width", [12, 16])
@pytest.mark.parametrize("block_mode", [True, False])
@pytest.mark.parametrize("data", [b"", b"a", _REPEATED, _COUNTING], ids=["empty", "one", "repeated", "counting"])
def test_gzip_reads_back_what_was_encoded(
    data: bytes, max_code_width: int, block_mode: bool
) -> None:
    compressed = compress(data, max_code_width, block_mode)
    assert decompressed_by_gzip(compressed) == data


def test_a_stream_starts_with_the_magic_and_its_flags() -> None:
    assert compress(b"a", 12, block_mode=True)[:3] == bytes((0x1F, 0x9D, 0x8C))


def test_a_code_width_outside_the_format_is_refused() -> None:
    with pytest.raises(ValueError):
        compress(b"a", 17)
