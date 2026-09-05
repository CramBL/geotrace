"""Tests for NavFileBuilder and the data-model types."""

from __future__ import annotations

import logging
from datetime import UTC, datetime

import pytest
from geotrace_sdk import (
    Annotation,
    Constellation,
    MarkerIcon,
    Meta,
    NavFileBuilder,
    NavFix,
    Satellite,
    SatelliteReport,
    TravelMode,
)

T0 = datetime(2024, 6, 1, 9, 0, 0, tzinfo=UTC)
T1 = datetime(2024, 6, 1, 9, 1, 0, tzinfo=UTC)
T2 = datetime(2024, 6, 1, 9, 2, 0, tzinfo=UTC)


def test_nav_fix_required_fields() -> None:
    fix = NavFix(lat=51.5074, lon=-0.1278, gps_time=T0)
    assert abs(fix.lat - 51.5074) < 1e-9
    assert abs(fix.lon - (-0.1278)) < 1e-9
    assert fix.gps_time is not None
    assert fix.heading is None
    assert fix.speed_mps is None
    assert fix.eph_m is None


def test_nav_fix_all_fields() -> None:
    fix = NavFix(
        lat=48.8566,
        lon=2.3522,
        gps_time=T0,
        sys_time=T0,
        heading=270.0,
        speed_mps=13.9,
        eph_m=1.5,
    )
    assert fix.heading is not None
    assert abs(fix.heading - 270.0) < 1e-9
    assert fix.speed_mps is not None
    assert abs(fix.speed_mps - 13.9) < 1e-6
    assert fix.eph_m is not None
    assert abs(fix.eph_m - 1.5) < 1e-9


def test_nav_fix_without_a_timestamp_is_rejected() -> None:
    with pytest.raises(ValueError, match="provide gps_time or sys_time"):
        NavFix(lat=51.5074, lon=-0.1278)


def test_nav_fix_with_only_a_host_timestamp() -> None:
    fix = NavFix(lat=51.5074, lon=-0.1278, sys_time=T0)
    assert fix.gps_time is None
    assert fix.sys_time == T0


def test_nav_fix_repr() -> None:
    fix = NavFix(lat=51.5, lon=-0.1, gps_time=T0)
    r = repr(fix)
    assert "NavFix" in r
    assert "51.5" in r


def test_nav_fix_eq() -> None:
    fix_a = NavFix(
        lat=51.5,
        lon=-0.1,
        gps_time=T0,
        heading=90.0,
        speed_mps=5.0,
        eph_m=2.0,
    )
    fix_b = NavFix(
        lat=51.5,
        lon=-0.1,
        gps_time=T0,
        heading=90.0,
        speed_mps=5.0,
        eph_m=2.0,
    )
    fix_c = NavFix(lat=51.5, lon=-0.2, gps_time=T0)
    assert fix_a == fix_b
    assert fix_a != fix_c


def test_satellite_minimal() -> None:
    sat = Satellite(Constellation.GPS, 12)
    assert sat.constellation == Constellation.GPS
    assert sat.prn == 12
    assert not sat.in_fix
    assert sat.elevation is None
    assert sat.snr is None


def test_satellite_all_fields() -> None:
    sat = Satellite(
        Constellation.GALILEO, 5, in_fix=True, elevation=45.0, azimuth=90.0, snr=35.5
    )
    assert sat.in_fix
    assert sat.elevation is not None
    assert abs(sat.elevation - 45.0) < 1e-4
    assert sat.azimuth is not None
    assert abs(sat.azimuth - 90.0) < 1e-4
    assert sat.snr is not None
    assert abs(sat.snr - 35.5) < 1e-4


def test_satellite_eq() -> None:
    sat_a = Satellite(Constellation.GPS, 3, in_fix=True, snr=40.0)
    sat_b = Satellite(Constellation.GPS, 3, in_fix=True, snr=40.0)
    sat_c = Satellite(Constellation.GPS, 4, in_fix=True, snr=40.0)
    assert sat_a == sat_b
    assert sat_a != sat_c


def test_satellite_report() -> None:
    sats = [
        Satellite(Constellation.GPS, 1, in_fix=True, snr=40.0),
        Satellite(Constellation.GLONASS, 7),
    ]
    report = SatelliteReport(sats, gps_time=T0)
    assert len(report.tracked) == 2
    assert report.tracked[0].prn == 1
    assert report.gps_time is not None


def test_satellite_report_eq() -> None:
    sats = [Satellite(Constellation.GPS, 1, in_fix=True, snr=40.0)]
    rep_a = SatelliteReport(sats, gps_time=T0)
    rep_b = SatelliteReport(sats, gps_time=T0)
    rep_c = SatelliteReport(sats, gps_time=T1)
    assert rep_a == rep_b
    assert rep_a != rep_c


def test_annotation_minimal() -> None:
    ann = Annotation(T0)
    assert ann.time.tzinfo is not None
    assert ann.label is None
    assert ann.icon is None


def test_annotation_all_fields() -> None:
    ann = Annotation(T0, label="Checkpoint", icon=MarkerIcon.CHECK)
    assert ann.label == "Checkpoint"
    assert ann.icon == MarkerIcon.CHECK


def test_annotation_label_past_the_field_capacity_raises() -> None:
    with pytest.raises(ValueError, match="markers/label"):
        Annotation(T0, label="l" * 256)


def test_annotation_eq() -> None:
    ann_a = Annotation(T0, label="Stop", icon=MarkerIcon.PIN)
    ann_b = Annotation(T0, label="Stop", icon=MarkerIcon.PIN)
    ann_c = Annotation(T0, label="Go")
    assert ann_a == ann_b
    assert ann_a != ann_c


def test_meta_empty() -> None:
    meta = Meta()
    assert meta.title is None
    assert meta.device is None
    assert meta.notes is None


def test_meta_full() -> None:
    meta = Meta(title="My Track", device="u-blox 9", notes="Test run")
    assert meta.title == "My Track"
    assert meta.device == "u-blox 9"
    assert meta.notes == "Test run"


def test_meta_eq() -> None:
    meta_a = Meta(title="Track", device="GPS-1")
    meta_b = Meta(title="Track", device="GPS-1")
    meta_c = Meta(title="Other")
    assert meta_a == meta_b
    assert meta_a != meta_c


def test_meta_travel_mode_enum() -> None:
    meta = Meta(travel_mode=TravelMode.BICYCLE)
    assert meta.travel_mode == TravelMode.BICYCLE


def test_meta_travel_mode_wire_name() -> None:
    meta = Meta(travel_mode="car")
    assert meta.travel_mode == TravelMode.CAR


def test_meta_travel_mode_unknown_preserved_as_str() -> None:
    meta = Meta(travel_mode="hovercraft")
    assert meta.travel_mode == "hovercraft"


def test_meta_travel_mode_absent() -> None:
    assert Meta().travel_mode is None


def test_meta_travel_mode_affects_eq() -> None:
    assert Meta(travel_mode=TravelMode.CAR) == Meta(travel_mode="car")
    assert Meta(travel_mode=TravelMode.CAR) != Meta(travel_mode=TravelMode.RAIL)
    assert Meta(travel_mode=TravelMode.CAR) != Meta()


def _three_point_builder() -> NavFileBuilder:
    b = NavFileBuilder()
    for i, (lat, lon) in enumerate([(51.5, -0.1), (51.51, -0.11), (51.52, -0.12)]):
        b.add(
            NavFix(lat=lat, lon=lon, gps_time=datetime(2024, 1, 1, 0, i, 0, tzinfo=UTC))
        )
    return b


def test_builder_produces_nav_file() -> None:
    b = _three_point_builder()
    f = b.finish()
    assert len(f.points) == 3


def test_builder_with_meta() -> None:
    b = NavFileBuilder()
    b.with_meta(Meta(title="Test track"))
    for i, (lat, lon) in enumerate([(51.5, -0.1), (51.51, -0.11), (51.52, -0.12)]):
        b.add(
            NavFix(lat=lat, lon=lon, gps_time=datetime(2024, 1, 1, 0, i, 0, tzinfo=UTC))
        )
    f = b.finish()
    assert len(f.points) == 3


def test_builder_chaining() -> None:
    f = (
        NavFileBuilder()
        .with_title("Chained")
        .add(NavFix(lat=51.5, lon=-0.1, gps_time=T0))
        .add(NavFix(lat=51.51, lon=-0.11, gps_time=T1))
        .finish()
    )
    assert len(f.points) == 2
    assert f.meta.title == "Chained"


def test_builder_with_title_device_notes() -> None:
    f = (
        NavFileBuilder()
        .with_title("My Track")
        .with_device("GPS-1")
        .with_notes("test run")
        .add(NavFix(lat=51.5, lon=-0.1, gps_time=T0))
        .finish()
    )
    assert f.meta.title == "My Track"
    assert f.meta.device == "GPS-1"
    assert f.meta.notes == "test run"


def test_builder_with_title_only() -> None:
    f = (
        NavFileBuilder()
        .with_title("Solo title")
        .add(NavFix(lat=51.5, lon=-0.1, gps_time=T0))
        .finish()
    )
    assert f.meta.title == "Solo title"
    assert f.meta.device is None
    assert f.meta.notes is None


def test_builder_with_satellite_reports() -> None:
    b = NavFileBuilder()
    b.add(NavFix(lat=51.5, lon=-0.1, gps_time=T0))
    b.add(
        SatelliteReport(
            [Satellite(Constellation.GPS, 3, in_fix=True, snr=38.0)],
            gps_time=T0,
        )
    )
    f = b.finish()
    assert len(f.points) == 1
    assert f.points[0].satellites is not None


def test_builder_drops_a_satellite_report_without_a_timestamp() -> None:
    b = NavFileBuilder()
    b.add(NavFix(lat=51.5, lon=-0.1, gps_time=T0))
    b.add(SatelliteReport([Satellite(Constellation.GPS, 3, in_fix=True)]))
    f = b.finish()

    assert len(f.points) == 1
    assert f.points[0].satellites is None


def test_builder_warns_on_the_geotrace_sdk_logger_when_it_drops_a_report(
    caplog: pytest.LogCaptureFixture,
) -> None:
    b = NavFileBuilder()
    b.add(NavFix(lat=51.5, lon=-0.1, gps_time=T0))
    b.add(SatelliteReport([Satellite(Constellation.GPS, 3, in_fix=True)]))

    with caplog.at_level(logging.WARNING, logger="geotrace_sdk"):
        b.finish()

    assert [(r.name, r.levelname, r.getMessage()) for r in caplog.records] == [
        (
            "geotrace_sdk.builder",
            "WARNING",
            "satellite report with no timestamp dropped",
        )
    ]


def test_a_log_level_lowered_after_the_first_record_applies(
    caplog: pytest.LogCaptureFixture,
) -> None:
    builder_dropping_a_report = NavFileBuilder()
    builder_dropping_a_report.add(NavFix(lat=51.5, lon=-0.1, gps_time=T0))
    builder_dropping_a_report.add(
        SatelliteReport([Satellite(Constellation.GPS, 3, in_fix=True)])
    )
    builder_dropping_a_report.finish()
    caplog.clear()

    builder_creating_a_ghost_fix = NavFileBuilder()
    builder_creating_a_ghost_fix.add(NavFix(lat=51.5, lon=-0.1, gps_time=T0))
    builder_creating_a_ghost_fix.add(
        SatelliteReport([Satellite(Constellation.GPS, 3, in_fix=True)], gps_time=T1)
    )
    with caplog.at_level(logging.DEBUG, logger="geotrace_sdk"):
        builder_creating_a_ghost_fix.finish()

    assert [r.levelname for r in caplog.records] == ["DEBUG"]


def test_builder_with_annotation() -> None:
    b = NavFileBuilder()
    b.add(NavFix(lat=51.5, lon=-0.1, gps_time=T0))
    b.add(NavFix(lat=51.51, lon=-0.11, gps_time=T2))
    b.add(Annotation(T1, label="Middle point"))
    f = b.finish()
    assert len(f.markers) == 1
    assert f.markers[0].annotation.label == "Middle point"


def test_builder_consumed_after_finish() -> None:
    b = _three_point_builder()
    b.finish()
    with pytest.raises(RuntimeError, match="consumed"):
        b.finish()


def test_builder_add_after_finish_raises() -> None:
    b = _three_point_builder()
    b.finish()
    with pytest.raises(RuntimeError, match="consumed"):
        b.add(NavFix(lat=0.0, lon=0.0, gps_time=T0))


def test_nav_file_meta() -> None:
    f = (
        NavFileBuilder()
        .with_title("Meta Test")
        .with_device("sensor-1")
        .add(NavFix(lat=51.5, lon=-0.1, gps_time=T0))
        .finish()
    )
    assert f.meta.title == "Meta Test"
    assert f.meta.device == "sensor-1"


def test_nav_file_to_bytes() -> None:
    f = NavFileBuilder().add(NavFix(lat=51.5, lon=-0.1, gps_time=T0)).finish()
    data = f.to_bytes()
    assert isinstance(data, bytes)
    assert len(data) > 0


def test_marker_convenience_accessors() -> None:
    f = (
        NavFileBuilder()
        .add(NavFix(lat=51.5, lon=-0.1, gps_time=T0))
        .add(NavFix(lat=51.52, lon=-0.12, gps_time=T2))
        .add(Annotation(T1, label="Waypoint", icon=MarkerIcon.PIN))
        .finish()
    )
    m = f.markers[0]
    assert m.label == "Waypoint"
    assert m.icon == MarkerIcon.PIN
    assert m.time is not None
    assert m.label == m.annotation.label
    assert m.icon == m.annotation.icon
