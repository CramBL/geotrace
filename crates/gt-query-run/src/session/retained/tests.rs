use std::sync::Arc;

use chrono::DateTime;
use gt_query::QueryMetric;
use gt_types::{FileIdx, FixRef, PointIdx, TrackIdx, TrackRef};

use super::{
    MatchExtent, MatchPosition, ResultTime, RetainedColumn, RetainedResultError, RowColumn,
};
use crate::session::QuerySession;
use crate::test_util::{self, LoadedState, TEST_EPOCH};

#[test]
fn retained_results_require_the_completed_recording_generation() {
    let file = || test_util::file_with_channels(Vec::new());
    let mut state = LoadedState::of(file());
    state.filter = state
        .filter
        .with_time_bounds(DateTime::from_timestamp(TEST_EPOCH + 1, 0), None);
    let mut session = QuerySession::new();
    test_util::run_text(&mut session, &state, "points");
    let position = MatchPosition::default();
    let track = TrackRef::new(FileIdx::new(0), TrackIdx::new(0));
    {
        let retained = session
            .retained_results(state.inputs().loaded_files)
            .unwrap();
        let matched = retained.query(0).unwrap().match_at(position).unwrap();
        assert_eq!(matched.track(), track);
        assert_eq!(matched.extent(), &MatchExtent::Points(1..2));
        let rows = matched.rows(None).unwrap();
        let row = rows.iter().next().unwrap();
        assert_eq!(row.index(), 1);
        assert_eq!(row.fix(), Some(FixRef::new(track, PointIdx::new(1))));
        assert!(rows.iter().nth(usize::MAX).is_none());
        assert!(matches!(
            matched.rows(Some(0)),
            Err(RetainedResultError::InvalidPosition)
        ));
    }

    state.replace_with(file());
    let replacement = state.inputs().loaded_files;
    assert!(track.resolve(replacement.files()).is_some());
    assert!(!session.results().unwrap().stale());
    assert!(matches!(
        session.retained_results(replacement),
        Err(RetainedResultError::Stale)
    ));
    assert!(matches!(
        session.displayed_results(replacement),
        Err(RetainedResultError::Stale)
    ));
}

#[test]
fn display_rows_read_the_original_series_after_a_snap_change() {
    let mut state = LoadedState::with_channels(Vec::new());
    let track = TrackRef::new(FileIdx::new(0), TrackIdx::new(0));
    state
        .snap_errors
        .insert(track, Arc::new(vec![Some(2.0), None]));
    let mut session = QuerySession::new();
    test_util::run_text(&mut session, &state, "points | table snap_error");

    state
        .snap_errors
        .insert(track, Arc::new(vec![Some(9.0), None]));
    session.refresh_staleness(state.inputs());
    let files = state.inputs().loaded_files;
    assert!(matches!(
        session.retained_results(files),
        Err(RetainedResultError::Stale)
    ));
    let display = session.displayed_results(files).unwrap();
    let query = display.queries().next().unwrap();
    let matched = query.matches().next().unwrap();
    let rows = matched.rows(None).unwrap();
    assert_eq!(rows.iter().next().unwrap().value(1), Some(2.0));
}

#[test]
fn a_session_keeps_its_current_run_when_a_superseded_run_finishes() {
    let file = || test_util::file_with_channels(Vec::new());
    let mut state = LoadedState::of(file());
    let mut session = QuerySession::new();
    session.set_text("points".to_owned());
    session.sync_checks(&state.schema());
    let abandoned = session.start_run(state.inputs()).unwrap();
    session.abandon_run();

    state.replace_with(file());
    let current = session.start_run(state.inputs()).unwrap();
    session.finish_run(abandoned.execute());
    assert!(session.run_in_flight());
    assert!(matches!(
        session.retained_results(state.inputs().loaded_files),
        Err(RetainedResultError::Unavailable)
    ));

    session.finish_run(current.execute());
    assert!(!session.run_in_flight());
    session
        .retained_results(state.inputs().loaded_files)
        .unwrap();
}

#[test]
fn a_channel_match_extent_uses_the_retained_filtered_timeline() {
    let channel = test_util::scalar_channel(
        "sensor",
        None,
        &[(0, 8.0), (1, 1.0), (2, 2.0), (3, 7.0), (4, 9.0)],
    );
    let mut state = LoadedState::with_channels(vec![channel]);
    state.filter = state.filter.with_time_bounds(
        DateTime::from_timestamp(TEST_EPOCH + 1, 0),
        DateTime::from_timestamp(TEST_EPOCH + 3, 0),
    );
    let mut session = QuerySession::new();
    test_util::run_text(&mut session, &state, "@sensor | where @sensor > 5");
    let results = session
        .retained_results(state.inputs().loaded_files)
        .unwrap();
    let matched = results
        .query(0)
        .unwrap()
        .match_at(MatchPosition::default())
        .unwrap();
    assert_eq!(
        matched.extent(),
        &MatchExtent::Channel {
            name: "sensor",
            times: &[(TEST_EPOCH + 3) as f64]
        }
    );
    assert_eq!(matched.range(), 2..3);
    let rows = matched.rows(Some(2)).unwrap();
    let row = (&rows).into_iter().next().unwrap();
    assert_eq!(
        row.time(),
        Some(ResultTime::ChannelSeconds((TEST_EPOCH + 3) as f64))
    );
    assert_eq!(row.value(0), Some(7.0));
    assert_eq!(row.fix(), None);
    assert!(matches!(
        matched.rows(Some(3)),
        Err(RetainedResultError::InvalidPosition)
    ));
}

#[test]
fn retained_point_columns_preserve_ordinals_and_values() {
    let state = LoadedState::with_channels(Vec::new());
    let mut session = QuerySession::new();
    test_util::run_text(
        &mut session,
        &state,
        "points | window 2 | table eph, max(lat), velocity, min(lat)",
    );
    let results = session
        .retained_results(state.inputs().loaded_files)
        .unwrap();
    let query = results.query(0).unwrap();
    let matched = query.match_at(query.first_match().unwrap()).unwrap();
    assert_eq!(matched.aggregates(), &[Some(55.6), Some(55.5)]);
    let (ordinal, RetainedColumn::Aggregate { index, .. }) = query.columns().nth(4).unwrap() else {
        panic!("expected the aggregate after velocity");
    };
    assert_eq!((ordinal, index), (4, 1));
    assert_eq!(
        query
            .aggregate_columns()
            .map(|(ordinal, column)| (ordinal, column.label()))
            .collect::<Vec<_>>(),
        [(2, "max(lat)"), (4, "min(lat)")],
    );
    let rows = matched.rows(None).unwrap();
    let columns: Vec<_> = rows
        .columns()
        .map(|(ordinal, column)| {
            let RowColumn::Metric(metric) = column else {
                panic!("expected point metric")
            };
            (ordinal, metric)
        })
        .collect();
    assert_eq!(
        columns,
        [
            (0, QueryMetric::Time),
            (1, QueryMetric::Eph),
            (3, QueryMetric::Velocity)
        ]
    );
    let values: Vec<_> = (&rows)
        .into_iter()
        .map(|row| (row.index(), row.value(1), row.aggregate(0), row.value(3)))
        .collect();
    assert_eq!(
        values,
        [
            (0, Some(2.5), Some(55.6), Some(10.0)),
            (1, None, Some(55.6), None)
        ]
    );
}
