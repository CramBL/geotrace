#!/usr/bin/env python3
"""Write a .gtd file with ad-hoc sensor channels, then read them back.

A channel is a named time series sampled at its own rate, correlated with the
nav track by timestamp. It can be scalar (an inclinometer angle) or a vector
whose components share one sample clock (an accelerometer's x/y/z axes).

The three channels below cover the three ways to declare a unit: a recognized
label string, a Unit catalog constant, and ChannelUnit.custom for a label
outside the catalog, whose values stay dimensionless in queries.
"""

from __future__ import annotations

import tempfile
from datetime import UTC, datetime, timedelta
from pathlib import Path

from geotrace_sdk import Channel, ChannelUnit, NavFile, NavFileBuilder, NavFix, Unit

START = datetime(2024, 6, 1, 8, 0, 0, tzinfo=UTC)


def main() -> None:
    times = [START + timedelta(seconds=i) for i in range(3)]

    builder = NavFileBuilder()
    builder.add(NavFix(lat=51.5074, lon=-0.1278, gps_time=START))
    builder.add(
        Channel(
            "incline",
            times,
            [1.0, 1.5, 2.0],
            unit="deg",
            description="boom inclinometer",
        )
    )
    builder.add(
        Channel(
            "accel",
            times,
            # Row-major: three samples of (x, y, z).
            [
                0.0,
                200.0,
                980.0,
                100.0,
                200.0,
                1000.0,
                200.0,
                200.0,
                1020.0,
            ],
            unit=Unit.MG,
            components=["x", "y", "z"],
        )
    )
    builder.add(
        Channel(
            "quality",
            times,
            [80.0, 81.0, 82.0],
            unit=ChannelUnit.custom("vendor score"),
        )
    )

    with tempfile.NamedTemporaryFile(suffix=".gtd", delete=False) as f:
        path = f.name
    try:
        builder.finish().write_to_file(path)
        nav_file = NavFile.open(path)
        print(f"{len(nav_file.channels)} channels:")
        for channel in nav_file.channels:
            unit = f" [{channel.unit}]" if channel.unit else ""
            components = ""
            if channel.is_vector:
                components = f" components: {', '.join(channel.components)}"
            print(f"  {channel.name} {len(channel.times)} samples{unit}{components}")
    finally:
        Path(path).unlink()


if __name__ == "__main__":
    main()
