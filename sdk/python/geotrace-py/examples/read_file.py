#!/usr/bin/env python3
"""Read and print a summary of a .gtd file.

Usage:
    python read_file.py <path-to-file.gtd>

With no path the script first writes a small file to a temp directory and then
reads that back, so it is runnable on its own.
"""

from __future__ import annotations

import sys
import tempfile
from datetime import UTC, datetime, timedelta
from pathlib import Path

from geotrace_sdk import (
    Annotation,
    Channel,
    Constellation,
    EventMarker,
    EventMarkerStyle,
    MarkerIcon,
    NavFile,
    NavFileBuilder,
    NavFix,
    Satellite,
    SatelliteReport,
    Unit,
)

START = datetime(2024, 6, 1, 8, 0, 0, tzinfo=UTC)


def summarise(path: Path) -> None:
    nav_file = NavFile.open(path)

    if nav_file.meta.title:
        print(f"Title:  {nav_file.meta.title}")
    if nav_file.meta.device:
        print(f"Device: {nav_file.meta.device}")

    if nav_file.points:
        print(f"Nav points: {len(nav_file.points)}")
        for i, point in enumerate(nav_file.points):
            line = f"  [{i}] {point.lat:.5f}, {point.lon:.5f}"
            if point.speed_mps is not None:
                line += f"  {point.speed_mps:.1f} m/s"
            if point.satellites is not None:
                line += f"  sats={len(point.satellites.tracked)}"
            print(line)

    if nav_file.markers:
        print(f"Markers: {len(nav_file.markers)}")
        for i, marker in enumerate(nav_file.markers):
            line = (
                f"  [{i}] {marker.lat:.5f}, {marker.lon:.5f}  icon={marker.icon_code}"
            )
            if marker.label:
                line += f" - {marker.label}"
            print(line)

    if nav_file.event_markers:
        print(f"Event markers: {len(nav_file.event_markers)}")
        for i, event_marker in enumerate(nav_file.event_markers):
            line = f"  [{i}] {event_marker.variant_path}"
            if event_marker.annotation:
                line += f" - {event_marker.annotation}"
            print(line)

    if nav_file.event_marker_styles:
        print(f"Event marker styles: {len(nav_file.event_marker_styles)}")
        for i, style in enumerate(nav_file.event_marker_styles):
            icon = style.icon_name or "auto"
            color = style.color or "auto"
            print(f"  [{i}] {style.variant_path}  icon={icon}  color={color}")

    if nav_file.channels:
        print(f"Channels: {len(nav_file.channels)}")
        for i, channel in enumerate(nav_file.channels):
            line = f"  [{i}] {channel.name} {len(channel.times)} samples"
            if channel.unit:
                line += f" [{channel.unit}]"
            if channel.is_vector:
                line += f" components: {' '.join(channel.components)}"
            print(line)


def write_sample_file(path: Path) -> None:
    """Write a sample file holding one of every section summarise() prints."""
    builder = (
        NavFileBuilder().with_title("Sample track").with_device("Example GPS v1.0")
    )

    for offset_secs, lat, lon in [
        (0, 51.5074, -0.1278),
        (30, 51.5088, -0.1248),
        (60, 51.5103, -0.1217),
    ]:
        builder.add(
            NavFix(
                lat=lat,
                lon=lon,
                gps_time=START + timedelta(seconds=offset_secs),
                heading=90.0,
                speed_mps=5.5,
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
                Satellite(Constellation.GALILEO, 3, snr=22.0),
            ],
            gps_time=START,
        )
    )
    builder.add(
        Annotation(
            START + timedelta(seconds=10), label="Start point", icon=MarkerIcon.PIN
        )
    )
    builder.add(
        EventMarker("power/boot", START + timedelta(seconds=2), annotation="cold start")
    )
    builder.add_event_marker_style(
        EventMarkerStyle("power/boot", icon=MarkerIcon.LIGHTNING, color="#44BB44")
    )
    builder.add(
        Channel(
            "incline",
            [START + timedelta(seconds=s) for s in (0, 30, 60)],
            [1.0, 1.5, 2.0],
            unit=Unit.DEG,
        )
    )

    builder.finish().write_to_file(path)


if __name__ == "__main__":
    if len(sys.argv) >= 2:
        summarise(Path(sys.argv[1]))
    else:
        sample = Path(tempfile.gettempdir()) / "geotrace_read_file_sample.gtd"
        write_sample_file(sample)
        try:
            summarise(sample)
        finally:
            sample.unlink()
