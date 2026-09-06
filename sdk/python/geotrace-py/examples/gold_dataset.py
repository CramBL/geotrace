#!/usr/bin/env python3
"""Gold dataset reference example for the GeoTrace Python SDK.

Reads the CSV fixtures in ``tests/fixtures/gold_dataset/`` (the cross-SDK
reference data, mirrored by the Rust/C/C++ ``gold_dataset`` examples), builds a
``.gtd`` file, writes it as ``gold_py.gtd``, and verifies the round-trip.

Run it from anywhere - the repository root is located relative to this file.
"""

from __future__ import annotations

import csv
from dataclasses import dataclass, field
from datetime import datetime
from pathlib import Path

from geotrace_sdk import (
    Annotation,
    Channel,
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
    TravelMode,
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

# (`gps_time`, `sys_time`) raw CSV strings -> satellites captured at that instant.
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
    return Meta(
        title=cols[0],
        device=cols[1],
        notes=cols[2],
        identity=cols[3],
        travel_mode=cols[4],
    )


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
                # the m/s value is bit-identical across SDKs (`kmh / 3.6` differs
                # by 1 ULP for some values).
                speed_mps=None if speed_kmh is None else speed_kmh * (1.0 / 3.6),
                eph_m=_opt_float(cols[7]),
            )
        )

        tracked = satellites.pop((cols[1], cols[2]), None)
        if tracked:
            builder.add(SatelliteReport(tracked, gps_time=gps_time, sys_time=sys_time))

    # Reports at a time no fix row holds. The builder gives each one a ghost fix.
    for (gps_col, sys_col), tracked in satellites.items():
        builder.add(
            SatelliteReport(
                tracked, gps_time=_parse_ts(gps_col), sys_time=_parse_ts(sys_col)
            )
        )


def _load_markers(builder: NavFileBuilder, base: Path) -> None:
    for cols in _rows(base / "markers.csv"):
        time = _parse_ts(cols[0])
        if time is None:
            continue
        icon = _icon(cols[2]) or MarkerIcon.PIN
        builder.add(Annotation(time=time, label=cols[1] or None, icon=icon))


def _load_events(builder: NavFileBuilder, base: Path) -> None:
    for cols in _rows(base / "events.csv"):
        time = _parse_ts(cols[0])
        if time is None:
            continue
        builder.add(EventMarker(cols[1], time, annotation=cols[2] or None))


@dataclass
class _ChannelAcc:
    """Accumulates one channel's metadata and samples across its CSV rows."""

    unit: str | None
    period_deg: float | None
    description: str | None
    components: list[str] | None
    times: list[datetime] = field(default_factory=list)
    values: list[float] = field(default_factory=list)


def _load_channels(builder: NavFileBuilder, base: Path) -> None:
    accumulators: dict[str, _ChannelAcc] = {}
    order: list[str] = []
    for cols in _rows(base / "channels.csv"):
        # cols: `name`, `unit`, `period_deg`, `description`, `components`,
        # `time`, `values`
        if len(cols) < 7:
            continue
        name = cols[0]
        acc = accumulators.get(name)
        if acc is None:
            acc = _ChannelAcc(
                unit=cols[1] or None,
                period_deg=float(cols[2]) if cols[2] else None,
                description=cols[3] or None,
                components=cols[4].split(";") if cols[4] else None,
            )
            accumulators[name] = acc
            order.append(name)
        time = _parse_ts(cols[5])
        if time is None:
            raise ValueError("channels.csv: invalid timestamp")
        acc.times.append(time)
        acc.values.extend(float(v) for v in cols[6].split(";"))

    for name in order:
        acc = accumulators[name]
        builder.add(
            Channel(
                name,
                acc.times,
                acc.values,
                unit=acc.unit,
                period_deg=acc.period_deg,
                description=acc.description,
                components=acc.components,
            )
        )


def _verify(path: Path) -> None:
    from geotrace_sdk import NavFile

    file = NavFile.open(path)

    meta = file.meta
    assert meta.title is not None and "Gold Dataset 🏆" in meta.title
    assert meta.device is not None and "Synthetic Generator 🧬" in meta.device
    assert meta.notes is not None and "🛰️" in meta.notes
    assert meta.travel_mode == TravelMode.BICYCLE

    points = file.points
    assert len(points) == 200, f"expected 200 nav points, got {len(points)}"

    antimeridian = [p for p in points if p.lon > 179.9 or p.lon < -179.9]
    assert len(antimeridian) == 11, f"antimeridian points: {len(antimeridian)}"

    stationary = [p for p in points if abs(p.lat - (-10.0)) < 1e-6]
    assert len(stationary) == 20, f"stationary points: {len(stationary)}"
    assert all(p.speed_mps == 0.0 for p in stationary)

    markers: list[Marker] = file.markers
    assert len(markers) == 16, f"expected 16 markers, got {len(markers)}"
    assert markers[0].label == "File Boundary Start"
    assert markers[0].icon == MarkerIcon.CHECK

    events = file.event_markers
    assert len(events) == 7, f"expected 7 event markers, got {len(events)}"

    styles = {s.variant_path: s for s in file.event_marker_styles}
    assert styles["style/custom-icon"].icon == MarkerIcon.LIGHTNING
    assert styles["style/custom-color"].color == "#FF00FF"

    channels = {c.name: c for c in file.channels}
    assert len(channels) == 2, f"expected 2 channels, got {len(channels)}"
    assert channels["accel"].is_vector
    assert channels["accel"].components == ["x", "y", "z"]
    assert channels["heading_raw"].period_deg == 360.0


def main() -> None:
    base = _find_repo_root() / "tests" / "fixtures" / "gold_dataset"

    builder = NavFileBuilder().with_meta(_load_meta(base))
    _load_event_styles(builder, base)
    _load_fixes(builder, base, _load_satellites(base))
    _load_markers(builder, base)
    _load_events(builder, base)
    _load_channels(builder, base)

    nav_file = builder.finish()

    out = base / "gold_py.gtd"
    nav_file.write_to_file(out)

    print(f"Gold dataset generated successfully: {out}")
    print(f"  Nav points:    {len(nav_file.points)}")
    print(f"  Markers:       {len(nav_file.markers)}")
    print(f"  Event markers: {len(nav_file.event_markers)}")
    print(f"  Channels:      {len(nav_file.channels)}")

    print("Verifying round-trip integrity...")
    _verify(out)
    print("Verified!")


if __name__ == "__main__":
    main()
