"""Speed conversion tests: each function returns the float the Rust SDK computes."""

from __future__ import annotations

from collections.abc import Callable

import pytest
from geotrace_sdk import kmh_from_mps, knots_from_mps, mps_from_kmh, mps_from_knots


# `23.2 / 3.6` is 6.444444444444444, `13.0 * 1852 / 3600` is 6.687777777777778,
# `6.444444444444445 * 3.6` is 23.200000000000003 and `6.687777777777779 * 3600 / 1852`
# is 13.000000000000002.
@pytest.mark.parametrize(
    ("convert", "speed", "expected"),
    [
        (mps_from_kmh, 23.2, 6.444444444444445),
        (mps_from_knots, 13.0, 6.687777777777779),
        (kmh_from_mps, 6.444444444444445, 23.2),
        (knots_from_mps, 6.687777777777779, 13.0),
    ],
)
def test_a_speed_converts_to_the_same_float_as_in_the_rust_sdk(
    convert: Callable[[float], float], speed: float, expected: float
) -> None:
    assert convert(speed) == expected
