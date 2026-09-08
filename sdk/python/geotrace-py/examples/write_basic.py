#!/usr/bin/env python3
"""Write a basic .gtd file from a hardcoded GPS track.

This example shows the minimal workflow: create a NavFileBuilder, add a few
NavFix points (plus an optional satellite report and a map annotation), call
finish(), and write to disk.
"""

from __future__ import annotations

import tempfile
from datetime import UTC, datetime, timedelta
from pathlib import Path

from geotrace_sdk import (
    Annotation,
    Constellation,
    MarkerIcon,
    NavFileBuilder,
    NavFix,
    Satellite,
    SatelliteReport,
)

START = datetime(2024, 6, 1, 8, 0, 0, tzinfo=UTC)

builder = NavFileBuilder().with_title("Quick tour").with_device("Example GPS v1.0")

builder.add(
    NavFix(
        lat=51.5074,
        lon=-0.1278,
        gps_time=START,
        heading=90.0,
        speed_mps=5.5,
        eph_m=3.2,
    )
)

builder.add(
    SatelliteReport(
        [
            Satellite(
                Constellation.GPS,
                1,
                in_fix=True,
                elevation=45.0,
                azimuth=90.0,
                snr=38.0,
            ),
            # Elevation and azimuth are optional. A receiver reports an SNR
            # for a satellite whose position it has not computed.
            Satellite(Constellation.GALILEO, 3, snr=22.0),
        ],
        gps_time=START,
    )
)

builder.add(
    NavFix(
        lat=51.5080,
        lon=-0.1265,
        gps_time=START + timedelta(seconds=10),
        heading=85.0,
        speed_mps=5.8,
    )
)

builder.add(Annotation(START, label="Start point", icon=MarkerIcon.PIN))

nav_file = builder.finish()

out = Path(tempfile.gettempdir()) / "geotrace_write_basic.gtd"
nav_file.write_to_file(out)
print(f"Wrote {len(nav_file.points)} nav points to {out}")

out.unlink()
