"""Round-trip tests: write a .gtd file then read it back and verify the data."""

from __future__ import annotations

import tempfile
from datetime import UTC, datetime, timedelta
from pathlib import Path

import geotrace_sdk
import pytest
from geotrace_sdk import (
    Annotation,
    Constellation,
    MarkerIcon,
    Meta,
    NavFile,
    NavFileBuilder,
    NavFix,
    Satellite,
    SatelliteReport,
    TravelMode,
)

ABSENT_COUNT_TIME = datetime(1969, 12, 31, 23, 59, 59, 999999, tzinfo=UTC)
T0 = datetime(2024, 6, 1, 9, 0, 0, tzinfo=UTC)
T1 = datetime(2024, 6, 1, 9, 1, 0, tzinfo=UTC)
T2 = datetime(2024, 6, 1, 9, 2, 0, tzinfo=UTC)


def _write_and_read(builder: NavFileBuilder, suffix: str = ".gtd") -> NavFile:
    """Serialise to a temp file and re-open it."""
    with tempfile.NamedTemporaryFile(suffix=suffix, delete=False) as f:
        path = f.name
    try:
        builder.finish().write_to_file(path)
        return NavFile.open(path)
    finally:
        Path(path).unlink()


def test_roundtrip_minimal() -> None:
    b = NavFileBuilder()
    b.add(NavFix(lat=51.5074, lon=-0.1278, gps_time=T0))
    nav_file = _write_and_read(b)

    assert len(nav_file.points) == 1
    pt = nav_file.points[0]
    assert abs(pt.lat - 51.5074) < 1e-5
    assert abs(pt.lon - (-0.1278)) < 1e-5


def test_roundtrip_three_points() -> None:
    fixes = [
        (51.50, -0.10, T0),
        (51.51, -0.11, T1),
        (51.52, -0.12, T2),
    ]
    b = NavFileBuilder()
    for lat, lon, t in fixes:
        b.add(NavFix(lat=lat, lon=lon, gps_time=t))

    nav_file = _write_and_read(b)
    assert len(nav_file.points) == 3

    for i, (lat, lon, _) in enumerate(fixes):
        pt = nav_file.points[i]
        assert abs(pt.lat - lat) < 1e-5
        assert abs(pt.lon - lon) < 1e-5


def test_roundtrip_heading_and_speed() -> None:
    b = NavFileBuilder()
    b.add(NavFix(lat=48.0, lon=2.0, gps_time=T0, heading=135.0, speed_mps=22.2))
    nav_file = _write_and_read(b)

    pt = nav_file.points[0]
    assert pt.heading is not None
    assert abs(pt.heading - 135.0) < 0.01
    assert pt.speed_mps is not None
    assert abs(pt.speed_mps - 22.2) < 0.01


def test_roundtrip_eph_m() -> None:
    b = NavFileBuilder()
    b.add(NavFix(lat=52.0, lon=13.0, gps_time=T0, eph_m=3.7))
    nav_file = _write_and_read(b)

    pt = nav_file.points[0]
    assert pt.eph_m is not None
    assert abs(pt.eph_m - 3.7) < 0.01


def test_roundtrip_gps_time_preserved() -> None:
    b = NavFileBuilder()
    b.add(NavFix(lat=0.0, lon=0.0, gps_time=T0))
    nav_file = _write_and_read(b)

    pt = nav_file.points[0]
    assert pt.gps_time is not None
    # Timestamps are stored at microsecond precision, allow 1 ms tolerance.
    delta = abs((pt.gps_time - T0).total_seconds())
    assert delta < 0.001


def test_roundtrip_sys_time_preserved() -> None:
    b = NavFileBuilder()
    b.add(NavFix(lat=0.0, lon=0.0, gps_time=T0, sys_time=T0))
    nav_file = _write_and_read(b)

    pt = nav_file.points[0]
    assert pt.sys_time is not None
    delta = abs((pt.sys_time - T0).total_seconds())
    assert delta < 0.001


def test_roundtrip_fix_without_a_lock_keeps_gps_time_none() -> None:
    b = NavFileBuilder()
    b.add(NavFix(lat=51.50, lon=-0.10, gps_time=T0, sys_time=T0))
    b.add(NavFix(lat=51.51, lon=-0.11, sys_time=T1))
    nav_file = _write_and_read(b)

    assert len(nav_file.points) == 2
    receiver_stamped, host_stamped = nav_file.points

    assert abs(receiver_stamped.lat - 51.50) < 1e-5
    assert receiver_stamped.gps_time is not None
    assert abs((receiver_stamped.gps_time - T0).total_seconds()) < 0.001
    assert receiver_stamped.sys_time is not None
    assert abs((receiver_stamped.sys_time - T0).total_seconds()) < 0.001

    assert abs(host_stamped.lat - 51.51) < 1e-5
    assert host_stamped.gps_time is None
    assert host_stamped.sys_time is not None
    assert abs((host_stamped.sys_time - T1).total_seconds()) < 0.001


def test_roundtrip_satellites() -> None:
    b = NavFileBuilder()
    b.add(NavFix(lat=51.5, lon=-0.1, gps_time=T0))
    b.add(
        SatelliteReport(
            [
                Satellite(Constellation.GPS, 5, in_fix=True, elevation=60.0, snr=42.0),
                Satellite(Constellation.GALILEO, 12, in_fix=False, azimuth=180.0),
            ],
            gps_time=T0,
        )
    )
    nav_file = _write_and_read(b)

    pt = nav_file.points[0]
    assert pt.satellites is not None
    sats = pt.satellites.tracked
    assert len(sats) == 2
    gps_sat = next(s for s in sats if s.constellation == Constellation.GPS)
    assert gps_sat.in_fix
    assert gps_sat.prn == 5
    assert gps_sat.elevation is not None
    assert abs(gps_sat.elevation - 60.0) < 0.1


def test_roundtrip_satellite_report_with_only_a_sys_time_keeps_gps_time_none() -> None:
    b = NavFileBuilder()
    b.add(NavFix(lat=51.5, lon=-0.1, gps_time=T0, sys_time=T0))
    b.add(SatelliteReport([Satellite(Constellation.GPS, 5, in_fix=True)], sys_time=T0))
    nav_file = _write_and_read(b)

    report = nav_file.points[0].satellites
    assert report is not None
    assert report.gps_time is None
    assert report.sys_time is not None
    assert abs((report.sys_time - T0).total_seconds()) < 0.001


def test_roundtrip_annotation() -> None:
    b = NavFileBuilder()
    b.add(NavFix(lat=51.5, lon=-0.1, gps_time=T0))
    b.add(NavFix(lat=51.52, lon=-0.12, gps_time=T2))
    b.add(Annotation(T1, label="Pit stop", icon=MarkerIcon.PIN))
    nav_file = _write_and_read(b)

    assert len(nav_file.markers) == 1
    m = nav_file.markers[0]
    assert m.annotation.label == "Pit stop"
    assert m.annotation.icon == MarkerIcon.PIN
    # Interpolated position should be between the two fixes.
    assert 51.5 < m.lat < 51.52
    assert -0.12 < m.lon < -0.1


def test_roundtrip_annotation_without_an_icon_reads_back_as_pin() -> None:
    b = NavFileBuilder()
    b.add(NavFix(lat=51.5, lon=-0.1, gps_time=T0))
    written = Annotation(T0, label="Pit stop")
    b.add(written)
    nav_file = _write_and_read(b)

    assert nav_file.markers[0].icon == MarkerIcon.PIN
    assert nav_file.markers[0].annotation == written


def test_roundtrip_to_bytes() -> None:
    b = NavFileBuilder()
    b.add(NavFix(lat=51.5, lon=-0.1, gps_time=T0))
    nav_file = b.finish()

    # Serialise to bytes, then read back from bytes.
    data = nav_file.to_bytes()
    assert len(data) > 0

    with tempfile.NamedTemporaryFile(suffix=".gtd", delete=False) as f:
        f.write(data)
        path = f.name
    try:
        reopened = NavFile.open(path)
        assert len(reopened.points) == 1
        assert abs(reopened.points[0].lat - 51.5) < 1e-5
    finally:
        Path(path).unlink()


def test_a_fix_at_the_absent_timestamp_count_is_rejected() -> None:
    b = NavFileBuilder()
    b.add(NavFix(lat=51.5, lon=-0.1, gps_time=ABSENT_COUNT_TIME))

    with pytest.raises(ValueError, match="nav_points/gps_time_us: record 0"):
        b.finish().to_bytes()


def test_a_fix_one_microsecond_before_the_absent_timestamp_count_round_trips() -> None:
    time = ABSENT_COUNT_TIME - timedelta(microseconds=1)
    b = NavFileBuilder()
    b.add(NavFix(lat=51.5, lon=-0.1, gps_time=time))

    assert _write_and_read(b).points[0].gps_time == time


def test_a_build_without_provenance_writes_only_the_sdk_version() -> None:
    b = NavFileBuilder()
    b.add(NavFix(lat=51.5, lon=-0.1, gps_time=T0))
    meta = _write_and_read(b).meta

    assert meta.sdk_version == geotrace_sdk.__version__
    assert meta.sdk_git_commit is None
    assert meta.sdk_commit_time is None


def test_roundtrip_travel_mode() -> None:
    b = NavFileBuilder()
    b.with_meta(Meta(travel_mode=TravelMode.PEDESTRIAN))
    b.add(NavFix(lat=51.5, lon=-0.1, gps_time=T0))
    nav_file = _write_and_read(b)

    assert nav_file.meta.travel_mode == TravelMode.PEDESTRIAN


def test_roundtrip_unknown_travel_mode_preserved() -> None:
    # A wire value outside the known set (e.g. written by a newer SDK) must
    # survive a round trip verbatim as a str, never be dropped.
    b = NavFileBuilder()
    b.with_meta(Meta(travel_mode="hovercraft"))
    b.add(NavFix(lat=51.5, lon=-0.1, gps_time=T0))
    nav_file = _write_and_read(b)

    assert nav_file.meta.travel_mode == "hovercraft"


def test_roundtrip_no_travel_mode() -> None:
    b = NavFileBuilder()
    b.add(NavFix(lat=51.5, lon=-0.1, gps_time=T0))
    nav_file = _write_and_read(b)

    assert nav_file.meta.travel_mode is None


def test_open_missing_file_raises() -> None:
    with pytest.raises(OSError):
        NavFile.open("/nonexistent/path/that/does/not/exist.gtd")


# Every icon the .gtd format defines must be exposed by the Python SDK and
# round-trip unchanged - this guards against the binding (or the .pyi stub
# that mypy checks against) drifting away from the canonical 14-icon set.
ALL_MARKER_ICONS = [
    MarkerIcon.PIN,
    MarkerIcon.CROSS,
    MarkerIcon.CIRCLE,
    MarkerIcon.LIGHTNING,
    MarkerIcon.WARNING,
    MarkerIcon.ERROR,
    MarkerIcon.CHECK,
    MarkerIcon.SATELLITE,
    MarkerIcon.SATELLITE_LOST,
    MarkerIcon.GEAR,
    MarkerIcon.REFRESH,
    MarkerIcon.DOWNLOAD,
    MarkerIcon.UPLOAD,
    MarkerIcon.WRENCH,
]


def test_roundtrip_all_marker_icons() -> None:
    b = NavFileBuilder()
    b.add(NavFix(lat=51.5, lon=-0.1, gps_time=T0))
    b.add(NavFix(lat=51.6, lon=-0.2, gps_time=T0 + timedelta(seconds=1000)))
    for i, icon in enumerate(ALL_MARKER_ICONS):
        b.add(Annotation(T0 + timedelta(seconds=10 * (i + 1)), label=str(i), icon=icon))

    nav_file = _write_and_read(b)

    assert len(nav_file.markers) == len(ALL_MARKER_ICONS)
    by_label = {m.label: m.icon for m in nav_file.markers}
    for i, icon in enumerate(ALL_MARKER_ICONS):
        assert by_label[str(i)] == icon, f"icon {icon} did not round-trip"


NAV_POINT_IDX_PAST_THE_NAV_POINTS_FIXTURE = (
    Path(__file__).resolve().parents[3]
    / "c"
    / "tests"
    / "fixtures"
    / "nav_point_idx_past_the_nav_points.gtd"
)


def test_a_satellite_report_pointing_past_the_nav_points_raises() -> None:
    data = NAV_POINT_IDX_PAST_THE_NAV_POINTS_FIXTURE.read_bytes()

    with pytest.raises(ValueError, match="sat_reports/nav_point_idx: record 0"):
        NavFile.from_bytes(data)
