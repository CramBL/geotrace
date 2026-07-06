"""Channel tests: scalar and vector sensor channels round-trip and validate."""

from __future__ import annotations

import tempfile
from datetime import UTC, datetime, timedelta
from pathlib import Path

import pytest
from geotrace_sdk import Channel, NavFile, NavFileBuilder, NavFix

T0 = datetime(2024, 6, 1, 9, 0, 0, tzinfo=UTC)
T1 = T0 + timedelta(seconds=1)


def _write_and_read(builder: NavFileBuilder) -> NavFile:
    """Serialise to a temp file and re-open it."""
    with tempfile.NamedTemporaryFile(suffix=".gtd", delete=False) as f:
        path = f.name
    try:
        builder.finish().write_to_file(path)
        return NavFile.open(path)
    finally:
        Path(path).unlink()


def test_scalar_and_vector_channels_round_trip() -> None:
    b = NavFileBuilder()
    b.add(NavFix(lat=51.5, lon=-0.1, gps_time=T0))
    b.add(
        Channel(
            "incline",
            [T0, T1],
            [1.5, 2.0],
            unit="deg",
            period_deg=360.0,
            description="boom",
        )
    )
    b.add(
        Channel(
            "accel",
            [T0, T1],
            [0.1, 0.2, 0.98, -0.1, 0.3, 1.02],
            unit="g",
            components=["x", "y", "z"],
        )
    )

    channels = _write_and_read(b).channels
    assert len(channels) == 2

    # Channels sort by name: accel (vector) then incline (scalar).
    accel, incline = channels
    assert accel.name == "accel"
    assert accel.is_vector
    assert accel.components == ["x", "y", "z"]
    assert accel.unit == "g"
    assert accel.period_deg is None
    assert accel.values == [0.1, 0.2, 0.98, -0.1, 0.3, 1.02]
    assert accel.times == [T0, T1]

    assert incline.name == "incline"
    assert not incline.is_vector
    assert incline.components == []
    assert incline.period_deg == 360.0
    assert incline.description == "boom"


def test_bare_channel_has_no_optional_fields() -> None:
    b = NavFileBuilder()
    b.add(Channel("temp", [T0], [20.0]))
    channel = _write_and_read(b).channels[0]
    assert channel.name == "temp"
    assert channel.unit is None
    assert channel.description is None
    assert channel.period_deg is None
    assert channel.components == []


def test_malformed_channel_raises() -> None:
    with pytest.raises(ValueError):  # a name that is not a lowercase identifier
        Channel("Bad Name", [T0], [1.0])
    with pytest.raises(ValueError):  # values not times * columns long
        Channel("accel", [T0], [1.0, 2.0])
    with pytest.raises(ValueError):  # a duplicate component label
        Channel("accel", [T0], [1.0, 2.0], components=["x", "x"])


def test_duplicate_channel_name_raises_at_finish() -> None:
    b = NavFileBuilder()
    b.add(Channel("accel", [T0], [1.0]))
    b.add(Channel("accel", [T0], [2.0]))
    with pytest.raises(ValueError):
        b.finish()
