use gt_loaded_files::{LoadedFiles, RecordingNames};
use gt_types::{FileIdx, FixRef, PointIdx, TrackIdx, TrackRef};

use crate::loaded_log::tests::fixtures;
use crate::loaded_log::{LoadedLog, LoadedLogId, LoadedLogs};
use crate::test_util;
use gt_ui_types::LogMatchColor;

/// What the map draws for a log: the entries a chip matched, at the fixes
/// they were associated to.
#[test]
fn a_layer_chip_puts_the_lines_it_matched_on_the_map() {
    let files = test_util::loaded(vec![test_util::recording_at(55.0, 10)]);
    let mut logs = LoadedLogs::default();
    let mut log = test_util::log_of(10);
    test_util::anchor_to(&mut log, &files, 0);
    let id = logs.push(log).id();

    fixtures::add_layer_chip(&mut logs, id, "entry 1");
    fixtures::wait_for_scans(&mut logs);

    let matches = test_util::map_matches(&mut logs, &files);
    assert_eq!(
        matches.layers().len(),
        1,
        "the live filter is empty, so it draws nothing"
    );
    assert_eq!(
        matches.layers().first().map(|layer| layer.color),
        Some(LogMatchColor::LayerSlot {
            index: 0,
            shared: false,
        })
    );
    assert_eq!(matches.match_count(), 1, "\"entry 1\" matches one line");
}

#[test]
fn a_map_match_names_the_fix_its_entry_was_placed_on() {
    /// Fixes of the recording's first track, the rest being its second.
    const FIRST_TRACK_FIXES: usize = 4;

    /// Entries of the log, one per fix of the recording.
    const ENTRIES: usize = 10;

    let files = test_util::loaded(vec![test_util::recording_in_two_tracks(
        FIRST_TRACK_FIXES,
        ENTRIES - FIRST_TRACK_FIXES,
    )]);
    let mut logs = LoadedLogs::default();
    let mut log = test_util::log_of(ENTRIES);
    test_util::anchor_to(&mut log, &files, 0);
    let id = logs.push(log).id();
    fixtures::add_layer_chip(&mut logs, id, "entry");
    fixtures::wait_for_scans(&mut logs);

    let fixes: Vec<FixRef> = test_util::map_matches(&mut logs, &files)
        .layers()
        .first()
        .map(|layer| {
            layer
                .matches
                .iter()
                .map(|log_match| log_match.fix)
                .collect()
        })
        .unwrap_or_default();

    assert_eq!(
        fixes,
        (0..ENTRIES)
            .map(|entry| {
                let (ti, pi) = if entry < FIRST_TRACK_FIXES {
                    (0, entry)
                } else {
                    (1, entry - FIRST_TRACK_FIXES)
                };
                FixRef::new(
                    TrackRef::new(FileIdx::new(0), TrackIdx::new(ti)),
                    PointIdx::new(pi),
                )
            })
            .collect::<Vec<FixRef>>()
    );
}

/// A log anchored to no recording has nothing to put on the map, however
/// much its filters match.
#[test]
fn an_unassociated_log_draws_nothing() {
    let recordings = test_util::loaded(vec![test_util::recording_at(55.0, 10)]);
    let mut logs = LoadedLogs::default();
    let id = logs.push(test_util::log_of(10)).id();

    fixtures::add_layer_chip(&mut logs, id, "entry");
    fixtures::wait_for_scans(&mut logs);

    assert!(test_util::map_matches(&mut logs, &recordings).is_empty());
}

/// The whole map contribution of a log switches off with the log, and comes
/// back with it.
#[test]
fn hiding_a_log_takes_its_layers_off_the_map() {
    let files = test_util::loaded(vec![test_util::recording_at(55.0, 10)]);
    let mut logs = LoadedLogs::default();
    let mut log = test_util::log_of(10);
    test_util::anchor_to(&mut log, &files, 0);
    let id = logs.push(log).id();
    fixtures::add_layer_chip(&mut logs, id, "entry");
    fixtures::wait_for_scans(&mut logs);
    assert_eq!(test_util::map_matches(&mut logs, &files).match_count(), 10);

    if let Some(log) = logs.get_mut_by_id(id) {
        log.set_visible(false);
    }
    assert!(test_util::map_matches(&mut logs, &files).is_empty());

    if let Some(log) = logs.get_mut_by_id(id) {
        log.set_visible(true);
    }
    assert_eq!(test_util::map_matches(&mut logs, &files).match_count(), 10);
}

/// A refine chip narrows the table, never the map: it has no colour to draw
/// in. The live filter draws over the chips that were added.
#[test]
fn the_map_holds_the_layer_chips_and_the_live_filter_over_them() {
    let files = test_util::loaded(vec![test_util::recording_at(55.0, 10)]);
    let mut logs = LoadedLogs::default();
    let mut log = test_util::log_of(10);
    test_util::anchor_to(&mut log, &files, 0);
    let id = logs.push(log).id();
    fixtures::add_layer_chip(&mut logs, id, "entry 2");
    if let Some((stack, slots)) = logs.filter_stack_mut_by_id(id) {
        stack.set_live_filter_text("entry 3");
        let refined = stack.add_live_filter_as_chip(slots);
        if let Some(chip) = refined {
            stack.switch_chip_to_refine_mode(chip, slots);
        }
        stack.set_live_filter_text("entry");
    }
    fixtures::wait_for_scans(&mut logs);

    let colors: Vec<LogMatchColor> = test_util::map_matches(&mut logs, &files)
        .layers()
        .iter()
        .map(|layer| layer.color)
        .collect();
    assert_eq!(
        colors,
        [
            LogMatchColor::LayerSlot {
                index: 0,
                shared: false,
            },
            LogMatchColor::LiveFilter,
        ]
    );
}

/// Unloading a log takes what it drew with it: the cached layers are what
/// the logs still loaded selected.
#[test]
fn unloading_a_log_takes_its_layers_off_the_map() {
    let files = test_util::loaded(vec![test_util::recording_at(55.0, 10)]);
    let mut logs = LoadedLogs::default();
    let mut kept = test_util::log_of(10);
    test_util::anchor_to(&mut kept, &files, 0);
    let kept = logs.push(kept).id();
    let mut unloaded = test_util::log_of_service("hal-powerd", 10);
    test_util::anchor_to(&mut unloaded, &files, 0);
    let unloaded = logs.push(unloaded).id();
    fixtures::add_layer_chip(&mut logs, kept, "entry 1");
    fixtures::add_layer_chip(&mut logs, unloaded, "entry");
    fixtures::wait_for_scans(&mut logs);
    assert_eq!(test_util::map_matches(&mut logs, &files).match_count(), 11);

    logs.remove_by_id(unloaded);

    assert_eq!(
        test_util::map_matches(&mut logs, &files).match_count(),
        1,
        "only the log still loaded draws"
    );
}

/// Unloading the anchored recording strands the log's layers: nothing draws
/// where no fix says it was.
#[test]
fn re_association_after_a_recording_is_unloaded_empties_the_map() {
    let mut files = test_util::loaded(vec![test_util::recording_at(55.0, 10)]);
    let mut logs = LoadedLogs::default();
    let mut log = test_util::log_of(10);
    test_util::anchor_to(&mut log, &files, 0);
    let id = logs.push(log).id();
    fixtures::add_layer_chip(&mut logs, id, "entry");
    fixtures::wait_for_scans(&mut logs);
    assert_eq!(test_util::map_matches(&mut logs, &files).match_count(), 10);

    files.remove_file(0);
    logs.reassociate_all(&files.view());

    assert!(test_util::map_matches(&mut logs, &files).is_empty());
}

/// Each layer identifies the log its filter read, which is how the map
/// hands a hovered hexagon back to the viewer showing that log.
#[test]
fn every_layer_names_the_log_it_was_filtered_out_of() {
    let files = test_util::loaded(vec![test_util::recording_at(55.0, 10)]);
    let mut logs = LoadedLogs::default();
    let mut ids = Vec::new();
    for service in ["navsyncd", "hal-powerd"] {
        let mut log = test_util::log_of_service(service, 10);
        test_util::anchor_to(&mut log, &files, 0);
        ids.push(logs.push(log).id());
    }
    if let [first, second] = ids.as_slice() {
        fixtures::add_layer_chip(&mut logs, *first, "entry 1");
        fixtures::add_layer_chip(&mut logs, *second, "entry 2");
    }
    fixtures::wait_for_scans(&mut logs);

    let layer_logs: Vec<LoadedLogId> = test_util::map_matches(&mut logs, &files)
        .layers()
        .iter()
        .map(|layer| layer.log.id)
        .collect();

    assert_eq!(layer_logs, ids, "one layer per log, each naming its own");
}

/// What a hexagon's tooltip identifies its log by: nothing while the
/// session holds one log, the log's name once it holds a second, and the
/// anchored recording after the name where two logs go by the same name.
#[rstest::rstest]
#[case::one_loaded_log(&[("navsyncd.log", "navsyncd")], &[None])]
#[case::two_names(
    &[("navsyncd.log", "navsyncd"), ("hal-powerd.log", "hal-powerd")],
    &[Some("navsyncd.log"), Some("hal-powerd.log")]
)]
#[case::one_name_twice(
    &[("navsyncd.log", "navsyncd"), ("navsyncd.log", "hal-powerd")],
    &[Some("navsyncd.log · walk.gtd"), Some("navsyncd.log · drive.gtd")]
)]
fn a_hexagon_tooltip_identifies_its_log_once_a_second_log_is_loaded(
    #[case] loaded_logs: &[(&str, &str)],
    #[case] expected: &[Option<&str>],
) {
    let files = test_util::loaded(vec![
        test_util::recording_named("walk.gtd", 55.0, 10),
        test_util::recording_named("drive.gtd", 60.0, 10),
    ]);
    let mut logs = LoadedLogs::default();
    for (index, (name, service)) in loaded_logs.iter().enumerate() {
        let mut log = LoadedLog::new(
            Some((*name).to_owned()),
            test_util::parsed_log_of_service(service, 10),
            test_util::association_window(),
        );
        test_util::anchor_to(&mut log, &files, index);
        let id = logs.push(log).id();
        fixtures::add_layer_chip(&mut logs, id, "entry 1");
    }
    fixtures::wait_for_scans(&mut logs);

    let display_names: Vec<Option<String>> = test_util::map_matches(&mut logs, &files)
        .layers()
        .iter()
        .map(|layer| layer.log.display_name.clone())
        .collect();

    assert_eq!(
        display_names,
        expected
            .iter()
            .map(|name| name.map(ToOwned::to_owned))
            .collect::<Vec<Option<String>>>()
    );
}

/// The recording in a tooltip is the name the app resolves now: a template
/// change reaches the layers the cache holds.
#[test]
fn a_name_template_change_resolves_the_tooltip_recordings_again() {
    let files = test_util::loaded(vec![
        test_util::recording_named("walk.gtd", 55.0, 10),
        test_util::recording_named("drive.gtd", 60.0, 10),
    ]);
    let mut logs = LoadedLogs::default();
    for index in 0..2 {
        let service = ["navsyncd", "hal-powerd"].get(index).copied().unwrap_or("");
        let mut log = LoadedLog::new(
            Some("navsyncd.log".to_owned()),
            test_util::parsed_log_of_service(service, 10),
            test_util::association_window(),
        );
        test_util::anchor_to(&mut log, &files, index);
        let id = logs.push(log).id();
        fixtures::add_layer_chip(&mut logs, id, "entry 1");
    }
    fixtures::wait_for_scans(&mut logs);
    assert_eq!(
        first_layer_display_name(&mut logs, &files, "{filename}"),
        Some("navsyncd.log · walk.gtd".to_owned())
    );

    assert_eq!(
        first_layer_display_name(&mut logs, &files, "recording {filename}"),
        Some("navsyncd.log · recording walk.gtd".to_owned())
    );
}

/// What the first layer's tooltip identifies its log by, with the
/// recordings named under `template`.
fn first_layer_display_name(
    logs: &mut LoadedLogs,
    recordings: &LoadedFiles,
    template: &str,
) -> Option<String> {
    let names = RecordingNames::resolve(recordings.view(), template);
    logs.map_matches(recordings.view(), &names)
        .layers()
        .first()?
        .log
        .display_name
        .clone()
}
