use gt_history_types::{StoredLogFilter, StoredLogFilterMode};

use crate::loaded_log::tests::fixtures;
use crate::loaded_log::{LoadedLog, LoadedLogs, LogPushOutcome};
use crate::test_util;

/// A hexagon of an unloaded log never identifies the log that took its
/// place in the list: an identity is never handed out twice.
#[test]
fn an_unloaded_logs_identity_is_never_handed_out_again() {
    let mut logs = LoadedLogs::default();
    let unloaded = logs.push(test_util::log_of(3)).id();
    let kept = logs.push(test_util::log_of_service("hal-powerd", 3)).id();
    assert_ne!(unloaded, kept);

    logs.remove_by_id(unloaded);
    let loaded_after = logs.push(test_util::log_of_service("telemetryd", 3)).id();

    assert_eq!(
        logs.iter_with_ids().map(|(id, _)| id).collect::<Vec<_>>(),
        [kept, loaded_after],
        "the log that stayed keeps its identity, and the new one takes its own"
    );
    assert_ne!(loaded_after, unloaded);
}

/// The second log's first layer chip takes the colour after the first
/// log's: a colour means one filter across the session.
#[test]
fn layer_colours_are_handed_out_across_every_loaded_log() {
    let mut logs = LoadedLogs::default();
    let first = logs.push(test_util::log_of(3)).id();
    let second = logs.push(test_util::log_of_service("hal-powerd", 3)).id();

    assert_eq!(
        fixtures::add_layer_chip(&mut logs, first, "entry 0"),
        Some(0)
    );
    assert_eq!(
        fixtures::add_layer_chip(&mut logs, second, "entry 1"),
        Some(1)
    );

    logs.remove_by_id(first);

    assert_eq!(
        fixtures::add_layer_chip(&mut logs, second, "entry 2"),
        Some(0),
        "unloading a log frees the colours its chips held"
    );
}

/// A log hands its colours back when it is unloaded, and takes them anew
/// when it is loaded again with the chips it kept.
#[test]
fn a_log_loaded_again_takes_colours_for_the_chips_it_kept() {
    let mut logs = LoadedLogs::default();
    let first = logs.push(test_util::log_of(3)).id();
    let second = logs.push(test_util::log_of_service("hal-powerd", 3)).id();
    fixtures::add_layer_chip(&mut logs, first, "entry 0");
    fixtures::add_layer_chip(&mut logs, second, "entry 1");

    let unloaded = logs.remove_by_id(first).expect("the log is loaded");
    let loaded_again = logs.push(unloaded).id();

    assert_eq!(
        fixtures::first_chip_slot(&logs, second),
        Some(1),
        "the log that stayed loaded keeps the colour it had"
    );
    assert_eq!(
        fixtures::first_chip_slot(&logs, loaded_again),
        Some(0),
        "the colour the unloaded log freed is the lowest one free again"
    );
}

/// One log per content: a second copy of a text the session already holds
/// is rejected, whatever name it arrived under.
#[test]
fn pushing_content_that_is_already_loaded_returns_the_loaded_log() {
    let mut logs = LoadedLogs::default();
    let loaded = logs.push(test_util::log_of(10));

    let second = logs.push(LoadedLog::new(
        Some("copy-of-navsyncd.log".to_owned()),
        test_util::parsed_log(10),
        test_util::association_window(),
    ));

    assert_eq!(second, LogPushOutcome::AlreadyLoaded(loaded.id()));
    assert_eq!(logs.len(), 1);
    assert_eq!(
        logs.get_by_id(loaded.id()).map(LoadedLog::name),
        Some("navsyncd.log"),
        "the loaded log keeps the name it was loaded under"
    );
}

/// The rejected copy leaves the loaded log as it was, chips and colour
/// slots included.
#[test]
fn a_refused_copy_takes_no_colour_slot_from_the_loaded_log() {
    let mut logs = LoadedLogs::default();
    let id = logs.push(test_util::log_of(10)).id();
    fixtures::add_layer_chip(&mut logs, id, "entry 1");

    let stored = vec![StoredLogFilter {
        text: "entry 2".to_owned(),
        regex: false,
        enabled: true,
        mode: StoredLogFilterMode::Layer { color_slot: 1 },
    }];
    let mut copy = test_util::log_of(10);
    copy.restore_attachment(
        fixtures::attachment_ref(),
        stored,
        &test_util::loaded(Vec::new()).view(),
    );
    logs.push(copy);

    assert_eq!(fixtures::first_chip_slot(&logs, id), Some(0));
    assert_eq!(
        logs.get_by_id(id).and_then(LoadedLog::attachment),
        None,
        "the loaded log took nothing from the copy that was refused"
    );
    assert_eq!(
        fixtures::add_layer_chip(&mut logs, id, "entry 2"),
        Some(1),
        "the refused copy left the palette as the loaded log had it"
    );
}
