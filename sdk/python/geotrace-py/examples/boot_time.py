#!/usr/bin/env python3
"""Record synthetic Linux CLOCK_BOOTTIME readings, which include suspend time."""

from datetime import UTC, datetime, timedelta

from geotrace_sdk import NavFile, NavFileBuilder, NavFix

START = datetime(2024, 6, 1, tzinfo=UTC)
MICROSECOND = timedelta(microseconds=1)
MICROSECONDS_PER_MILLISECOND = 1_000
SYSTEM_ADVANCEMENTS_US = [0, 1_000_001, 2_000_000]
BOOT_READINGS_US = [60_000_000, 61_000_000, 62_000_000]

builder = NavFileBuilder()
for system_us, boot_us in zip(SYSTEM_ADVANCEMENTS_US, BOOT_READINGS_US, strict=True):
    builder.add(
        NavFix(0, 0, sys_time=START + system_us * MICROSECOND, boot_time_us=boot_us)
    )
loaded = NavFile.from_bytes(builder.finish().to_bytes())
reference = loaded.points[0]
assert reference.sys_time is not None
assert reference.boot_time_us is not None
for point in loaded.points:
    assert point.sys_time is not None
    assert point.boot_time_us is not None
    system_elapsed_us = (point.sys_time - reference.sys_time) // MICROSECOND
    boot_elapsed_us = point.boot_time_us - reference.boot_time_us
    delta_us = system_elapsed_us - boot_elapsed_us
    delta_ms = delta_us / MICROSECONDS_PER_MILLISECOND
    print(f"Boot {point.boot_time_us}us, system/boot delta {delta_ms:+.3f}ms")
