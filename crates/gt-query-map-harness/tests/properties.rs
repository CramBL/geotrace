//! Properties of the query/map boundary over generated datasets and programs.
//!
//! Every property drives the real [`MapScenario`], so each assertion also runs
//! the harness's own check of the per-point classification against the map's
//! [`gt_map::display_counts::DisplayCounts`]. The panel/map agreement is
//! exercised wherever a property calls `picture()` or `classify()`.

mod support;

use std::collections::HashMap;

use chrono::Duration;
use gt_query_map_harness::{
    Dataset, FileSpec, MapScenario, PointClass, PointSpec, TrackSpec, epoch, track,
};
use gt_types::TrackRef;
use gt_ui_types::{PinnedPopup, PointVisibility};
use proptest::prelude::*;
use support::generate::{
    Agg, CmpOp, GenDataset, Metric, Mode, Newline, Predicate, Program, RenderStyle, Separator,
    Stage, Term, Wrap, gen_dataset, gen_dataset_and_program, gen_point_local_program,
    gen_render_style, gen_windowed_dataset, gen_windowed_dataset_and_program,
};
use support::oracle;

/// The per-point verdicts a scenario reports, per track.
fn observed(scenario: &MapScenario) -> HashMap<TrackRef, Vec<PointVisibility>> {
    scenario
        .dataset()
        .track_refs()
        .into_iter()
        .map(|track_ref| {
            let picture = scenario.picture();
            let points = picture
                .tracks
                .iter()
                .find(|shown| shown.track == track_ref)
                .map(|shown| shown.points.iter().map(|point| point.visibility).collect())
                .unwrap_or_default();
            (track_ref, points)
        })
        .collect()
}

/// The draw stages covering each point, per track, translated from halo layer
/// back to the stage that drew it.
fn observed_draw_stages(
    scenario: &MapScenario,
    program: &Program,
) -> HashMap<TrackRef, Vec<Vec<usize>>> {
    let layers = program.draw_layers();
    scenario
        .picture()
        .tracks
        .iter()
        .map(|shown| {
            let per_point = shown
                .points
                .iter()
                .map(|point| {
                    layers
                        .iter()
                        .enumerate()
                        .filter(|(layer, _)| point.draw_layers.contains(*layer))
                        .map(|(_, stage)| *stage)
                        .collect()
                })
                .collect();
            (shown.track, per_point)
        })
        .collect()
}

/// The window a generated dataset filters by, as instants.
fn window_of(
    dataset: &GenDataset,
) -> Option<(chrono::DateTime<chrono::Utc>, chrono::DateTime<chrono::Utc>)> {
    dataset.window_secs.map(|(start, end)| {
        (
            epoch() + Duration::seconds(start),
            epoch() + Duration::seconds(end),
        )
    })
}

/// Points still visible after running `program`, per track.
fn shown_points(scenario: &MapScenario) -> HashMap<TrackRef, Vec<bool>> {
    observed(scenario)
        .into_iter()
        .map(|(track_ref, points)| {
            let shown = points
                .iter()
                .map(|visibility| visibility.is_shown())
                .collect();
            (track_ref, shown)
        })
        .collect()
}

/// Where a shrunk counterexample is recorded. The default persistence looks for
/// a source root beside the test, which an integration test has none of.
fn config(cases: u32) -> ProptestConfig {
    ProptestConfig {
        cases,
        failure_persistence: Some(Box::new(
            proptest::test_runner::FileFailurePersistence::Direct(
                "tests/proptest-regressions/properties.txt",
            ),
        )),
        ..ProptestConfig::default()
    }
}

proptest! {
    #![proptest_config(config(512))]

    /// The classification is total: every point is shown, hidden by the query,
    /// outside the time filter, or on a track that is not shown - never one of
    /// the states only a category toggle or a bad index can produce.
    #[test]
    fn every_point_lands_in_one_reachable_state(
        (dataset, program) in gen_dataset_and_program(),
    ) {
        let mut scenario = dataset.scenario();
        scenario.run(&program.render_plain());
        for points in observed(&scenario).values() {
            for visibility in points {
                prop_assert!(
                    matches!(
                        visibility,
                        PointVisibility::Shown
                            | PointVisibility::HiddenByQuery
                            | PointVisibility::OutsideTimeFilter
                            | PointVisibility::TrackNotShown
                    ),
                    "unreachable state {visibility:?}"
                );
            }
        }
    }

    /// Halos only ever sit on points the map still draws, and a program of only
    /// `draw` stages hides nothing at all.
    #[test]
    fn halos_stay_on_shown_points(
        (dataset, program) in gen_dataset_and_program(),
    ) {
        let mut scenario = dataset.scenario();
        scenario.run(&program.render_plain());
        let all_draw = program.stages.iter().all(|stage| stage.mode == Mode::Draw);
        for (track_ref, points) in observed(&scenario) {
            for (index, visibility) in points.iter().enumerate() {
                let class = scenario.classify(track_ref, index);
                prop_assert!(
                    class.draw_layers.is_empty() || visibility.is_shown(),
                    "{track_ref:?} point {index} carries a halo while {visibility:?}"
                );
                if all_draw {
                    prop_assert_ne!(
                        *visibility,
                        PointVisibility::HiddenByQuery,
                        "a draw-only program hid {:?} point {}",
                        track_ref,
                        index
                    );
                }
            }
        }
    }

    /// Appending a stage never brings a hidden point back.
    #[test]
    fn appending_a_stage_never_unhides(
        (dataset, program) in gen_dataset_and_program(),
    ) {
        prop_assume!(program.stages.len() >= 2);
        let mut shorter = dataset.scenario();
        shorter.run(&program.prefix(program.stages.len() - 1).render_plain());
        let before = shown_points(&shorter);

        let mut longer = dataset.scenario();
        longer.run(&program.render_plain());
        let after = shown_points(&longer);

        for (track_ref, later) in after {
            let earlier = before.get(&track_ref).cloned().unwrap_or_default();
            for (index, &shown) in later.iter().enumerate() {
                if shown {
                    prop_assert!(
                        earlier.get(index).copied().unwrap_or(false),
                        "{track_ref:?} point {index} came back after another stage"
                    );
                }
            }
        }
    }

    /// Writing a point-local `keep` or `hide` stage twice changes nothing, and
    /// swapping two adjacent point-local `hide` stages changes nothing either:
    /// their masks are intersections of the same per-point verdicts.
    ///
    /// Point-local is the whole condition. A stage reading a window or `accel`
    /// judges a point by its neighbours in the current run, so hiding earlier
    /// changes what it matches - see
    /// `chaining::swapping_two_hides_can_change_the_map`.
    #[test]
    fn repeating_and_swapping_point_local_stages_are_no_ops(
        dataset in gen_dataset(),
        program in gen_point_local_program(),
        index in 0usize..4,
    ) {
        prop_assert!(program.stages.iter().all(Stage::is_point_local));
        let mut plain = dataset.scenario();
        plain.run(&program.render_plain());
        let expected = shown_points(&plain);

        if program
            .stages
            .get(index)
            .is_some_and(|stage| stage.mode != Mode::Draw)
        {
            let mut repeated = dataset.scenario();
            repeated.run(&program.with_stage_repeated(index).render_plain());
            prop_assert_eq!(
                shown_points(&repeated),
                expected.clone(),
                "repeating stage {} changed the map",
                index
            );
        }

        let both_hide = [index, index + 1].iter().all(|&at| {
            program
                .stages
                .get(at)
                .is_some_and(|stage| stage.mode == Mode::Hide)
        });
        if both_hide {
            let mut swapped = dataset.scenario();
            swapped.run(&program.with_adjacent_swapped(index).render_plain());
            prop_assert_eq!(
                shown_points(&swapped),
                expected,
                "swapping hides at {} changed the map",
                index
            );
        }
    }

    /// A point outside the time filter is never reported as hidden by the query:
    /// the run never saw it, so nothing it did can explain its absence.
    #[test]
    fn a_filtered_out_point_is_never_hidden_by_the_query(
        (dataset, program) in gen_windowed_dataset_and_program(),
    ) {
        let mut scenario = dataset.scenario();
        scenario.run(&program.render_plain());
        let files = scenario.dataset().files().files().to_vec();
        let window = window_of(&dataset);
        for (track_ref, points) in observed(&scenario) {
            let Some(track) = track_ref.resolve(&files) else {
                continue;
            };
            for (index, visibility) in points.iter().enumerate() {
                let inside = track.points.get(index).is_some_and(|point| {
                    window.is_none_or(|(start, end)| {
                        let time = point.tpv.time().utc();
                        time >= start && time <= end
                    })
                });
                if !inside {
                    prop_assert_ne!(
                        *visibility,
                        PointVisibility::HiddenByQuery,
                        "{:?} point {} is outside the window yet hidden",
                        track_ref,
                        index
                    );
                }
            }
        }
    }

    /// Clearing is a true reset: a run, a clear, and another run read exactly
    /// like the second run on its own, and a cleared scenario reads like one
    /// that never ran.
    #[test]
    fn clearing_returns_to_the_never_run_baseline(
        (dataset, first) in gen_dataset_and_program(),
        (_, second) in gen_dataset_and_program(),
    ) {
        let baseline = dataset.scenario();
        let untouched = baseline.picture().to_string();

        let mut reused = dataset.scenario();
        reused.run(&first.render_plain());
        reused.clear();
        prop_assert_eq!(
            reused.picture().to_string(),
            untouched,
            "clearing left something behind"
        );

        reused.run(&second.render_plain());
        let mut fresh = dataset.scenario();
        fresh.run(&second.render_plain());
        prop_assert_eq!(
            reused.picture().to_string(),
            fresh.picture().to_string(),
            "the first run leaked into the second"
        );
    }

    /// Another file's tracks never change a track's own classification.
    #[test]
    fn a_track_is_classified_independently_of_the_others(
        (dataset, program) in gen_dataset_and_program(),
    ) {
        let alone = GenDataset {
            files: dataset.files.iter().take(1).cloned().collect(),
            window_secs: dataset.window_secs,
        };
        prop_assume!(!alone.files.is_empty());

        let mut with_others = dataset.scenario();
        with_others.run(&program.render_plain());
        let together = observed(&with_others);

        let mut on_its_own = alone.scenario();
        on_its_own.run(&program.render_plain());

        for (track_ref, points) in observed(&on_its_own) {
            prop_assert_eq!(
                together.get(&track_ref).cloned().unwrap_or_default(),
                points,
                "{:?} reads differently beside another file",
                track_ref
            );
        }
    }

    /// The same dataset and text run twice give the same picture.
    #[test]
    fn a_run_is_deterministic(
        (dataset, program) in gen_dataset_and_program(),
    ) {
        let text = program.render_plain();
        let mut once = dataset.scenario();
        once.run(&text);
        let mut twice = dataset.scenario();
        twice.run(&text);
        prop_assert_eq!(once.picture().to_string(), twice.picture().to_string());
    }
}

proptest! {
    #![proptest_config(config(512))]

    /// A pinned popup only ever draws for a point the map draws. Pin every point
    /// in turn, run the program, and no withheld point may be showing a popup -
    /// the invariant behind the popup that outlived its own point.
    #[test]
    fn a_pinned_popup_never_outlives_its_point(
        (dataset, program) in gen_dataset_and_program(),
    ) {
        let text = program.render_plain();
        let baseline = dataset.scenario().picture();
        for shown in &baseline.tracks {
            let track_ref = shown.track;
            for index in 0..shown.points.len() {
                // Pin before the run, so the pin predates the query that hides
                // its point - the order the bug needed.
                let mut scenario = dataset.scenario();
                scenario.select_point(track_ref, index);
                scenario.run(&text);
                let picture = scenario.picture();
                let drawn = picture
                    .tracks
                    .iter()
                    .find(|shown| shown.track == track_ref)
                    .and_then(|shown| shown.points.get(index))
                    .is_some_and(PointClass::is_shown);
                prop_assert_eq!(
                    matches!(picture.pin, Some(PinnedPopup::Drawn(_))),
                    drawn,
                    "{:?} point {} draws {} but its popup disagrees ({:?}), program {}",
                    track_ref,
                    index,
                    drawn,
                    picture.pin,
                    text
                );
            }
        }
    }

    /// The harness agrees with a naive reference fold of the documented
    /// semantics, point by point and halo by halo.
    #[test]
    fn the_map_agrees_with_the_reference_fold(
        (dataset, program) in gen_dataset_and_program(),
    ) {
        let mut scenario = dataset.scenario();
        scenario.run(&program.render_plain());
        let files = scenario.dataset().files().files().to_vec();
        let expected = oracle::expect(&files, window_of(&dataset), &program);

        let seen = observed(&scenario);
        let drawn = observed_draw_stages(&scenario, &program);
        for (track_ref, expectation) in expected {
            prop_assert_eq!(
                seen.get(&track_ref).cloned().unwrap_or_default(),
                expectation.visibility,
                "{:?} visibility disagrees for {}",
                track_ref,
                program.render_plain()
            );
            prop_assert_eq!(
                drawn.get(&track_ref).cloned().unwrap_or_default(),
                expectation.draw_stages,
                "{:?} halos disagree for {}",
                track_ref,
                program.render_plain()
            );
        }
    }

    /// Every prefix of a program agrees with the reference fold of that prefix,
    /// and each stage only ever shrinks what is shown: the pipeline composes
    /// stage by stage, with no stage reaching backwards.
    #[test]
    fn every_prefix_agrees_with_the_reference_fold(
        (dataset, program) in gen_dataset_and_program(),
    ) {
        let files = dataset.dataset().files().files().to_vec();
        let window = window_of(&dataset);
        let mut previous: Option<HashMap<TrackRef, Vec<bool>>> = None;
        for count in 1..=program.stages.len() {
            let prefix = program.prefix(count);
            let mut scenario = dataset.scenario();
            scenario.run(&prefix.render_plain());
            let seen = observed(&scenario);
            for (track_ref, expectation) in oracle::expect(&files, window, &prefix) {
                prop_assert_eq!(
                    seen.get(&track_ref).cloned().unwrap_or_default(),
                    expectation.visibility,
                    "{:?} disagrees after {} stages of {}",
                    track_ref,
                    count,
                    program.render_plain()
                );
            }
            let shown = shown_points(&scenario);
            if let Some(before) = previous {
                for (track_ref, later) in &shown {
                    let earlier = before.get(track_ref).cloned().unwrap_or_default();
                    for (index, &visible) in later.iter().enumerate() {
                        prop_assert!(
                            !visible || earlier.get(index).copied().unwrap_or(false),
                            "{:?} point {} came back at stage {}",
                            track_ref,
                            index,
                            count
                        );
                    }
                }
            }
            previous = Some(shown);
        }
    }

    /// Narrowing the time filter never changes the classification of a point
    /// that stays inside it, for a program that judges each point on its own.
    ///
    /// Point-local is the whole condition again. A window or an `accel` reads
    /// the neighbours the run still has, so trimming the slice can withdraw a
    /// window that reached over the cut - see
    /// `index_mapping::narrowing_the_window_can_change_a_windowed_match`.
    #[test]
    fn narrowing_the_window_leaves_the_survivors_alone(
        dataset in gen_windowed_dataset(),
        program in gen_point_local_program(),
        trim in 1i64..=8,
    ) {
        let (start, end) = dataset.window_secs.unwrap_or((0, 45));
        prop_assume!(end - trim > start);
        let wide = GenDataset { files: dataset.files.clone(), window_secs: Some((start, end)) };
        let narrow = GenDataset {
            files: dataset.files.clone(),
            window_secs: Some((start, end - trim)),
        };

        let mut before = wide.scenario();
        before.run(&program.render_plain());
        let mut after = narrow.scenario();
        after.run(&program.render_plain());

        let files = after.dataset().files().files().to_vec();
        let wide_view = observed(&before);
        let cut = epoch() + Duration::seconds(end - trim);
        let from = epoch() + Duration::seconds(start);
        for (track_ref, points) in observed(&after) {
            let Some(track) = track_ref.resolve(&files) else {
                continue;
            };
            let wide_points = wide_view.get(&track_ref).cloned().unwrap_or_default();
            for (index, visibility) in points.iter().enumerate() {
                let inside = track.points.get(index).is_some_and(|point| {
                    let time = point.tpv.time().utc();
                    time >= from && time <= cut
                });
                if inside {
                    prop_assert_eq!(
                        wide_points.get(index).copied(),
                        Some(*visibility),
                        "{:?} point {} reads differently once the window narrowed, program {}",
                        track_ref,
                        index,
                        program.render_plain()
                    );
                }
            }
        }
    }
}

proptest! {
    #![proptest_config(config(1024))]

    /// The same program written with different separators, line endings,
    /// indentation, and leading or trailing blank lines produces exactly the
    /// same map. This is what `split_queries` promises.
    #[test]
    fn the_writing_style_never_changes_the_map(
        (dataset, program) in gen_dataset_and_program(),
        style in gen_render_style(),
    ) {
        let mut plain = dataset.scenario();
        plain.run(&program.render_plain());
        let expected = plain.picture().to_string();

        let text = program.render(&style);
        let mut styled = dataset.scenario();
        styled.run(&text);
        prop_assert_eq!(
            styled.picture().to_string(),
            expected,
            "style {:?} changed the map for {:?}",
            style,
            text
        );
    }

    /// An always-true `keep` hides nothing. An always-false one hides every
    /// point the run saw. Written on `lat`, the one metric every point carries -
    /// on any other, a point missing it would be skipped and so hidden, which
    /// the property below covers.
    #[test]
    fn keep_hides_by_its_predicate_alone(dataset in gen_dataset()) {
        let mut everything = dataset.scenario();
        everything.run("points | where lat <= 90 deg | keep");
        for (track_ref, points) in observed(&everything) {
            for (index, visibility) in points.iter().enumerate() {
                prop_assert_ne!(
                    *visibility,
                    PointVisibility::HiddenByQuery,
                    "{:?} point {} was hidden by a keep that matches everything",
                    track_ref,
                    index
                );
            }
        }

        let mut nothing = dataset.scenario();
        nothing.run("points | where lat > 90 deg | keep");
        for (track_ref, points) in observed(&nothing) {
            for (index, visibility) in points.iter().enumerate() {
                prop_assert_ne!(
                    *visibility,
                    PointVisibility::Shown,
                    "{:?} point {} survived a keep that matches nothing",
                    track_ref,
                    index
                );
            }
        }
    }

    /// A `keep` on a metric hides every point without that metric: a missing
    /// value makes the predicate skip, and a skipped point was never matched.
    #[test]
    fn a_keep_on_a_missing_metric_hides_that_point(dataset in gen_dataset()) {
        let mut scenario = dataset.scenario();
        scenario.run("points | where velocity >= 0 km/h | keep");
        let files = scenario.dataset().files().files().to_vec();
        for (track_ref, points) in observed(&scenario) {
            let Some(track) = track_ref.resolve(&files) else {
                continue;
            };
            for (index, visibility) in points.iter().enumerate() {
                let carries_velocity = track
                    .points
                    .get(index)
                    .is_some_and(|point| point.tpv.velocity().is_some());
                if !carries_velocity {
                    prop_assert_ne!(
                        *visibility,
                        PointVisibility::Shown,
                        "{:?} point {} has no velocity yet survived a keep on it",
                        track_ref,
                        index
                    );
                }
            }
        }
    }
}

/// The renderer's messiness, spelled out: the same two-stage program written
/// plainly, wrapped at its pipes with CRLF endings, and with whitespace-only
/// separator lines. All three run to the same map, which
/// `the_writing_style_never_changes_the_map` asserts over generated programs.
#[test]
fn a_program_renders_with_the_messiness_it_is_given() {
    let program = Program {
        stages: vec![
            Stage {
                mode: Mode::Hide,
                window: None,
                predicate: Predicate::Cmp {
                    term: Term::Point(Metric::Velocity),
                    op: CmpOp::Lt,
                    threshold: 5.0,
                },
            },
            Stage {
                mode: Mode::Draw,
                window: Some(3),
                predicate: Predicate::And(
                    Box::new(Predicate::Cmp {
                        term: Term::Agg {
                            func: Agg::Avg,
                            metric: Metric::Velocity,
                        },
                        op: CmpOp::Ge,
                        threshold: 30.0,
                    }),
                    Box::new(Predicate::Cmp {
                        term: Term::Agg {
                            func: Agg::Max,
                            metric: Metric::Eph,
                        },
                        op: CmpOp::Lt,
                        threshold: 20.0,
                    }),
                ),
            },
        ],
    };
    insta::assert_snapshot!(program.render_plain(), @r"
    points | where velocity < 5 km/h | hide

    points | window 3 | where (avg(velocity) >= 30 km/h) and (max(eph) < 20 m) | draw
    ");

    let messy = RenderStyle {
        newline: Newline::Crlf,
        separators: vec![Separator::SpacesAndBlanks],
        leading_blanks: 1,
        trailing_blanks: 2,
        wrap: Wrap::Wrapped { indent: 4 },
    };
    let text = program.render(&messy);
    assert!(text.contains("\r\n"), "the messy style writes CRLF endings");
    assert!(
        text.contains("\r\n    | window 3"),
        "and wraps at the pipes with an indent: {:?}",
        text
    );
    assert!(
        text.contains("\r\n \t\r\n"),
        "and separates with a line holding only whitespace: {:?}",
        text
    );

    // However it is written, the editor still sees exactly the two queries.
    let mut scenario = MapScenario::new(Dataset::single_track(TrackSpec::from_points(
        [1.0, 40.0, 40.0, 40.0]
            .iter()
            .enumerate()
            .map(|(index, &speed)| PointSpec::at_secs(index as i64).speed_kmh(speed).eph_m(5.0))
            .collect(),
    )));
    scenario.run(&text);
    insta::assert_snapshot!(scenario.panel(), @"
    chunks: 2
      2..51 ok
      61..157 ok
    run: completed
      1 match on 1 track — 1 of 4 points hidden
      1 match on 1 track
    ");
}

/// A one-point track and a track shorter than any window it meets, which the
/// generators reach only by chance, pinned as fixed cases.
#[test]
fn short_tracks_survive_a_windowed_program() {
    let mut scenario = MapScenario::new(Dataset::of_files(&[
        FileSpec::with_tracks("one.gtd", vec![TrackSpec::from_speeds_kmh(&[40.0])]),
        FileSpec::with_tracks(
            "two.gtd",
            vec![TrackSpec::from_points(vec![
                PointSpec::at_secs(0).speed_kmh(40.0),
                PointSpec::at_secs(1),
            ])],
        ),
    ]));
    scenario.run("points | window 5 | where avg(velocity) > 1 km/h | keep");
    insta::assert_snapshot!(scenario.picture(), @r"
    one.gtd#0  x
    two.gtd#0  xx
    counts: shown 0, halos 0
    ");
    assert_eq!(
        scenario.classify(track(0, 0), 0).visibility,
        PointVisibility::HiddenByQuery,
        "a keep whose window never fits keeps nothing"
    );
}
