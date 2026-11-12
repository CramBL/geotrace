from dataclasses import dataclass
from datetime import UTC, datetime, timedelta

import pytest
from geotrace_sdk import NavFile, NavFileBuilder, NavFix

START = datetime(2024, 6, 1, tzinfo=UTC)
MAX_BOOT_TIME_US = 2**64 - 2


@dataclass(frozen=True)
class Clocks:
    gps_time: datetime | None
    sys_time: datetime | None


@pytest.mark.parametrize(
    "clocks",
    [
        Clocks(gps_time=START, sys_time=START),
        Clocks(gps_time=None, sys_time=START),
        Clocks(gps_time=START, sys_time=None),
    ],
)
def test_boot_time_keeps_exact_values_with_each_absolute_clock(clocks: Clocks) -> None:
    readings = [0, None, 2**53 + 1, 2**53 + 1, 7, MAX_BOOT_TIME_US]
    builder = NavFileBuilder()
    for index in reversed(range(len(readings))):
        elapsed = timedelta(microseconds=index)
        fix = NavFix(
            lat=0,
            lon=0,
            gps_time=clocks.gps_time + elapsed if clocks.gps_time is not None else None,
            sys_time=clocks.sys_time + elapsed if clocks.sys_time is not None else None,
            boot_time_us=readings[index],
        )
        assert fix.boot_time_us == readings[index]
        builder.add(fix)

    loaded = NavFile.from_bytes(builder.finish().to_bytes())
    assert [point.boot_time_us for point in loaded.points] == readings
    rewritten = NavFile.from_bytes(loaded.to_bytes())
    assert [point.boot_time_us for point in rewritten.points] == readings


def test_omitted_boot_time_and_explicit_none_remain_absent() -> None:
    fixes = [
        NavFix(0, 0, sys_time=START),
        NavFix(0, 0, sys_time=START, boot_time_us=None),
    ]
    builder = NavFileBuilder()
    for fix in fixes:
        assert fix.boot_time_us is None
        builder.add(fix)
    loaded = NavFile.from_bytes(builder.finish().to_bytes())
    assert [point.boot_time_us for point in loaded.points] == [None, None]


@pytest.mark.parametrize(
    ("boot_time_us", "expected_error"),
    [
        (-1, OverflowError),
        (2**64 - 1, ValueError),
        (2**64, OverflowError),
        (2**128, OverflowError),
    ],
)
def test_boot_time_rejects_values_outside_the_representable_range(
    boot_time_us: int, expected_error: type[Exception]
) -> None:
    with pytest.raises(expected_error):
        NavFix(0, 0, sys_time=START, boot_time_us=boot_time_us)
