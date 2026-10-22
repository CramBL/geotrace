#!/usr/bin/env python3
"""Record host-clock satellite polling independently of navigation summaries."""

from datetime import UTC, datetime, timedelta
from pathlib import Path

from geotrace_sdk import (
    Constellation,
    NavFileBuilder,
    NavFix,
    SatelliteCoverage,
    SatelliteIdentity,
    SatelliteMembership,
    SatelliteObservation,
    SatelliteObservationReport,
    SatelliteScope,
    SatelliteUsage,
    SatelliteUsageReport,
    SolutionState,
)

START = datetime(2023, 11, 14, 22, 13, 20, tzinfo=UTC)
POLL = START + timedelta(seconds=2)
identity = SatelliteIdentity(Constellation.GPS, 1)
usage = SatelliteUsageReport(
    [
        SatelliteUsage(identity, SatelliteMembership.UNKNOWN),
        SatelliteUsage(identity, SatelliteMembership.USED),
    ],
    scope=SatelliteScope.constellations([Constellation.GPS]),
    coverage=SatelliteCoverage.COMPLETE,
    reported_count=8,
    sys_time=POLL,
)
observations = SatelliteObservationReport(
    [SatelliteObservation(identity, snr=0), SatelliteObservation(identity)],
    scope=SatelliteScope.global_(),
    coverage=SatelliteCoverage.PARTIAL,
    reported_count=12,
    sys_time=POLL,
)
output = Path(__file__).resolve().parents[1] / "build" / "examples"
output.mkdir(parents=True, exist_ok=True)
variants: list[tuple[str, list[SatelliteUsageReport | SatelliteObservationReport]]] = [
    ("usage", [usage]),
    ("observations", [observations]),
    ("both", [usage, observations]),
]
for name, reports in variants:
    builder = NavFileBuilder()
    for seconds in [0, 10]:
        builder.add(
            NavFix(
                0,
                seconds,
                sys_time=START + timedelta(seconds=seconds),
                state=SolutionState.FIX,
                used_count=8,
                hdop=0.9,
            )
        )
    for report in reports:
        builder.add(report)
    path = output / f"navigation_{name}.gtd"
    builder.finish().write_to_file(path)
    print(path)
