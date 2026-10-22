#!/usr/bin/env python3
"""Record navigation summaries independently of satellite detail."""

from datetime import UTC, datetime
from pathlib import Path

from geotrace_sdk import (
    NavFileBuilder,
    NavFix,
    SolutionDimension,
    SolutionMethod,
    SolutionState,
)

START = datetime(2023, 11, 14, 22, 13, 20, tzinfo=UTC)
builder = NavFileBuilder()
builder.add(
    NavFix(
        0,
        0,
        sys_time=START,
        state=SolutionState.FIX,
        method=SolutionMethod.RTK_FIXED,
        dimension=SolutionDimension.THREE_D,
        used_count=8,
        hdop=0.9,
        vdop=1.1,
        pdop=1.3,
    )
)
builder.add(
    NavFix(
        0,
        0,
        sys_time=START,
        state=SolutionState.UNKNOWN,
        used_count=0,
        position_provenance=None,
    )
)
output = (
    Path(__file__).resolve().parents[1] / "build" / "examples" / "navigation_only.gtd"
)
output.parent.mkdir(parents=True, exist_ok=True)
builder.finish().write_to_file(output)
print(output)
