"""Canonical navigation facts and independently timed satellite evidence."""

from __future__ import annotations

import gc
import math
from datetime import UTC, datetime, timedelta
from pathlib import Path
from typing import Any

import pytest
from geotrace_sdk import (
    Constellation,
    NavFile,
    NavFileBuilder,
    NavFix,
    PositionProvenance,
    Satellite,
    SatelliteCoverage,
    SatelliteIdentity,
    SatelliteMembership,
    SatelliteObservation,
    SatelliteObservationReport,
    SatelliteReport,
    SatelliteScope,
    SatelliteUsage,
    SatelliteUsageReport,
    SolutionDimension,
    SolutionMethod,
    SolutionState,
)

T0 = datetime(2023, 11, 14, 22, 13, 20, tzinfo=UTC)
G01 = SatelliteIdentity(Constellation.GPS, 1)
GLOBAL = SatelliteScope.global_()


@pytest.mark.parametrize(
    ("field", "values"),
    [
        ("state", [None, *SolutionState]),
        ("method", [None, *SolutionMethod]),
        ("dimension", [None, *SolutionDimension]),
        ("position_provenance", [None, *PositionProvenance]),
        ("used_count", [None, 0, 2**32 - 1]),
        ("hdop", [None, 0.9]),
        ("vdop", [None, 1.1]),
        ("pdop", [None, 1.3]),
    ],
)
def test_navigation_metadata_readback_and_equality(
    field: str, values: list[Any], tmp_path: Path
) -> None:
    fixes = [NavFix(0, 0, sys_time=T0, **{field: value}) for value in values]
    builder = NavFileBuilder()
    for fix in fixes:
        builder.add(fix)
    path = tmp_path / "metadata.gtd"
    builder.finish().write_to_file(path)
    points = NavFile.open(path).points
    assert len(points) == len(values)
    assert [getattr(point, field) for point in points] == values
    for index, fix in enumerate(fixes):
        assert getattr(fix, field) == values[index]
        for other_index, other in enumerate(fixes):
            assert (fix == other) == (index == other_index)


def test_default_provenance_and_conflicting_navigation_facts() -> None:
    builder = NavFileBuilder()
    for state, count in [(SolutionState.NO_FIX, 12), (SolutionState.FIX, 0)]:
        fix = NavFix(0, 0, sys_time=T0, state=state, used_count=count, hdop=0.9)
        assert fix.position_provenance is PositionProvenance.SOURCE_PROVIDED
        assert fix.heading is None
        builder.add(fix)
    points = builder.finish().points
    assert [(p.state, p.used_count, p.hdop, p.eph_m) for p in points] == [
        (SolutionState.NO_FIX, 12, 0.9, None),
        (SolutionState.FIX, 0, 0.9, None),
    ]


@pytest.mark.parametrize(
    "field", ["state", "method", "dimension", "position_provenance"]
)
@pytest.mark.parametrize("value", [1, Constellation.GPS])
def test_navigation_enum_class_validation(field: str, value: Any) -> None:
    with pytest.raises(TypeError, match="member"):
        NavFix(0, 0, sys_time=T0, **{field: value})


@pytest.mark.parametrize("count", [-1, 2**32])
def test_unsigned_count_bounds(count: int) -> None:
    with pytest.raises(OverflowError):
        NavFix(0, 0, sys_time=T0, used_count=count)
    with pytest.raises(OverflowError):
        SatelliteUsageReport(
            [],
            scope=GLOBAL,
            coverage=SatelliteCoverage.PARTIAL,
            sys_time=T0,
            reported_count=count,
        )


def test_scope_canonicalization_and_immutable_values() -> None:
    scope = SatelliteScope.constellations(
        [Constellation.GPS, Constellation.GLONASS, Constellation.GPS]
    )
    assert scope == SatelliteScope.constellations(
        [Constellation.GLONASS, Constellation.GPS]
    )
    assert scope.constellation_set == [Constellation.GPS, Constellation.GLONASS]
    assert not scope.is_global
    assert GLOBAL.is_global
    assert SatelliteScope.constellations(list(Constellation)) != GLOBAL
    with pytest.raises(ValueError, match="at least one"):
        SatelliteScope.constellations([])
    with pytest.raises(TypeError, match="Constellation"):
        SatelliteScope.constellations([1])  # type: ignore[list-item]
    with pytest.raises(AttributeError):
        G01.prn = 2  # type: ignore[misc]
    assert SatelliteIdentity(Constellation.GPS, 2) != G01


@pytest.mark.parametrize("value", [0, SolutionState.UNKNOWN])
def test_report_enum_class_validation(value: Any) -> None:
    with pytest.raises(TypeError, match="SatelliteMembership"):
        SatelliteUsage(G01, value)
    with pytest.raises(TypeError, match="SatelliteCoverage"):
        SatelliteObservationReport([], scope=GLOBAL, coverage=value, sys_time=T0)


@pytest.mark.parametrize(
    "report_type", [SatelliteUsageReport, SatelliteObservationReport]
)
def test_report_requires_recorded_clock_and_explicit_coverage(
    report_type: type[SatelliteUsageReport] | type[SatelliteObservationReport],
) -> None:
    with pytest.raises(ValueError, match="provide gps_time or sys_time"):
        report_type([], scope=GLOBAL, coverage=SatelliteCoverage.COMPLETE)
    with pytest.raises(TypeError):
        report_type([], sys_time=T0)  # type: ignore[call-arg]
    with pytest.raises(TypeError):
        report_type(
            [],
            scope=GLOBAL,
            coverage=SatelliteCoverage.COMPLETE,
            sys_time=T0.replace(tzinfo=None),
        )


def test_independent_reports_preserve_rows_counts_times_and_owned_lifetimes(
    tmp_path: Path,
) -> None:
    rows = [SatelliteUsage(G01, member) for member in SatelliteMembership]
    scope = SatelliteScope.constellations([Constellation.GPS])
    usage = SatelliteUsageReport(
        rows,
        scope=scope,
        coverage=SatelliteCoverage.COMPLETE,
        reported_count=8,
        gps_time=T0,
        sys_time=T0 + timedelta(hours=1),
    )
    observations = SatelliteObservationReport(
        [SatelliteObservation(G01, snr=0), SatelliteObservation(G01)],
        scope=GLOBAL,
        coverage=SatelliteCoverage.PARTIAL,
        reported_count=12,
        gps_time=T0 + timedelta(milliseconds=400),
    )
    rows.clear()
    builder = NavFileBuilder().add(NavFix(0, 0, gps_time=T0))
    assert builder.add(usage) is builder
    builder.add(observations).add(usage)
    path = tmp_path / "reports.gtd"
    builder.finish().write_to_file(path)
    nav_file = NavFile.open(path)
    sequence = nav_file.points
    point = sequence[0]
    stored = point.usage_reports
    del nav_file
    gc.collect()
    assert sequence[0].usage_reports == stored
    assert point.satellites is None
    assert len(stored) == 2
    assert stored[0] != usage
    assert stored[0] != stored[1]
    assert [r.input_order for r in stored] == [0, 2]
    for report in stored:
        assert report.satellites == [
            SatelliteUsage(G01, m) for m in SatelliteMembership
        ]
        assert report.reported_count == 8
        assert report.coverage is SatelliteCoverage.COMPLETE
        assert report.scope == scope
        assert report.gps_time == T0
        assert report.sys_time == T0 + timedelta(hours=1)
        assert report.normalized_time == T0
    obs = point.observation_reports[0]
    del point, sequence
    gc.collect()
    assert obs.input_order == 1
    assert obs.normalized_time == T0 + timedelta(milliseconds=400)
    assert obs.satellites[0].snr == 0
    assert obs.satellites[1].snr is None
    assert obs.reported_count == 12
    assert obs.gps_time == obs.normalized_time
    assert obs.sys_time is None
    assert obs.scope.is_global
    detached_rows = obs.satellites
    detached_rows.clear()
    assert len(obs.satellites) == 2


@pytest.mark.parametrize(
    "report_type", [SatelliteUsageReport, SatelliteObservationReport]
)
def test_report_equality_includes_scope_coverage_aggregate_and_clocks(
    report_type: Any,
) -> None:
    arguments: dict[str, Any] = {
        "scope": GLOBAL,
        "coverage": SatelliteCoverage.COMPLETE,
        "sys_time": T0,
    }
    original = report_type([], **arguments)
    assert original == report_type([], **arguments)
    variants: list[dict[str, Any]] = [
        {"scope": SatelliteScope.constellations([Constellation.GPS])},
        {"coverage": SatelliteCoverage.PARTIAL},
        {"reported_count": 0},
        {"gps_time": T0},
        {"sys_time": T0 + timedelta(seconds=1)},
    ]
    for changes in variants:
        assert original != report_type([], **(arguments | changes))
    assert original.input_order is None
    assert original.normalized_time is None


@pytest.mark.parametrize("failed_finish", [False, True])
def test_new_reports_after_finish_use_consumed_builder_error(
    failed_finish: bool,
) -> None:
    builder = NavFileBuilder()
    if failed_finish:
        builder.add(
            SatelliteUsageReport(
                [], scope=GLOBAL, coverage=SatelliteCoverage.COMPLETE, sys_time=T0
            )
        )
        with pytest.raises(ValueError):
            builder.finish()
    else:
        builder.add(NavFix(0, 0, sys_time=T0)).finish()
    for report_type in [SatelliteUsageReport, SatelliteObservationReport]:
        report = report_type(
            [], scope=GLOBAL, coverage=SatelliteCoverage.COMPLETE, sys_time=T0
        )
        with pytest.raises(RuntimeError, match="consumed"):
            builder.add(report)


def test_host_only_polling_batches_equal_times_and_preserves_longitude() -> None:
    builder = NavFileBuilder()
    builder.add(NavFix(0, 0, sys_time=T0))
    builder.add(NavFix(0, 10, sys_time=T0 + timedelta(seconds=10)))
    time = T0 + timedelta(seconds=2)
    usage = SatelliteUsageReport(
        [SatelliteUsage(G01, SatelliteMembership.UNKNOWN)],
        scope=GLOBAL,
        coverage=SatelliteCoverage.PARTIAL,
        sys_time=time,
    )
    observation = SatelliteObservationReport(
        [SatelliteObservation(G01)],
        scope=GLOBAL,
        coverage=SatelliteCoverage.PARTIAL,
        sys_time=time,
    )
    points = builder.add(usage).add(observation).finish().points
    assert len(points) == 3
    carrier = points[1]
    assert carrier.lon == pytest.approx(2)
    assert carrier.position_provenance is PositionProvenance.SDK_SYNTHESIZED
    assert carrier.state is None
    report_orders: list[
        tuple[SatelliteUsageReport | SatelliteObservationReport, int]
    ] = [
        (carrier.usage_reports[0], 0),
        (carrier.observation_reports[0], 1),
    ]
    for report, order in report_orders:
        assert report.sys_time == report.normalized_time == time
        assert report.gps_time is None
        assert report.input_order == order


@pytest.mark.parametrize(
    "report_type", [SatelliteUsageReport, SatelliteObservationReport]
)
def test_empty_reports_preserve_coverage_and_optional_aggregate(
    report_type: Any, tmp_path: Path
) -> None:
    reports = [
        report_type(
            [], scope=GLOBAL, coverage=coverage, reported_count=count, sys_time=T0
        )
        for coverage in SatelliteCoverage
        for count in [None, 0, 2**32 - 1]
    ]
    builder = NavFileBuilder().add(NavFix(0, 0, sys_time=T0))
    for report in reports:
        builder.add(report)
    path = tmp_path / "empty_reports.gtd"
    builder.finish().write_to_file(path)
    point = NavFile.open(path).points[0]
    stored = (
        point.usage_reports
        if report_type is SatelliteUsageReport
        else point.observation_reports
    )
    assert [(r.coverage, r.reported_count, r.satellites) for r in stored] == [
        (r.coverage, r.reported_count, r.satellites) for r in reports
    ]
    assert point.satellites is None


def test_legacy_false_membership_preserves_known_unused() -> None:
    point = (
        NavFileBuilder()
        .add(NavFix(0, 0, sys_time=T0))
        .add(SatelliteReport([Satellite(Constellation.GPS, 1)], sys_time=T0))
        .finish()
        .points[0]
    )
    assert point.usage_reports[0].satellites[0].membership is SatelliteMembership.UNUSED
    assert point.satellites is not None
    assert point.satellites.tracked[0].in_fix is False
    assert (
        SatelliteUsage(G01, SatelliteMembership.UNKNOWN)
        != point.usage_reports[0].satellites[0]
    )


def test_dimensionless_nan_uses_the_optional_numeric_disk_convention(
    tmp_path: Path,
) -> None:
    fix = NavFix(
        0, 0, sys_time=T0, hdop=float("nan"), vdop=float("nan"), pdop=float("nan")
    )
    path = tmp_path / "nan.gtd"
    NavFileBuilder().add(fix).finish().write_to_file(path)
    point = NavFile.open(path).points[0]
    assert (point.hdop, point.vdop, point.pdop, point.eph_m) == (None, None, None, None)


def test_canonical_values_and_loaded_points_are_immutable() -> None:
    fix = NavFix(0, 0, sys_time=T0)
    observation = SatelliteObservation(G01)
    usage = SatelliteUsage(G01, SatelliteMembership.USED)
    report = SatelliteUsageReport(
        [usage], scope=GLOBAL, coverage=SatelliteCoverage.PARTIAL, sys_time=T0
    )
    point = NavFileBuilder().add(fix).add(report).finish().points[0]
    values: list[tuple[Any, str]] = [
        (fix, "state"),
        (point, "state"),
        (G01, "prn"),
        (GLOBAL, "is_global"),
        (observation, "snr"),
        (usage, "membership"),
        (report, "coverage"),
    ]
    for value, field in values:
        with pytest.raises(AttributeError):
            setattr(value, field, None)


@pytest.mark.parametrize("clock", ["gps_time", "sys_time"])
def test_navigation_only_recording_preserves_positionless_status(
    clock: str, tmp_path: Path
) -> None:
    builder = NavFileBuilder()
    builder.add(
        NavFix(
            55.67,
            12.56,
            gps_time=T0 if clock == "gps_time" else None,
            sys_time=T0 if clock == "sys_time" else None,
            state=SolutionState.FIX,
            used_count=8,
            hdop=0.9,
        )
    )
    time = T0 + timedelta(seconds=2)
    builder.add(
        NavFix(
            None,
            None,
            gps_time=time if clock == "gps_time" else None,
            sys_time=time if clock == "sys_time" else None,
            state=SolutionState.NO_FIX,
        )
    )
    builder.add(
        NavFix(
            55.671,
            12.562,
            gps_time=T0 + timedelta(seconds=5) if clock == "gps_time" else None,
            sys_time=T0 + timedelta(seconds=5) if clock == "sys_time" else None,
            state=SolutionState.FIX,
            used_count=9,
        )
    )
    path = tmp_path / "navigation_only.gtd"
    builder.finish().write_to_file(path)
    points = NavFile.open(path).points
    assert len(points) == 3
    assert points[0].used_count == 8
    assert points[0].hdop == pytest.approx(0.9)
    assert points[0].eph_m is None
    middle = points[1]
    assert math.isnan(middle.lat) and math.isnan(middle.lon)
    assert middle.state is SolutionState.NO_FIX
    assert middle.position_provenance is None
    assert getattr(middle, clock) == time
    assert getattr(middle, "sys_time" if clock == "gps_time" else "gps_time") is None
    assert (
        middle.used_count,
        middle.heading,
        middle.speed_mps,
        middle.eph_m,
        middle.hdop,
    ) == (None, None, None, None, None)
    for point in points:
        assert point.satellites is None
        assert point.usage_reports == []
        assert point.observation_reports == []
    with pytest.raises(RuntimeError, match="consumed"):
        builder.add(
            NavFix(
                None,
                None,
                gps_time=time if clock == "gps_time" else None,
                sys_time=time if clock == "sys_time" else None,
                state=SolutionState.NO_FIX,
            )
        )


@pytest.mark.parametrize("state", list(SolutionState))
def test_positionless_states_allow_optional_navigation_measurements(
    state: SolutionState,
) -> None:
    fix = NavFix(
        None,
        None,
        sys_time=T0,
        state=state,
        used_count=0,
        hdop=0.9,
        position_provenance=PositionProvenance.SOURCE_PROVIDED,
    )
    assert fix.position_provenance is None
    point = NavFileBuilder().add(fix).finish().points[0]
    assert point.state is state
    assert (point.used_count, point.hdop, point.eph_m) == (0, 0.9, None)


@pytest.mark.parametrize("coordinates", [(None, 0), (0, None)])
def test_navigation_rejects_one_missing_coordinate(
    coordinates: tuple[float | None, float | None],
) -> None:
    with pytest.raises(ValueError, match="lat and lon"):
        NavFix(*coordinates, sys_time=T0, state=SolutionState.NO_FIX)


def test_positionless_navigation_requires_explicit_state_and_aware_timestamp() -> None:
    with pytest.raises(ValueError, match="require state"):
        NavFix(None, None, sys_time=T0)
    with pytest.raises(ValueError, match="provide gps_time or sys_time"):
        NavFix(None, None, state=SolutionState.NO_FIX)
    with pytest.raises(TypeError):
        NavFix(None, None, sys_time=T0.replace(tzinfo=None), state=SolutionState.NO_FIX)
    for state in [1, Constellation.GPS]:
        with pytest.raises(TypeError, match="SolutionState"):
            NavFix(None, None, sys_time=T0, state=state)  # type: ignore[arg-type]


@pytest.mark.parametrize("state", list(SolutionState))
def test_status_only_fix_equality_preserves_recorded_values(
    state: SolutionState,
) -> None:
    fix = NavFix(None, None, sys_time=T0, state=state)
    other = NavFix(None, None, sys_time=T0, state=state)
    before = repr(fix)
    assert fix == other and other == fix
    assert repr(fix) == before
    assert math.isnan(fix.lat) and math.isnan(fix.lon)
    assert fix.position_provenance is None
    assert fix.state is state and fix.sys_time == T0


@pytest.mark.parametrize(
    "changes",
    [
        {"sys_time": T0 + timedelta(seconds=1)},
        {"state": SolutionState.UNKNOWN},
        {"used_count": 0},
        {"used_count": 8},
        {"method": SolutionMethod.RTK_FIXED},
        {"dimension": SolutionDimension.THREE_D},
        {"hdop": 0.9},
        {"vdop": 0.0},
        {"pdop": 1.2},
    ],
)
def test_status_only_fix_equality_compares_other_navigation_facts(
    changes: dict[str, Any],
) -> None:
    arguments: dict[str, Any] = {"sys_time": T0, "state": SolutionState.NO_FIX}
    source = NavFix(None, None, **arguments)
    other = NavFix(None, None, **(arguments | changes))
    assert source != other and other != source


@pytest.mark.parametrize(
    ("coordinates", "state", "origin", "equal"),
    [
        ((math.nan, math.nan), SolutionState.NO_FIX, None, True),
        (
            (math.nan, math.nan),
            SolutionState.NO_FIX,
            PositionProvenance.SOURCE_PROVIDED,
            False,
        ),
        (
            (math.nan, math.nan),
            SolutionState.NO_FIX,
            PositionProvenance.SDK_SYNTHESIZED,
            False,
        ),
        ((math.nan, math.nan), None, None, False),
        ((math.nan, 12.0), SolutionState.NO_FIX, None, False),
        ((55.0, math.nan), SolutionState.NO_FIX, None, False),
    ],
)
def test_raw_nan_fix_equality_requires_the_complete_absence_convention(
    coordinates: tuple[float, float],
    state: SolutionState | None,
    origin: PositionProvenance | None,
    equal: bool,
) -> None:
    arguments: dict[str, Any] = {
        "sys_time": T0,
        "state": state,
        "position_provenance": origin,
    }
    first = NavFix(*coordinates, **arguments)
    second = NavFix(*coordinates, **arguments)
    assert (first == second) is equal
    assert (second == first) is equal
    status = NavFix(None, None, sys_time=T0, state=SolutionState.NO_FIX)
    assert (status == first) is equal
    assert (first == status) is equal


@pytest.mark.parametrize(
    "changed", [None, "count", "state", "usage", "observation", "legacy"]
)
def test_status_only_loaded_point_equality_includes_all_satellite_evidence(
    changed: str | None, tmp_path: Path
) -> None:
    def recording(change: str | None) -> NavFile:
        fix = NavFix(
            None,
            None,
            sys_time=T0,
            state=SolutionState.UNKNOWN if change == "state" else SolutionState.NO_FIX,
            used_count=0 if change == "count" else None,
        )
        builder = NavFileBuilder().add(fix)
        builder.add(
            SatelliteUsageReport(
                [SatelliteUsage(G01, SatelliteMembership.USED)],
                scope=GLOBAL,
                coverage=SatelliteCoverage.PARTIAL,
                reported_count=8 if change == "usage" else None,
                sys_time=T0,
            )
        )
        builder.add(
            SatelliteObservationReport(
                [SatelliteObservation(G01, snr=0 if change == "observation" else None)],
                scope=GLOBAL,
                coverage=SatelliteCoverage.PARTIAL,
                sys_time=T0,
            )
        )
        if change == "legacy":
            builder.add(SatelliteReport([], sys_time=T0))
        return builder.finish()

    source = recording(None)
    other = recording(changed)
    first, second = source.points[0], other.points[0]
    assert (first == second) is (changed is None)
    assert (second == first) is (changed is None)
    path = tmp_path / "status_equality.gtd"
    other.write_to_file(path)
    reopened = NavFile.open(path).points[0]
    assert reopened == second
    assert (first == reopened) is (changed is None)
    assert math.isnan(first.lat) and math.isnan(reopened.lon)
    assert repr(first) == "NavPoint(lat=NaN, lon=NaN)"


def test_source_provided_nan_values_keep_ieee_inequality_after_readback(
    tmp_path: Path,
) -> None:
    first = NavFix(math.nan, math.nan, sys_time=T0, state=SolutionState.NO_FIX)
    second = NavFix(math.nan, math.nan, sys_time=T0, state=SolutionState.NO_FIX)
    assert first.position_provenance is PositionProvenance.SOURCE_PROVIDED
    assert first != second
    source = NavFileBuilder().add(first).finish()
    other = NavFileBuilder().add(second).finish()
    assert source.points[0] != other.points[0]
    path = tmp_path / "malformed.gtd"
    source.write_to_file(path)
    point = NavFile.open(path).points[0]
    assert point != source.points[0]
    assert math.isnan(point.lat) and math.isnan(point.lon)
    assert point.position_provenance is PositionProvenance.SOURCE_PROVIDED
    assert point.state is SolutionState.NO_FIX
