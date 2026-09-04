"""Reference encoder for the Unix `compress` format (`.Z`).

`gt_ionex::unix_compress` decodes what CDDIS serves its legacy IONEX files in.
This module writes its fixtures, since no `compress` binary ships with the
development machines. Every generated stream is checked against `gzip -d`
before it is written, so a decoder test reads fixtures an independent
implementation agrees with.
"""

import subprocess

MAGIC = bytes((0x1F, 0x9D))
BLOCK_MODE_FLAG = 0x80
INITIAL_CODE_WIDTH = 9
CLEAR_CODE = 256
BYTE_CODES = 256


class _CodeWriter:
    """Packs codes low bit first, eight to a group of `width` bytes.

    The format's readers take the code width from the stream position, so a
    group is padded out whole before the width changes or the table restarts.
    """

    def __init__(self, out: bytearray, width: int) -> None:
        self._out = out
        self._width = width
        self._group = bytearray(width)
        self._offset_bits = 0

    def write(self, code: int) -> None:
        for bit in range(self._width):
            if (code >> bit) & 1:
                position = self._offset_bits + bit
                self._group[position // 8] |= 1 << (position % 8)
        self._offset_bits += self._width
        if self._offset_bits == self._width * 8:
            self._write_group(self._width)

    def restart_at(self, width: int) -> None:
        """Pad the group out whole and continue at `width` bytes to a group."""
        if self._offset_bits:
            self._write_group(self._width)
        self._width = width
        self._group = bytearray(width)

    def end(self) -> None:
        """Write the bytes the last codes reach into, without padding.

        The reader stops where fewer bits than a code are left, so the last
        group is the one place the format writes a partial one.
        """
        if self._offset_bits:
            self._write_group((self._offset_bits + 7) // 8)

    def _write_group(self, length: int) -> None:
        self._out += self._group[:length]
        self._group = bytearray(self._width)
        self._offset_bits = 0


def compress(data: bytes, max_code_width: int = 16, block_mode: bool = True) -> bytes:
    """`data` as a compress stream, restarting the table once it fills."""
    if not 9 <= max_code_width <= 16:
        raise ValueError(f"{max_code_width} is outside the 9 to 16 bit code widths")

    out = bytearray(MAGIC)
    out.append(max_code_width | (BLOCK_MODE_FLAG if block_mode else 0))
    writer = _CodeWriter(out, INITIAL_CODE_WIDTH)
    first_code = CLEAR_CODE + 1 if block_mode else BYTE_CODES
    highest_code = (1 << max_code_width) - 1

    table: dict[tuple[int, int], int] = {}
    next_code = first_code
    code_width = INITIAL_CODE_WIDTH
    prefix: int | None = None

    for byte in data:
        if prefix is None:
            prefix = byte
            continue
        pair = (prefix, byte)
        found = table.get(pair)
        if found is not None:
            prefix = found
            continue

        writer.write(prefix)
        prefix = byte
        if next_code > highest_code:
            if block_mode:
                writer.write(CLEAR_CODE)
                writer.restart_at(INITIAL_CODE_WIDTH)
                table.clear()
                next_code = first_code
                code_width = INITIAL_CODE_WIDTH
            continue

        table[pair] = next_code
        next_code += 1
        # A reader adds each string one code later than this loop does, so the
        # width changes when the reader's table fills, one code behind.
        if next_code - 1 > (1 << code_width) - 1 and code_width < max_code_width:
            code_width += 1
            writer.restart_at(code_width)

    if prefix is not None:
        writer.write(prefix)
    writer.end()
    return bytes(out)


def decompressed_by_gzip(compressed: bytes) -> bytes:
    """What the system `gzip` reads the stream as, the independent reference."""
    return subprocess.run(
        ["gzip", "-dc"],
        input=compressed,
        capture_output=True,
        check=True,
    ).stdout
