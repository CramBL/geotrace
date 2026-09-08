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

    builder = NavFileBuilder().with_title("Channel tour")
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
                980.0,
                200.0,
                200.0,
                980.0,
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

    out = Path(tempfile.gettempdir()) / "geotrace_channels.gtd"
    builder.finish().write_to_file(out)
    try:
        nav_file = NavFile.open(out)
        print(f"{len(nav_file.channels)} channels:")
        for channel in nav_file.channels:
            line = f"  {channel.name:<10} {len(channel.times)} samples"
            if channel.unit:
                line += f" [{channel.unit}]"
            if channel.is_vector:
                line += f" components: {' '.join(channel.components)}"
            print(line)
    finally:
        out.unlink()


if __name__ == "__main__":
    main()
