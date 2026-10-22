#!/usr/bin/env python3
"""Record pre-parsed navigation facts, including a status without position."""

from datetime import UTC, datetime, timedelta
from pathlib import Path

from geotrace_sdk import NavFileBuilder, NavFix, SolutionState

start = datetime(2026, 10, 22, 12, 0, tzinfo=UTC)
writer = NavFileBuilder().with_title("Basic navigation recording")
writer.add(
    NavFix(
        55.67,
        12.56,
        gps_time=start,
        state=SolutionState.FIX,
        used_count=8,
        hdop=0.9,
        heading=75.0,
        speed_mps=4.2,
    )
)
writer.add(
    NavFix(
        None,
        None,
        gps_time=start + timedelta(seconds=2),
        state=SolutionState.NO_FIX,
    )
)
writer.add(
    NavFix(
        55.671,
        12.562,
        gps_time=start + timedelta(seconds=5),
        state=SolutionState.FIX,
        used_count=9,
    )
)
output = (
    Path(__file__).resolve().parents[1] / "build" / "examples" / "navigation_only.gtd"
)
output.parent.mkdir(parents=True, exist_ok=True)
writer.finish().write_to_file(output)
print(output)
