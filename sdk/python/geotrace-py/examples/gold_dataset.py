#!/usr/bin/env python3
"""Gold dataset reference example for the GeoTrace Python SDK.

Reads the CSV fixtures in ``tests/fixtures/gold_dataset/`` (the cross-SDK
reference data, mirrored by the Rust/C/C++ ``gold_dataset`` examples), builds a
``.gtd`` file, writes it as ``gold_py.gtd``, and verifies the round-trip.

Run it from anywhere - the repository root is located relative to this file.
"""

from __future__ import annotations

import csv
from datetime import datetime
from pathlib import Path

from geotrace_sdk import (
    Annotation,
    Constellation,
    EventMarker,
    EventMarkerStyle,
    Marker,
    MarkerIcon,
    Meta,
    NavFileBuilder,
    NavFix,
    Satellite,
    SatelliteReport,
)

_ICONS = {
    "pin": MarkerIcon.PIN,
    "cross": MarkerIcon.CROSS,
    "circle": MarkerIcon.CIRCLE,
    "lightning": MarkerIcon.LIGHTNING,
    "warning": MarkerIcon.WARNING,
    "error": MarkerIcon.ERROR,
    "check": MarkerIcon.CHECK,
    "satellite": MarkerIcon.SATELLITE,
    "satellite_lost": MarkerIcon.SATELLITE_LOST,
    "gear": MarkerIcon.GEAR,
    "refresh": MarkerIcon.REFRESH,
    "download": MarkerIcon.DOWNLOAD,
    "upload": MarkerIcon.UPLOAD,
    "wrench": MarkerIcon.WRENCH,
}

_CONSTELLATIONS = {
    "gps": Constellation.GPS,
    "glonass": Constellation.GLONASS,
    "galileo": Constellation.GALILEO,
    "beidou": Constellation.BEIDOU,
}

# (gps_time, sys_time) raw CSV strings -> satellites captured at that instant.
SatKey = tuple[str, str]


def _find_repo_root() -> Path:
    """Walk up from this file until a directory holding the gold fixtures."""
    for parent in Path(__file__).resolve().parents:
        if (parent / "tests" / "fixtures" / "gold_dataset").is_dir():
            return parent
    msg = "could not locate tests/fixtures/gold_dataset above this file"
    raise FileNotFoundError(msg)


def _parse_ts(value: str) -> datetime | None:
    return datetime.fromisoformat(value) if value else None


def _opt_float(value: str) -> float | None:
    return float(value) if value else None


def _icon(name: str) -> MarkerIcon | None:
    return _ICONS.get(name)


def _rows(path: Path) -> list[list[str]]:
    """Return the data rows of a CSV file (header skipped), fields stripped."""
    # The fixtures contain UTF-8 (emoji in meta.csv), so be explicit to stop Windows
    # falling back to its cp1252 locale default and failing to decode them.
    with path.open(newline="", encoding="utf-8") as handle:
        rows = [[cell.strip() for cell in row] for row in csv.reader(handle)]
    return [row for row in rows[1:] if row and any(row)]


def _load_meta(base: Path) -> Meta:
    cols = _rows(base / "meta.csv")[0]
    return Meta(title=cols[0], device=cols[1], notes=cols[2], identity=cols[3])


def _load_event_styles(builder: NavFileBuilder, base: Path) -> None:
    for cols in _rows(base / "event_styles.csv"):
        builder.add_event_marker_style(
            EventMarkerStyle(
                cols[0],
                icon=_icon(cols[1]),
                color=cols[2] or None,
            )
        )


def _load_satellites(base: Path) -> dict[SatKey, list[Satellite]]:
    reports: dict[SatKey, list[Satellite]] = {}
    for cols in _rows(base / "satellites.csv"):
        sat = Satellite(
            _CONSTELLATIONS[cols[2]],
            int(cols[3]),
            in_fix=cols[4] == "true",
            elevation=_opt_float(cols[5]),
            azimuth=_opt_float(cols[6]),
            snr=_opt_float(cols[7]),
        )
        reports.setdefault((cols[0], cols[1]), []).append(sat)
    return reports


def _load_fixes(
    builder: NavFileBuilder,
    base: Path,
    satellites: dict[SatKey, list[Satellite]],
) -> None:
    for cols in _rows(base / "fixes.csv"):
        gps_time = _parse_ts(cols[1])
        sys_time = _parse_ts(cols[2])
        speed_kmh = _opt_float(cols[6])
        builder.add(
            NavFix(
                lat=float(cols[3]),
                lon=float(cols[4]),
                gps_time=gps_time,
                sys_time=sys_time,
                heading=_opt_float(cols[5]),
                # Match the SDK's MPS_PER_KMH = 1.0 / 3.6 constant-multiply so
                # the m/s value is bit-identical across SDKs (kmh / 3.6 differs
                # by 1 ULP for some values).
                speed_mps=None if speed_kmh is None else speed_kmh * (1.0 / 3.6),
                eph_m=_opt_float(cols[7]),
            )
        )

        tracked = satellites.pop((cols[1], cols[2]), None)
        if tracked:
            builder.add(SatelliteReport(tracked, gps_time=gps_time, sys_time=sys_time))


def _load_markers(builder: NavFileBuilder, base: Path) -> None:
    for cols in _rows(base / "markers.csv"):
        time = _parse_ts(cols[0])
        if time is None:
            continue
        builder.add(Annotation(time=time, label=cols[1] or None, icon=_icon(cols[2])))


def _load_events(builder: NavFileBuilder, base: Path) -> None:
    for cols in _rows(base / "events.csv"):
        time = _parse_ts(cols[0])
        if time is None:
            continue
        builder.add(EventMarker(cols[1], time, annotation=cols[2] or None))


def _verify(path: Path) -> None:
    from geotrace_sdk import NavFile

    file = NavFile.open(path)

    meta = file.meta
    assert meta.title is not None and "Gold Dataset 🏆" in meta.title
    assert meta.device is not None and "Synthetic Generator 🧬" in meta.device
    assert meta.notes is not None and "🛰️" in meta.notes

    points = file.points
    assert len(points) == 199, f"expected 199 nav points, got {len(points)}"

    antimeridian = [p for p in points if p.lon > 179.9 or p.lon < -179.9]
    assert len(antimeridian) == 10, f"antimeridian points: {len(antimeridian)}"

    stationary = [p for p in points if abs(p.lat - (-10.0)) < 1e-6]
    assert len(stationary) == 20, f"stationary points: {len(stationary)}"
    assert all(p.speed_mps == 0.0 for p in stationary)

    markers: list[Marker] = file.markers
    assert len(markers) == 15, f"expected 15 markers, got {len(markers)}"
    assert markers[0].label == "File Boundary Start"
    assert markers[0].icon == MarkerIcon.CHECK

    events = file.event_markers
    assert len(events) == 6, f"expected 6 event markers, got {len(events)}"

    styles = {s.variant_path: s for s in file.event_marker_styles}
    assert styles["style/custom-icon"].icon == MarkerIcon.LIGHTNING
    assert styles["style/custom-color"].color == "#FF00FF"


def main() -> None:
    base = _find_repo_root() / "tests" / "fixtures" / "gold_dataset"

    builder = NavFileBuilder().with_meta(_load_meta(base))
    _load_event_styles(builder, base)
    _load_fixes(builder, base, _load_satellites(base))
    _load_markers(builder, base)
    _load_events(builder, base)

    nav_file = builder.finish()

    out = base / "gold_py.gtd"
    nav_file.write_to_file(out)

    print(f"Gold dataset generated successfully: {out}")
    print(f"  Nav points:    {len(nav_file.points)}")
    print(f"  Markers:       {len(nav_file.markers)}")
    print(f"  Event markers: {len(nav_file.event_markers)}")

    print("Verifying round-trip integrity...")
    _verify(out)
    print("Verified!")


if __name__ == "__main__":
    main()
