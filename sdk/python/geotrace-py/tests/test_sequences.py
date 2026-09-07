"""Sequence tests: the collections of a NavFile index, slice and iterate."""

from __future__ import annotations

import gc
import tempfile
from datetime import UTC, datetime, timedelta
from pathlib import Path

import pytest
from geotrace_sdk import (
    Annotation,
    Channel,
    EventMarker,
    EventMarkerStyle,
    MarkerIcon,
    NavFile,
    NavFileBuilder,
    NavFix,
    NavPointSequence,
)

T0 = datetime(2024, 6, 1, 9, 0, 0, tzinfo=UTC)
T1 = T0 + timedelta(seconds=30)
T2 = T0 + timedelta(seconds=60)

LATITUDES = [51.50, 51.51, 51.52]
LONGITUDES = [-0.10, -0.11, -0.12]


def _write_and_read(builder: NavFileBuilder) -> NavFile:
    with tempfile.NamedTemporaryFile(suffix=".gtd", delete=False) as f:
        path = f.name
    try:
        builder.finish().write_to_file(path)
        return NavFile.open(path)
    finally:
        Path(path).unlink()


def _three_fixes() -> NavFile:
    """A file whose first fix alone has a heading, a speed and an accuracy."""
    b = NavFileBuilder()
    b.add(
        NavFix(
            lat=LATITUDES[0],
            lon=LONGITUDES[0],
            gps_time=T0,
            heading=90.0,
            speed_mps=12.5,
            eph_m=3.25,
        )
    )
    b.add(NavFix(lat=LATITUDES[1], lon=LONGITUDES[1], gps_time=T1))
    b.add(NavFix(lat=LATITUDES[2], lon=LONGITUDES[2], sys_time=T2))
    return _write_and_read(b)


def _every_collection() -> NavFile:
    b = NavFileBuilder()
    for lat, lon, time in zip(LATITUDES, LONGITUDES, [T0, T1, T2], strict=True):
        b.add(NavFix(lat=lat, lon=lon, gps_time=time))
    b.add(Annotation(T1, label="Coffee stop", icon=MarkerIcon.PIN))
    b.add(EventMarker("power/boot", T1))
    b.add_event_marker_style(EventMarkerStyle("power/boot", color="#FF9900"))
    b.add(Channel("temp", [T0, T1], [20.0, 21.0]))
    return _write_and_read(b)


COLLECTION_NAMES = [
    "points",
    "markers",
    "event_markers",
    "channels",
    "event_marker_styles",
]


def test_len_is_the_fix_count() -> None:
    assert len(_three_fixes().points) == 3


def test_an_index_reads_the_point_at_that_position() -> None:
    assert _three_fixes().points[1].lat == pytest.approx(LATITUDES[1])


def test_a_negative_index_counts_from_the_end() -> None:
    assert _three_fixes().points[-1].lat == pytest.approx(LATITUDES[2])


@pytest.mark.parametrize("index", [3, -4])
def test_an_index_outside_the_points_raises_index_error(index: int) -> None:
    with pytest.raises(IndexError, match="index out of range"):
        _three_fixes().points[index]


def test_a_non_integer_index_raises_type_error() -> None:
    with pytest.raises(TypeError):
        _three_fixes().points["first"]  # type: ignore[call-overload]


@pytest.mark.parametrize(
    ("key", "expected"),
    [
        (slice(0, 2), [0, 1]),
        (slice(None, None, 2), [0, 2]),
        (slice(None, None, -1), [2, 1, 0]),
        (slice(5, 9), []),
    ],
)
def test_a_slice_selects_the_points_it_names(key: slice, expected: list[int]) -> None:
    selected = _three_fixes().points[key]
    assert isinstance(selected, list)
    assert [p.lat for p in selected] == pytest.approx([LATITUDES[i] for i in expected])


def test_iteration_yields_every_point_in_order() -> None:
    assert [p.lat for p in _three_fixes().points] == pytest.approx(LATITUDES)


def test_repr_states_the_point_count() -> None:
    assert repr(_three_fixes().points) == "NavPointSequence(len=3)"


def test_a_sequence_reads_the_file_once_the_nav_file_is_gone() -> None:
    def sequence_from_a_dropped_nav_file() -> NavPointSequence:
        return _three_fixes().points

    points = sequence_from_a_dropped_nav_file()
    gc.collect()
    assert [p.lat for p in points] == pytest.approx(LATITUDES)


@pytest.mark.parametrize("name", COLLECTION_NAMES)
def test_every_collection_indexes_what_it_iterates(name: str) -> None:
    sequence = getattr(_every_collection(), name)
    elements = list(sequence)

    assert len(sequence) == len(elements)
    assert [repr(sequence[i]) for i in range(len(sequence))] == [
        repr(element) for element in elements
    ]
    assert repr(sequence[-1]) == repr(elements[-1])


@pytest.mark.parametrize(
    ("column", "field"),
    [
        ("latitudes", "lat"),
        ("longitudes", "lon"),
        ("gps_times", "gps_time"),
        ("sys_times", "sys_time"),
        ("headings", "heading"),
        ("speeds_mps", "speed_mps"),
        ("eph_m_values", "eph_m"),
    ],
)
def test_a_column_equals_the_matching_field_of_every_point(
    column: str, field: str
) -> None:
    points = _three_fixes().points
    assert getattr(points, column)() == [getattr(p, field) for p in points]
