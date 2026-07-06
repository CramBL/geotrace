#!/usr/bin/env python3
"""Write a .gtd file with ad-hoc sensor channels, then read them back.

A channel is a named time series sampled at its own rate, correlated with the
nav track by timestamp. It can be scalar (an inclinometer angle) or a vector
whose components share one sample clock (an accelerometer's x/y/z axes).
"""

from __future__ import annotations

import tempfile
from datetime import UTC, datetime, timedelta
from pathlib import Path

from geotrace_sdk import Channel, NavFile, NavFileBuilder, NavFix

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
            [0.0, 0.2, 0.98, 0.1, 0.2, 1.00, 0.2, 0.2, 1.02],
            unit="g",
            components=["x", "y", "z"],
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
