//! What a run reports about the tracks it read: the samples the providers hand
//! the evaluator, the point ranges the map paints for them, and the staleness
//! flag that grays a run out once its inputs change.

#![expect(
    clippy::expect_used,
    reason = "the fixture helpers beside the tests are not covered by clippy's in-test relaxations"
)]

use std::ops::Range;
use std::sync::Arc;

use chrono::{DateTime, Utc};
use geotrace_sdk::{Angle, NavFileBuilder, NavFix};
use geotrace_sdk_units::ChannelUnit;
use gt_filter::GlobalFilter;
use gt_loaded_files::{FileHistory, LoadedFiles};
use gt_query::{ChannelSchema, MetricProvider};
use gt_query_run::{
    JammingValues, QuerySession, RunInputs, SnapErrorValues, TrackProvider, schema_from_files,
};
use gt_types::coordinates::{Latitude, Longitude};
use gt_types::time_types::GpsTime;
use gt_types::tpv::TimePositionVelocity;
use gt_types::track::{TimeRange, TrackMetadata};
use gt_types::{
    Channel, FileIdx, FileMetadata, FileSource, LoadedFile, LoadedTrack, NavPoint, TrackIdx,
    TrackRef,
};
use gt_ui_types::{GeomagneticSeries, TecSeries, TrackDataVisibility};
use rustc_hash::FxHashMap;
use uom::si::f64::Velocity;
use uom::si::velocity::kilometer_per_hour;

/// The Unix second the fixtures start at.
const EPOCH: i64 = 1_700_000_000;

/// The one track every fixture builds.
fn track_zero() -> TrackRef {
    TrackRef::new(FileIdx::new(0), TrackIdx::new(0))
}

fn utc(millis: i64) -> DateTime<Utc> {
    DateTime::from_timestamp_millis(EPOCH * 1_000 + millis).expect("a valid timestamp")
}

/// One fix per millisecond offset past [`EPOCH`], each at 36 km/h.
fn fixes_at(offsets_millis: &[i64]) -> Vec<NavPoint> {
    let at_one_speed: Vec<(i64, f64)> = offsets_millis
        .iter()
        .map(|&millis| (millis, 36.0))
        .collect();
    fixes_at_speeds(&at_one_speed)
}

/// One fix per millisecond offset past [`EPOCH`] and speed in km/h, moving north
/// so the track has a length.
fn fixes_at_speeds(fixes: &[(i64, f64)]) -> Vec<NavPoint> {
    fixes
        .iter()
        .map(|&(millis, speed_kmh)| {
            let tpv = TimePositionVelocity::builder()
                .time(GpsTime::from_utc(utc(millis)))
                .lat(Latitude::new(55.0 + millis as f64 / 1_000_000.0))
                .lon(Longitude::new(12.0))
                .velocity(Velocity::new::<kilometer_per_hour>(speed_kmh))
                .build();
            NavPoint::new(tpv, None)
        })
        .collect()
}

fn utc_micros(micros: i64) -> DateTime<Utc> {
    DateTime::from_timestamp_micros(EPOCH * 1_000_000 + micros).expect("a valid timestamp")
}

/// A one-track file written as `.gtd` bytes and read back through the loader.
/// Three fixes sit a second apart, each stamped by the receiver on the whole
/// second and by the host clock `micros_ahead` later.
fn gtd_file_with_the_host_clock_ahead(micros_ahead: i64) -> LoadedFile {
    let mut recorder = NavFileBuilder::new().open();
    for second in 0..3 {
        let receiver_micros = second * 1_000_000;
        recorder.add_nav_fix(
            NavFix::builder()
                .gps_time(utc_micros(receiver_micros))
                .sys_time(utc_micros(receiver_micros + micros_ahead))
                .lat(Angle::degrees(55.0 + second as f64 / 1_000.0))
                .lon(Angle::degrees(12.0))
                .build(),
        );
    }
    let mut bytes = Vec::new();
    recorder
        .finish()
        .expect("the recorded fixes are valid")
        .write(&mut bytes)
        .expect("the file writes to memory");
    gt_loader::load_bytes(&bytes, "ride.gtd".to_owned()).expect("the written file loads")
}

/// A scalar channel sampled at the given millisecond offsets past [`EPOCH`].
fn scalar_channel(name: &str, unit: Option<&str>, samples: &[(i64, f64)]) -> Channel {
    Channel {
        name: name.to_owned(),
        unit: unit.map(ChannelUnit::from_file_label),
        period: None,
        description: None,
        components: vec![],
        times: samples.iter().map(|&(millis, _)| utc(millis)).collect(),
        values: samples.iter().map(|&(_, value)| value).collect(),
    }
}

/// A one-track file named `filename`, carrying `points` and `channels`. The
/// track metadata spans the fixes, since the global filter reads it.
fn file_named(filename: &str, points: Vec<NavPoint>, channels: Vec<Channel>) -> LoadedFile {
    let first = points
        .first()
        .map_or_else(Default::default, |p: &NavPoint| p.tpv.time().utc());
    let metadata = TrackMetadata {
        time_range: TimeRange::spanning(first, points.iter().map(|p| p.tpv.time().utc())),
        tpv_count: points.len(),
        ..gt_test_utils::empty_track_metadata()
    };
    LoadedFile {
        metadata: FileMetadata {
            filename: filename.to_owned(),
            ..gt_test_utils::empty_file_metadata()
        },
        tracks: vec![LoadedTrack {
            channels,
            metadata,
            ..gt_test_utils::loaded_track_with_points(points)
        }],
        event_marker_styles: FxHashMap::default(),
        orphaned_event_markers: vec![],
        source: FileSource::GtdBytes(Arc::from(Vec::<u8>::new())),
        load_warnings: vec![],
    }
}

/// The loaded state a session runs against, owned so the borrowed [`RunInputs`]
/// can be rebuilt per call.
struct LoadedState {
    files: LoadedFiles,
    visibility: TrackDataVisibility,
    filter: GlobalFilter,
    snap_errors: SnapErrorValues,
    jamming: JammingValues,
    geomagnetic: GeomagneticSeries,
    tec: TecSeries,
}

impl LoadedState {
    fn of(file: LoadedFile) -> Self {
        let mut files = LoadedFiles::new();
        files.push(file, FileHistory::None);
        let visibility = TrackDataVisibility::from_loaded(files.files());
        Self {
            files,
            visibility,
            filter: GlobalFilter::default(),
            snap_errors: SnapErrorValues::default(),
            jamming: JammingValues::default(),
            geomagnetic: GeomagneticSeries::default(),
            tec: TecSeries::default(),
        }
    }

    /// Unload the loaded file, then load `file` in its place.
    fn replace_with(&mut self, file: LoadedFile) {
        self.files.remove_file(0);
        self.files.push(file, FileHistory::None);
        self.visibility = TrackDataVisibility::from_loaded(self.files.files());
    }

    fn inputs(&self) -> RunInputs<'_> {
        RunInputs {
            loaded_files: self.files.view(),
            visibility: &self.visibility,
            filter: &self.filter,
            snap_errors: &self.snap_errors,
            jamming: &self.jamming,
            geomagnetic: &self.geomagnetic,
            tec: &self.tec,
        }
    }

    fn schema(&self) -> ChannelSchema {
        schema_from_files(self.files.files())
    }
}

/// Drive one run of `text` to completion, the way a headless caller does.
fn run_text(session: &mut QuerySession, state: &LoadedState, text: &str) {
    session.set_text(text.to_owned());
    session.sync_checks(&state.schema());
    let prepared = session
        .start_run(state.inputs())
        .expect("the query checks and nothing is in flight");
    session.finish_run(prepared.execute());
}

/// The hidden point ranges of the last run, for the one loaded track.
fn hidden_ranges(session: &QuerySession) -> Vec<Range<usize>> {
    session
        .matches()
        .expect("a completed run")
        .hidden_ranges(track_zero())
        .to_vec()
}

/// The first draw layer's point ranges of the last run, for the one loaded
/// track.
fn drawn_ranges(session: &QuerySession) -> Vec<Range<usize>> {
    session
        .matches()
        .expect("a completed run")
        .draws
        .first()
        .expect("a query that draws")
        .ranges_for(track_zero())
        .to_vec()
}

#[test]
fn a_channel_keep_query_hides_a_track_with_no_match() {
    let channel = scalar_channel("accel", Some("g"), &[(0, 0.1), (500, 0.2)]);
    let state = LoadedState::of(file_named(
        "ride.gtd",
        fixes_at(&[0, 1_000, 2_000, 3_000]),
        vec![channel],
    ));
    let mut session = QuerySession::new();

    run_text(&mut session, &state, "@accel | where @accel > 5 g | keep");

    assert_eq!(hidden_ranges(&session), vec![0..4]);
}

#[test]
fn a_points_keep_query_hides_a_track_with_no_match() {
    let state = LoadedState::of(file_named(
        "ride.gtd",
        fixes_at(&[0, 1_000, 2_000, 3_000]),
        vec![],
    ));
    let mut session = QuerySession::new();

    run_text(
        &mut session,
        &state,
        "points | where velocity > 500 km/h | keep",
    );

    assert_eq!(hidden_ranges(&session), vec![0..4]);
}

/// The samples of a closed span are the ones whose timestamp lands inside it,
/// whatever order the file stored them in. Only the sample at 1 s does here.
#[test]
fn a_channel_span_holds_only_the_samples_inside_it_when_the_file_stored_them_out_of_order() {
    let channel = scalar_channel("sensor", None, &[(0, 0.0), (2_000, 20.0), (1_000, 10.0)]);
    let points = fixes_at(&[0, 1_000, 2_000, 3_000]);
    let provider = TrackProvider::new(&points, std::slice::from_ref(&channel), None);

    let span = provider.channel_span("sensor", EPOCH as f64 + 0.5, EPOCH as f64 + 1.5);

    assert_eq!(span.values, vec![10.0]);
}

/// A window aggregate reads the samples of the window's time span, whatever
/// order the file stored them in. The two windows holding the sample at 1 s
/// match (points 0 and 1, and points 1 and 2): it is the only sample above the
/// bar.
#[test]
fn a_window_aggregate_reads_a_channel_the_file_stored_out_of_order() {
    let channel = scalar_channel("sensor", None, &[(0, 0.0), (2_000, 2.0), (1_000, 10.0)]);
    let state = LoadedState::of(file_named(
        "ride.gtd",
        fixes_at(&[0, 1_000, 2_000, 3_000]),
        vec![channel],
    ));
    let mut session = QuerySession::new();

    run_text(
        &mut session,
        &state,
        "points | window 2 | where max(@sensor) > 5 | draw",
    );

    assert_eq!(drawn_ranges(&session), vec![0..3]);
}

/// Both fixes of this 2 Hz track fall in the same whole second, with the
/// matched sample at 250 ms between them.
#[test]
fn a_matched_sample_covers_the_two_fixes_around_it_at_two_hz() {
    let channel = scalar_channel("accel", Some("g"), &[(250, 9.0)]);
    let state = LoadedState::of(file_named("ride.gtd", fixes_at(&[0, 500]), vec![channel]));
    let mut session = QuerySession::new();

    run_text(&mut session, &state, "@accel | where @accel > 1 g | draw");

    assert_eq!(drawn_ranges(&session), vec![0..2]);
}

/// Nothing sorts a recording's fixes, so a backward time step can leave a fix
/// stamped past the window between two fixes inside it. The run evaluates the
/// two inside the window, and the one outside it falls in no match.
#[test]
fn a_time_window_keeps_the_fixes_on_both_sides_of_a_backward_time_step() {
    let mut state = LoadedState::of(file_named(
        "ride.gtd",
        fixes_at(&[0, 10_000, 1_000, 2_000]),
        vec![],
    ));
    state.filter = GlobalFilter {
        time_end: Some(utc(5_000)),
        ..GlobalFilter::default()
    };
    let mut session = QuerySession::new();

    run_text(
        &mut session,
        &state,
        "points | where velocity > 1 km/h | draw",
    );

    assert_eq!(drawn_ranges(&session), vec![0..1, 2..4]);
}

/// `accel` at the second fix of this 2 Hz track is 20 m/s2. That fix is 0.5 s
/// and 10 m/s past the first. The first fix has no predecessor to difference
/// against.
#[test]
fn accel_is_valued_between_two_fixes_in_the_same_second() {
    let state = LoadedState::of(file_named(
        "ride.gtd",
        fixes_at_speeds(&[(0, 36.0), (500, 72.0)]),
        vec![],
    ));
    let mut session = QuerySession::new();

    run_text(
        &mut session,
        &state,
        "points | where accel > 15 m/s2 | draw",
    );

    assert_eq!(drawn_ranges(&session), vec![1..2]);
}

/// A count window's channel span holds the samples between its own two fixes.
/// A window over this 2 Hz track spans half a second. The window at fixes 0 and
/// 1 reads the sample at 0 ms and matches. The window at fixes 1 and 2 reads
/// only the sample at 750 ms, which is under the bar.
#[test]
fn a_count_window_reads_the_channel_samples_between_its_own_fixes_at_two_hz() {
    let channel = scalar_channel("sensor", None, &[(0, 100.0), (750, 1.0)]);
    let state = LoadedState::of(file_named(
        "ride.gtd",
        fixes_at(&[0, 500, 1_000, 1_500]),
        vec![channel],
    ));
    let mut session = QuerySession::new();

    run_text(
        &mut session,
        &state,
        "points | window 2 | where max(@sensor) > 50 | draw",
    );

    assert_eq!(drawn_ranges(&session), vec![0..2]);
}

/// A 1 s duration window over this 2 Hz track holds two fixes. The window at
/// fix 0 averages 50 km/h over fixes 0 and 1, and the window at fix 1 averages
/// 50 km/h over fixes 1 and 2. The windows at fixes 2 and 3 run past the last
/// fix.
#[test]
fn a_duration_window_groups_the_fixes_of_its_own_span_at_two_hz() {
    let state = LoadedState::of(file_named(
        "ride.gtd",
        fixes_at_speeds(&[(0, 100.0), (500, 0.0), (1_000, 100.0), (1_500, 0.0)]),
        vec![],
    ));
    let mut session = QuerySession::new();

    run_text(
        &mut session,
        &state,
        "points | window 1 s | where avg(velocity) > 30 km/h | draw",
    );

    assert_eq!(drawn_ranges(&session), vec![0..3]);
}

#[test]
fn results_go_stale_when_another_file_of_the_same_name_replaces_the_loaded_one() {
    let mut state = LoadedState::of(file_named("ride.gtd", fixes_at(&[0, 1_000]), vec![]));
    let mut session = QuerySession::new();
    run_text(
        &mut session,
        &state,
        "points | where velocity > 1 km/h | draw",
    );

    state.replace_with(file_named(
        "ride.gtd",
        fixes_at(&[10_000, 11_000, 12_000, 13_000]),
        vec![],
    ));
    session.refresh_staleness(state.inputs());

    assert!(session.results().expect("a completed run").stale());
}

#[test]
fn sys_time_reads_past_time_for_a_fix_whose_host_clock_is_500_microseconds_ahead() {
    let state = LoadedState::of(gtd_file_with_the_host_clock_ahead(500));
    let mut session = QuerySession::new();

    run_text(
        &mut session,
        &state,
        "points | where time < sys_time | draw",
    );

    assert_eq!(drawn_ranges(&session), vec![0..3]);
}

#[test]
fn clock_delta_reads_below_zero_for_a_fix_whose_host_clock_is_500_microseconds_ahead() {
    let state = LoadedState::of(gtd_file_with_the_host_clock_ahead(500));
    let mut session = QuerySession::new();

    run_text(
        &mut session,
        &state,
        "points | where clock_delta < 0 s | draw",
    );

    assert_eq!(drawn_ranges(&session), vec![0..3]);
}
