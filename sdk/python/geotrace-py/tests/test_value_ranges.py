"""Value-range tests: a coordinate or heading outside its range reads back unchanged."""

from __future__ import annotations

import math
from pathlib import Path

from geotrace_sdk import NavFile

FIXTURE = (
    Path(__file__).resolve().parents[3]
    / "c"
    / "tests"
    / "fixtures"
    / "out_of_range_values.gtd"
)


def test_out_of_range_coordinates_read_verbatim() -> None:
    points = NavFile.open(FIXTURE).points
    assert len(points) == 4

    assert math.isnan(points[0].lat)
    assert points[1].lat == 91.0
    assert points[2].lon == -181.0
    assert points[3].heading == 675.0
