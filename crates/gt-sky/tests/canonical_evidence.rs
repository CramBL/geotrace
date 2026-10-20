use chrono::{DateTime, Duration, Utc};
use rstest::rstest;

use gt_sky::{SkyTrails, TrailMembershipFilter};
use gt_test_utils::Queryable as _;

use gt_types::satellites::{Constellation, ConstellationSet, Prn, SatSample};
use gt_types::{
    GpsTime, Latitude, Longitude, NavPoint, NavigationEvidenceSource, PointIdx, SatelliteCoverage,
    SatelliteIdentity, SatelliteMembership, SatelliteObservation, SatelliteObservationReport,
    SatelliteQueryScope, SatelliteReportMetadata, SatelliteScope, SatelliteUsage,
    SatelliteUsageReport, SkyGeometry, SysTime, TimePositionVelocity,
};

fn time(milliseconds: i64) -> GpsTime {
    GpsTime::from_utc(DateTime::<Utc>::UNIX_EPOCH + Duration::milliseconds(milliseconds))
}

fn point(milliseconds: i64) -> NavPoint {
    NavPoint::new(
        TimePositionVelocity::builder()
            .time(time(milliseconds))
            .lat(Latitude::new(55.0))
            .lon(Longitude::new(12.0))
            .build(),
        None,
    )
}

fn metadata(milliseconds: i64) -> SatelliteReportMetadata {
    SatelliteReportMetadata {
        gps_time: Some(time(milliseconds)),
        sys_time: None,
        normalized_time: time(milliseconds),
        input_order: 0,
        scope: SatelliteScope::Global,
        coverage: SatelliteCoverage::Complete,
        reported_count: None,
    }
}

fn identity(prn: u32) -> SatelliteIdentity {
    SatelliteIdentity {
        constellation: Constellation::Gps,
        prn: Prn::new(prn),
    }
}

fn observation(milliseconds: i64) -> SatelliteObservationReport {
    SatelliteObservationReport {
        metadata: metadata(milliseconds),
        satellites: vec![SatelliteObservation {
            identity: identity(1),
            measurements: SatSample {
                azimuth: Some(45.0),
                elevation: Some(35.0),
                snr: None,
            },
        }],
    }
}

#[test]
fn canonical_rows_preserve_membership_and_missing_geometry() {
    let mut point = point(0);
    point.satellite_evidence.observations.push(observation(0));
    point.satellite_evidence.usage.push(SatelliteUsageReport {
        metadata: metadata(0),
        satellites: vec![
            SatelliteUsage {
                identity: identity(2),
                membership: SatelliteMembership::Used,
            },
            SatelliteUsage {
                identity: identity(3),
                membership: SatelliteMembership::Unknown,
            },
        ],
    });
    let rows = point
        .sky_evidence(SkyGeometry::Identities)
        .expect("display evidence")
        .satellites();
    assert_eq!(
        rows.iter()
            .map(|row| (row.identity.prn.value(), row.membership, row.sky_position()))
            .collect::<Vec<_>>(),
        vec![
            (1, SatelliteMembership::Unused, Some((45.0, 35.0))),
            (2, SatelliteMembership::Used, None),
            (3, SatelliteMembership::Unknown, None),
        ]
    );
    assert_eq!(point.resolved_used_count(SatelliteQueryScope::Global), None);
    point.satellite_evidence.usage.clear();
    let rows = point
        .sky_evidence(SkyGeometry::Identities)
        .expect("observations")
        .satellites();
    assert_eq!(rows[0].membership, SatelliteMembership::Unknown);
}

#[test]
fn sky_counts_preserve_navigation_summary_and_independent_observations() {
    let mut point = point(0);
    point.navigation.used_count = Some(8);
    let mut observations = observation(0);
    observations.satellites = (1..=12)
        .map(|prn| SatelliteObservation {
            identity: identity(prn),
            measurements: observations.satellites[0].measurements,
        })
        .collect();
    point.satellite_evidence.observations.push(observations);
    let evidence = point
        .sky_evidence(SkyGeometry::Position)
        .expect("sky evidence");
    assert_eq!(evidence.counts().used, Some(8));
    assert_eq!(evidence.counts().seen, Some(12));
    assert_eq!(
        evidence.used_count.expect("used").evidence,
        NavigationEvidenceSource::NavigationCount
    );
    assert_eq!(
        evidence.seen_count.expect("seen").evidence,
        NavigationEvidenceSource::ObservationList(0)
    );
    assert!(
        evidence
            .satellites()
            .iter()
            .all(|row| row.membership == SatelliteMembership::Unknown)
    );
}

#[rstest]
#[case::navigation_summary(Some(8), 8, NavigationEvidenceSource::NavigationCount)]
#[case::global_report(None, 9, NavigationEvidenceSource::UsageAggregate(1))]
fn scalar_selection_is_independent_of_geometry_and_scoped_samples(
    #[case] summary: Option<u32>,
    #[case] expected_used: u32,
    #[case] expected_evidence: NavigationEvidenceSource,
) {
    let mut point = point(0);
    point.navigation.used_count = summary;
    let mut scoped = metadata(0);
    scoped.scope = SatelliteScope::Constellations(ConstellationSet::single(Constellation::Gps));
    scoped.reported_count = Some(4);
    let mut global = metadata(400);
    global.reported_count = Some(9);
    point.satellite_evidence.usage = vec![
        SatelliteUsageReport {
            metadata: scoped,
            satellites: vec![],
        },
        SatelliteUsageReport {
            metadata: global,
            satellites: vec![],
        },
    ];
    let mut scoped_observation = observation(0);
    scoped_observation.metadata = scoped;
    let mut global_observation = observation(400);
    global_observation.metadata.reported_count = Some(12);
    point.satellite_evidence.observations = vec![scoped_observation, global_observation];
    let evidence = point
        .sky_evidence(SkyGeometry::Position)
        .expect("sky evidence");
    assert_eq!(evidence.observation.expect("geometry").0, 0);
    assert_eq!(evidence.counts().used, Some(expected_used));
    assert_eq!(
        evidence.used_count.expect("used").evidence,
        expected_evidence
    );
    assert_eq!(evidence.counts().seen, Some(12));
    assert_eq!(
        evidence.seen_count.expect("seen").evidence,
        NavigationEvidenceSource::ObservationAggregate(1)
    );
}

#[test]
fn complementary_scopes_preserve_rows_trail_membership_and_borrowed_context() {
    let mut source = point(0);
    let glonass = SatelliteIdentity {
        constellation: Constellation::Glonass,
        prn: Prn::new(3),
    };
    let glonass_unknown = SatelliteIdentity {
        prn: Prn::new(4),
        ..glonass
    };
    let mut gps_metadata = metadata(0);
    gps_metadata.scope =
        SatelliteScope::Constellations(ConstellationSet::single(Constellation::Gps));
    let mut glonass_metadata = metadata(100);
    glonass_metadata.scope =
        SatelliteScope::Constellations(ConstellationSet::single(Constellation::Glonass));
    source.satellite_evidence.usage = vec![
        SatelliteUsageReport {
            metadata: gps_metadata,
            satellites: vec![
                SatelliteUsage {
                    identity: identity(1),
                    membership: SatelliteMembership::Used,
                },
                SatelliteUsage {
                    identity: identity(2),
                    membership: SatelliteMembership::Unused,
                },
            ],
        },
        SatelliteUsageReport {
            metadata: glonass_metadata,
            satellites: vec![
                SatelliteUsage {
                    identity: glonass,
                    membership: SatelliteMembership::Used,
                },
                SatelliteUsage {
                    identity: glonass_unknown,
                    membership: SatelliteMembership::Unknown,
                },
            ],
        },
    ];
    let mut observations = observation(0);
    observations.satellites.push(SatelliteObservation {
        identity: glonass,
        measurements: observations.satellites[0].measurements,
    });
    source.satellite_evidence.observations.push(observations);
    let evidence = source
        .sky_evidence(SkyGeometry::Position)
        .expect("sky evidence");
    let rows = evidence.satellites();
    assert_eq!(
        rows.iter()
            .map(|row| (row.identity, row.membership, row.usage_report_index))
            .collect::<Vec<_>>(),
        vec![
            (identity(1), SatelliteMembership::Used, Some(0)),
            (identity(2), SatelliteMembership::Unused, Some(0)),
            (glonass, SatelliteMembership::Used, Some(1)),
            (glonass_unknown, SatelliteMembership::Unknown, Some(1)),
        ]
    );
    assert_eq!(
        evidence
            .usage_reports()
            .iter()
            .map(|(index, _)| *index)
            .collect::<Vec<_>>(),
        vec![0, 1]
    );
    let mut target = point(1_000);
    target.navigation.used_count = Some(20);
    let track = gt_test_utils::loaded_track_with_points(vec![source, target]);
    let trails = gt_sky::extract_trails(&track);
    assert!(trails.trails.iter().all(|trail| trail.ever_in_fix()));
    assert_eq!(
        trails
            .trails
            .iter()
            .map(|trail| trail.samples[0].usage_report_index)
            .collect::<Vec<_>>(),
        vec![Some(0), Some(1)]
    );
    assert_eq!(
        trails
            .display_counts_at(
                time(0),
                TrailMembershipFilter {
                    show_not_in_fix: false,
                    in_fix_only: false
                }
            )
            .iter()
            .map(|count| (count.shown, count.used))
            .collect::<Vec<_>>(),
        vec![(1, Some(1)), (1, Some(1))]
    );
    let borrowed = track
        .nearest_sky_evidence(PointIdx::new(1), SkyGeometry::Position)
        .expect("borrowed context");
    assert_eq!(borrowed.point_index, PointIdx::new(0));
    assert_eq!(borrowed.evidence.satellites(), rows);
    assert_eq!(borrowed.evidence.counts().used, None);
    assert_eq!(
        track.points[1]
            .resolved_used_count(SatelliteQueryScope::Global)
            .expect("target count")
            .count,
        20
    );
}

#[rstest]
#[case::explicit_unknown(true, SatelliteMembership::Unknown, 0)]
#[case::partial_omission(false, SatelliteMembership::Used, 1)]
fn membership_selection_requires_identity_evidence(
    #[case] explicit_unknown: bool,
    #[case] expected_membership: SatelliteMembership,
    #[case] expected_report: usize,
) {
    let mut point = point(0);
    let mut unknown_metadata = metadata(0);
    unknown_metadata.coverage = SatelliteCoverage::Partial;
    point.satellite_evidence.usage = vec![
        SatelliteUsageReport {
            metadata: unknown_metadata,
            satellites: if explicit_unknown {
                vec![SatelliteUsage {
                    identity: identity(1),
                    membership: SatelliteMembership::Unknown,
                }]
            } else {
                vec![]
            },
        },
        SatelliteUsageReport {
            metadata: metadata(400),
            satellites: vec![SatelliteUsage {
                identity: identity(1),
                membership: SatelliteMembership::Used,
            }],
        },
    ];
    point.satellite_evidence.observations.push(observation(0));
    let rows = point
        .sky_evidence(SkyGeometry::Position)
        .expect("sky evidence")
        .satellites();
    assert_eq!(rows[0].membership, expected_membership);
    assert_eq!(rows[0].usage_report_index, Some(expected_report));
}

#[rstest]
#[case::at_boundary(20_400, Some(10_000))]
#[case::outside_boundary(20_401, None)]
#[case::fractional_offset(10_000, Some(-400))]
fn borrowing_uses_normalized_report_time(
    #[case] target_ms: i64,
    #[case] expected_age_ms: Option<i64>,
) {
    let mut source = point(10_000);
    let mut report = observation(10_400);
    report.metadata.gps_time = None;
    report.metadata.sys_time = Some(SysTime::from_utc(
        DateTime::<Utc>::UNIX_EPOCH + Duration::hours(1),
    ));
    source.satellite_evidence.observations.push(report);
    let track = gt_test_utils::loaded_track_with_points(vec![source, point(target_ms)]);
    let borrowed = track.nearest_sky_evidence(PointIdx::new(1), SkyGeometry::Position);
    assert_eq!(
        borrowed.map(|report| report.age.num_milliseconds()),
        expected_age_ms
    );
    if let Some(borrowed) = borrowed {
        assert_eq!(borrowed.point_index, PointIdx::new(0));
        assert_eq!(
            borrowed
                .evidence
                .observation
                .expect("observation")
                .1
                .metadata,
            track.points[0].satellite_evidence.observations[0].metadata
        );
        assert_eq!(
            track.points[1].resolved_seen_count(SatelliteQueryScope::Global),
            None
        );
    }
}

#[test]
fn geometry_selection_filters_before_report_ordering() {
    let mut point = point(10_000);
    let mut azimuth_only = observation(10_000);
    azimuth_only.satellites[0].measurements.elevation = None;
    point.satellite_evidence.observations = vec![azimuth_only, observation(10_400)];
    assert_eq!(
        point
            .sky_evidence(SkyGeometry::Azimuth)
            .expect("ring")
            .observation
            .expect("observation")
            .0,
        0
    );
    assert_eq!(
        point
            .sky_evidence(SkyGeometry::Position)
            .expect("disc")
            .observation
            .expect("observation")
            .0,
        1
    );
    point.satellite_evidence.observations.pop();
    assert!(point.supports_sky_geometry(SkyGeometry::Azimuth));
    assert!(!point.supports_sky_geometry(SkyGeometry::Position));
    let trails = gt_sky::extract_trails(&gt_test_utils::loaded_track_with_points(vec![point]));
    assert_eq!(trails.epochs.len(), 1);
    assert_eq!(trails.trails.len(), 0);
}

#[test]
fn independent_samples_keep_scope_time_and_trail_gaps() {
    let mut point = point(0);
    let mut missing_geometry = observation(1_000);
    missing_geometry.satellites[0].measurements.elevation = None;
    let mut unrelated = observation(1_500);
    unrelated.metadata.scope =
        SatelliteScope::Constellations(ConstellationSet::single(Constellation::Glonass));
    unrelated.satellites.clear();
    point.satellite_evidence.observations = vec![
        observation(0),
        missing_geometry,
        unrelated,
        observation(2_000),
        observation(3_000),
        observation(500),
    ];
    let trails = gt_sky::extract_trails(&gt_test_utils::loaded_track_with_points(vec![point]));
    let samples = &trails.trails[0].samples;
    assert_eq!(
        samples.iter().map(|sample| sample.time).collect::<Vec<_>>(),
        vec![time(0), time(2_000), time(3_000), time(500)]
    );
    assert!(!samples[1].epoch.follows(samples[0].epoch));
    assert!(samples[2].epoch.follows(samples[1].epoch));
    assert!(!samples[3].epoch.follows(samples[2].epoch));
    assert_eq!(trails.epochs[3].observation_index, 3);
    assert_eq!(trails.epochs[3].metadata.normalized_time, time(2_000));
    assert!(
        samples
            .iter()
            .all(|sample| sample.membership == SatelliteMembership::Unknown)
    );
}

#[test]
fn an_unrelated_scope_preserves_the_previous_trail_epoch() {
    let mut point = point(0);
    let mut unrelated = observation(1_000);
    unrelated.metadata.scope =
        SatelliteScope::Constellations(ConstellationSet::single(Constellation::Glonass));
    unrelated.satellites.clear();
    point.satellite_evidence.observations = vec![observation(0), unrelated, observation(2_000)];
    let trails = gt_sky::extract_trails(&gt_test_utils::loaded_track_with_points(vec![point]));
    assert!(
        trails.trails[0].samples[1]
            .epoch
            .follows(trails.trails[0].samples[0].epoch)
    );
}

#[rstest]
#[case::dark_observations("sky_partial_membership_dark", true, true)]
#[case::light_observations("sky_partial_membership_light", false, true)]
#[case::dark_identities("sky_identities_only_dark", true, false)]
#[case::light_identities("sky_identities_only_light", false, false)]
fn partial_sky_evidence_shows_membership(
    #[case] snapshot: &str,
    #[case] dark_mode: bool,
    #[case] observations: bool,
) {
    let mut point = point(0);
    let mut usage = SatelliteUsageReport {
        metadata: metadata(0),
        satellites: vec![
            SatelliteUsage {
                identity: identity(1),
                membership: SatelliteMembership::Used,
            },
            SatelliteUsage {
                identity: identity(2),
                membership: SatelliteMembership::Unused,
            },
            SatelliteUsage {
                identity: identity(3),
                membership: SatelliteMembership::Unknown,
            },
        ],
    };
    usage.metadata.coverage = SatelliteCoverage::Partial;
    usage.metadata.reported_count = Some(8);
    point.satellite_evidence.usage.push(usage);
    if observations {
        let mut observation = observation(0);
        observation.metadata.coverage = SatelliteCoverage::Partial;
        observation.metadata.reported_count = Some(12);
        observation.satellites.extend([
            SatelliteObservation {
                identity: identity(2),
                measurements: SatSample {
                    azimuth: Some(210.0),
                    elevation: Some(60.0),
                    snr: None,
                },
            },
            SatelliteObservation {
                identity: identity(3),
                measurements: SatSample {
                    azimuth: Some(310.0),
                    elevation: Some(25.0),
                    snr: None,
                },
            },
            SatelliteObservation {
                identity: identity(4),
                measurements: SatSample {
                    azimuth: Some(90.0),
                    elevation: None,
                    snr: None,
                },
            },
        ]);
        point.satellite_evidence.observations.push(observation);
    }
    let mut harness = gt_test_utils::TestHarness::builder()
        .theme(dark_mode)
        .size(egui::vec2(330.0, 400.0))
        .ui(move |ui| {
            gt_sky::SkyPlot::from_evidence(
                point
                    .sky_evidence(SkyGeometry::Identities)
                    .expect("display evidence"),
                gt_sky::SkyPlotSize::Full,
            )
            .ui(ui);
        });
    harness.run();
    let expected_counts = if observations {
        "Satellites used / seen 8 / 12"
    } else {
        "Satellites used / seen 8 / —"
    };
    assert!(harness.inner.query_by_label(expected_counts).is_some());
    assert_eq!(
        harness.inner.query_all_by_label("No sky positions").count(),
        1
    );
    let grid_rims = harness
        .inner
        .output()
        .shapes
        .iter()
        .filter(
            |shape| matches!(&shape.shape, egui::Shape::Circle(circle) if circle.radius > 100.0),
        )
        .count();
    assert_eq!(grid_rims > 0, observations);
    if !observations {
        for label in [
            "G01 GPS",
            "G02 GPS",
            "G03 GPS",
            "Used",
            "Unused",
            "Usage unknown",
        ] {
            let rect = harness.inner.get_by_label(label).rect();
            assert!(
                rect.bottom() < 150.0,
                "identity-only section exceeds its compact allocation"
            );
        }
    }
    let diamonds: Vec<_> = harness
        .inner
        .output()
        .shapes
        .iter()
        .filter_map(|shape| match &shape.shape {
            egui::Shape::Path(path) if path.closed && path.points.len() == 4 => Some(path),
            _ => None,
        })
        .collect();
    assert_eq!(diamonds.len(), if observations { 2 } else { 1 });
    assert!(
        diamonds
            .iter()
            .all(|diamond| diamond.visual_bounding_rect().width() >= 4.0)
    );
    harness.snapshot(snapshot);
}

#[rstest]
#[case::dark(true)]
#[case::light(false)]
fn compact_observations_without_usage_have_visible_unknown_diamonds(#[case] dark_mode: bool) {
    let mut point = point(0);
    point.satellite_evidence.observations.push(observation(0));
    let mut harness = gt_test_utils::TestHarness::builder()
        .theme(dark_mode)
        .size(egui::vec2(170.0, 180.0))
        .ui(move |ui| {
            gt_sky::SkyPlot::from_evidence(
                point.sky_evidence(SkyGeometry::Position).expect("position"),
                gt_sky::SkyPlotSize::Compact,
            )
            .ui(ui);
        });
    harness.run();
    assert!(
        harness
            .inner
            .query_by_label("Satellites used / seen — / 1")
            .is_some()
    );
    let diamonds: Vec<_> = harness
        .inner
        .output()
        .shapes
        .iter()
        .filter_map(|shape| match &shape.shape {
            egui::Shape::Path(path) if path.closed && path.points.len() == 4 => Some(path),
            _ => None,
        })
        .collect();
    assert_eq!(diamonds.len(), 1);
    assert!((diamonds[0].visual_bounding_rect().width() - 4.0).abs() < f32::EPSILON);
    harness.snapshot(if dark_mode {
        "compact_unknown_observations_dark"
    } else {
        "compact_unknown_observations_light"
    });
}

struct IndependentPolling {
    glonass_membership: SatelliteMembership,
}

impl IndependentPolling {
    fn points(&self) -> Vec<NavPoint> {
        (0..4)
            .map(|second| {
                let constellation = if second % 2 == 0 {
                    Constellation::Gps
                } else {
                    Constellation::Glonass
                };
                let membership = if constellation == Constellation::Gps {
                    SatelliteMembership::Used
                } else {
                    self.glonass_membership
                };
                let mut point = point(second * 1_000);
                let mut report = observation(second * 1_000);
                report.metadata.scope =
                    SatelliteScope::Constellations(ConstellationSet::single(constellation));
                let satellite_identity = SatelliteIdentity {
                    constellation,
                    prn: Prn::new(if constellation == Constellation::Gps {
                        1
                    } else {
                        3
                    }),
                };
                for satellite in &mut report.satellites {
                    satellite.identity = satellite_identity;
                    satellite.measurements.azimuth = Some(40.0 + second as f32 * 10.0);
                }
                point.satellite_evidence.usage.push(SatelliteUsageReport {
                    metadata: report.metadata,
                    satellites: vec![SatelliteUsage {
                        identity: satellite_identity,
                        membership,
                    }],
                });
                point.satellite_evidence.observations.push(report);
                point
            })
            .collect()
    }

    fn trails(&self) -> SkyTrails {
        gt_sky::extract_trails(&gt_test_utils::loaded_track_with_points(self.points()))
    }
}

#[rstest]
#[case::exact_other_scope(1_000, SatelliteMembership::Unused, true, false, vec![(Constellation::Gps, 1, Some(1)), (Constellation::Glonass, 1, Some(0))])]
#[case::between_scopes(1_500, SatelliteMembership::Unused, true, false, vec![(Constellation::Gps, 1, Some(1)), (Constellation::Glonass, 1, Some(0))])]
#[case::gps_finished(2_500, SatelliteMembership::Unused, true, false, vec![(Constellation::Gps, 0, Some(0)), (Constellation::Glonass, 1, Some(0))])]
#[case::unknown(1_500, SatelliteMembership::Unknown, true, false, vec![(Constellation::Gps, 1, Some(1)), (Constellation::Glonass, 1, None)])]
#[case::hide_known_unused(1_500, SatelliteMembership::Unused, false, false, vec![(Constellation::Gps, 1, Some(1)), (Constellation::Glonass, 0, Some(0))])]
#[case::keep_unknown(1_500, SatelliteMembership::Unknown, false, false, vec![(Constellation::Gps, 1, Some(1)), (Constellation::Glonass, 1, None)])]
#[case::only_used(1_500, SatelliteMembership::Unknown, false, true, vec![(Constellation::Gps, 1, Some(1)), (Constellation::Glonass, 0, Some(0))])]
fn displayed_markers_and_counts_share_polling_times_and_membership_filters(
    #[case] milliseconds: i64,
    #[case] membership: SatelliteMembership,
    #[case] show_not_in_fix: bool,
    #[case] in_fix_only: bool,
    #[case] expected: Vec<(Constellation, usize, Option<usize>)>,
) {
    let trails = IndependentPolling {
        glonass_membership: membership,
    }
    .trails();
    let filter = TrailMembershipFilter {
        show_not_in_fix,
        in_fix_only,
    };
    let counts = trails.display_counts_at(time(milliseconds), filter);
    assert_eq!(
        counts
            .iter()
            .map(|count| (count.constellation, count.shown, count.used))
            .collect::<Vec<_>>(),
        expected
    );
    let markers = trails
        .trails
        .iter()
        .filter_map(|trail| {
            trail
                .marker_at(time(milliseconds))
                .filter(|marker| filter.allows(trail, *marker))
                .map(|marker| {
                    (
                        trail.constellation,
                        trail.prn.value(),
                        marker.sample.membership,
                    )
                })
        })
        .collect::<Vec<_>>();
    let expected_markers = expected
        .iter()
        .filter(|(_, shown, _)| *shown > 0)
        .map(|(constellation, _, _)| {
            if *constellation == Constellation::Gps {
                (*constellation, 1, SatelliteMembership::Used)
            } else {
                (*constellation, 3, membership)
            }
        })
        .collect::<Vec<_>>();
    assert_eq!(markers, expected_markers);
    for count in counts {
        let unfiltered =
            usize::from(count.constellation != Constellation::Gps || milliseconds <= 2_000);
        assert_eq!(count.before_membership_filters, unfiltered);
    }
}

#[rstest]
#[case::missing_geometry(false)]
#[case::omitted_identity(true)]
fn relevant_reports_break_display_interpolation(#[case] omitted: bool) {
    let mut points = IndependentPolling {
        glonass_membership: SatelliteMembership::Unused,
    }
    .points();
    let mut missing = points[0].clone();
    missing.tpv = point(1_000).tpv;
    let report = &mut missing.satellite_evidence.observations[0];
    report.metadata = SatelliteReportMetadata {
        scope: report.metadata.scope,
        ..metadata(1_000)
    };
    if omitted {
        report.satellites.clear();
    } else {
        report.satellites[0].measurements.elevation = None;
    }
    points.insert(2, missing);
    let trails = gt_sky::extract_trails(&gt_test_utils::loaded_track_with_points(points));
    let gps = trails
        .trails
        .iter()
        .find(|trail| trail.constellation == Constellation::Gps)
        .expect("GPS trail");
    assert!(gps.marker_at(time(1_500)).is_none());
    let counts = trails.display_counts_at(
        time(1_500),
        TrailMembershipFilter {
            show_not_in_fix: true,
            in_fix_only: false,
        },
    );
    assert_eq!((counts[0].shown, counts[0].used), (0, Some(0)));
}

#[test]
fn displayed_geometry_count_is_independent_of_reported_seen_count() {
    let mut point = point(0);
    let mut report = observation(0);
    report.metadata.reported_count = Some(12);
    let mut second = report.satellites[0];
    second.identity = identity(2);
    report.satellites.push(second);
    point.satellite_evidence.observations.push(report);
    assert_eq!(
        point
            .resolved_seen_count(SatelliteQueryScope::Global)
            .expect("aggregate")
            .count,
        12
    );
    let trails = gt_sky::extract_trails(&gt_test_utils::loaded_track_with_points(vec![point]));
    let count = trails.display_counts_at(
        time(0),
        TrailMembershipFilter {
            show_not_in_fix: true,
            in_fix_only: false,
        },
    )[0];
    assert_eq!((count.shown, count.used), (2, None));
}

#[rstest]
#[case::first_exact(0, 0, SatelliteMembership::Used, 40.0)]
#[case::first_bracketing_pair(1_000, 0, SatelliteMembership::Used, 50.0)]
#[case::later_exact(2_000, 2_000, SatelliteMembership::Unused, 60.0)]
fn repeated_times_select_the_first_recorded_sample_or_pair(
    #[case] scrub_ms: i64,
    #[case] source_ms: i64,
    #[case] membership: SatelliteMembership,
    #[case] azimuth: f32,
) {
    let mut points = IndependentPolling {
        glonass_membership: SatelliteMembership::Unused,
    }
    .points();
    points.retain(|point| {
        point.satellite_evidence.observations[0].satellites[0]
            .identity
            .constellation
            == Constellation::Gps
    });
    points[1].satellite_evidence.usage[0].satellites[0].membership = SatelliteMembership::Unused;
    let mut repeated = points.clone();
    repeated[0].satellite_evidence.usage[0].satellites[0].membership = SatelliteMembership::Unknown;
    points.extend(repeated);
    let trails = gt_sky::extract_trails(&gt_test_utils::loaded_track_with_points(points));
    let marker = trails.trails[0]
        .marker_at(time(scrub_ms))
        .expect("selected marker");
    assert_eq!(
        (marker.sample.time, marker.sample.membership, marker.azimuth),
        (time(source_ms), membership, azimuth)
    );
    let count = trails.display_counts_at(
        time(scrub_ms),
        TrailMembershipFilter {
            show_not_in_fix: true,
            in_fix_only: true,
        },
    )[0];
    assert_eq!(
        count.shown,
        usize::from(membership == SatelliteMembership::Used)
    );
}

#[test]
fn backward_time_boundary_breaks_interpolation_between_forward_samples() {
    let mut points = IndependentPolling {
        glonass_membership: SatelliteMembership::Unused,
    }
    .points();
    points.swap(2, 3);
    let trails = gt_sky::extract_trails(&gt_test_utils::loaded_track_with_points(points));
    assert!(trails.trails[0].marker_at(time(1_500)).is_none());
    let count = trails.display_counts_at(
        time(1_500),
        TrailMembershipFilter {
            show_not_in_fix: true,
            in_fix_only: false,
        },
    )[0];
    assert_eq!(
        (count.shown, count.used, count.before_membership_filters),
        (0, Some(0), 0)
    );
}
