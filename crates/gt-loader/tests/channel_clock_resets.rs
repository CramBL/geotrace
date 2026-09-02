//! What the loader reports about a recording from a tracker whose clock
//! restarts at every boot, read back through the real file path.

#![expect(
    clippy::expect_used,
    reason = "a test loading a fixture recording fails loudly when it cannot"
)]

use gt_types::LoadedFile;

fn recording_whose_clock_restarts_at_every_boot() -> LoadedFile {
    gt_loader::load_bytes(
        &gt_test_utils::recording_whose_clock_restarts_at_every_boot(),
        "clock_reset.gtd".to_owned(),
    )
    .expect("the fixture loads")
}

#[test]
fn a_channel_whose_sample_timestamps_step_backwards_is_reported_with_its_worst_step() {
    let file = recording_whose_clock_restarts_at_every_boot();

    let [warning] = file.load_warnings.as_slice() else {
        panic!(
            "expected one load warning, got {:?}",
            file.load_warnings
                .iter()
                .map(|warning| &warning.issue)
                .collect::<Vec<_>>()
        );
    };
    assert_eq!(warning.count, 1);
    assert_eq!(
        warning.issue,
        "sensor channel(s) whose sample timestamps step backwards"
    );
    assert_eq!(
        warning.description,
        "\"accel\": 3 backward steps, worst 14s back. The recorder's clock stepped back while \
         the channel was sampled: the plot draws each stretch between two backward steps as \
         its own line."
    );
}

/// The gold dataset's channels are sampled on a clock that never steps back.
#[test]
fn a_recording_whose_channels_hold_their_order_raises_no_such_warning() {
    let file = gt_loader::load_bytes(gt_test_utils::GOLD_BYTES, "gold.gtd".to_owned())
        .expect("the gold dataset loads");

    assert!(
        !file
            .load_warnings
            .iter()
            .any(|warning| warning.issue.contains("step backwards")),
        "warnings are {:?}",
        file.load_warnings
    );
}
