use egui_kittest::{Harness, kittest::NodeT as _, kittest::Queryable as _};
use gt_store::{FlareStore, IonexStore, SolarStore};
use gt_test_utils::{DEMO_BYTES, GOLD_BYTES, SyntheticGtdSpec, TestHarness};
use gt_types::{FileIdx, TrackIdx, TrackRef};
use strum::IntoEnumIterator as _;

use crate::app::App;
use crate::app::test_util;
use crate::app::test_util::harness::TestDroppedFile;
use crate::app::ui_tests;

/// Snapshot of the app with the gold dataset loaded. Captures the side panel,
/// the map area, and the plot with the metric filter row (including the Sync
/// button, grid toggle, and metric chips).
#[test]
fn snapshot_app_with_file_loaded() {
    let (mut harness, _config_path) = TestHarness::builder()
        .size(egui::vec2(1280.0, 800.0))
        .eframe(test_util::harness::build_app);
    harness.inner.step();

    test_util::harness::drop_file_and_wait_for_load(
        &mut harness.inner,
        TestDroppedFile::bytes(GOLD_BYTES, "gold.gtd"),
    );
    // The app repaints continuously (map + background jobs). Run many frames
    // so the map zoom and plot layout converge before we snapshot.
    harness.inner.run_steps(60);

    harness.snapshot_with_color_tolerance("app_with_file_loaded");
}

/// The load warning as the user meets it: the toast the application raises
/// for a recording the archives place a disturbance in, stating the recording
/// and what each metric reached over it.
#[test]
fn snapshot_space_weather_warning_toast() {
    let (mut harness, _config_path) = TestHarness::builder()
        .size(egui::vec2(1280.0, 800.0))
        .eframe(test_util::harness::build_app);
    harness.inner.step();

    test_util::harness::drop_file_and_wait_for_load(
        &mut harness.inner,
        TestDroppedFile::bytes(GOLD_BYTES, "gold.gtd"),
    );
    harness.inner.run_steps(60);

    let recorded = harness
        .state()
        .shared
        .borrow()
        .loaded_files
        .files()
        .first()
        .and_then(|file| file.metadata.time_range)
        .map(|range| range.start.date_naive())
        .expect("the gold recording is loaded");
    let dir = tempfile::tempdir().expect("temp dir");
    let store = gt_store::Store::open_in(dir.path())
        .open_or_create_archive::<SolarStore>()
        .expect("archive");
    // Every period of the archived day is at storm level, so the recording is
    // disturbed wherever in the day its first track falls.
    test_util::day_archive::archive_kp_day(&store, recorded, 5.0);
    let ctx = harness.inner.ctx.clone();
    harness.state_mut().geomagnetic_indices = crate::app::solar::GeomagneticIndexScheduler::new(
        ctx,
        Some(store),
        gt_solar::DEFAULT_BASE_URL.to_owned(),
        gt_fetch::TransportSource::Offline,
        gt_pending_writes::PendingWrites::default(),
    );
    // Two frames put the toast the archived day raises past its slide-in
    // without reaching its expiry.
    harness.inner.step();
    harness.inner.step();

    harness.snapshot_with_color_tolerance("space_weather_warning_toast");
}

/// Every level the map's environment indicator lists, and the reference
/// window a row's link opens on that metric's material.
#[test]
fn the_map_warning_levels_open_their_reference_windows() {
    let (mut harness, _config_path) = TestHarness::builder()
        .size(egui::vec2(1280.0, 800.0))
        .eframe(test_util::harness::build_app);
    harness.inner.run_steps(2);

    harness
        .inner
        .get_by_label(egui_phosphor::regular::CLOUD_LIGHTNING)
        .click();
    harness.inner.run_steps(2);

    for level in &*crate::app::space_weather_warning::WARNING_LEVELS {
        assert!(
            harness
                .inner
                .query_by_label_contains(&level.trigger)
                .is_some(),
            "the popup never states {:?}",
            level.trigger
        );
    }

    let interference = gt_jam::reference::AIRCRAFT_INTERFERENCE;
    harness
        .inner
        .get_by_label_contains(interference.link_question)
        .click();
    harness.inner.run_steps(2);

    assert!(harness.inner.state().reference_window.is_open());
    assert!(
        harness
            .inner
            .query_all_by_label_contains(interference.title)
            .next()
            .is_some(),
        "the reference window shows its title"
    );
}

/// The same loaded-file view under the light theme, so the side panel, chip row,
/// and plot are all exercised on a light background - the general light-mode
/// baseline alongside the plot- and badge-specific ones.
#[test]
fn snapshot_app_with_file_loaded_light() {
    let (mut harness, _config_path) = TestHarness::builder()
        .size(egui::vec2(1280.0, 800.0))
        .eframe(test_util::harness::build_app);
    harness.inner.step();

    test_util::harness::drop_file_and_wait_for_load(
        &mut harness.inner,
        TestDroppedFile::bytes(GOLD_BYTES, "gold.gtd"),
    );
    harness.inner.ctx.set_theme(egui::ThemePreference::Light);
    harness.inner.run_steps(60);

    harness.snapshot_with_color_tolerance("app_with_file_loaded_light");
}

/// Snapshot of the point window pinned on a fix the receiver wrote a latitude
/// of 91° for: the window marks the recorded value and states the position the
/// map draws the fix at.
#[test]
fn snapshot_app_point_window_coordinate_out_of_range() {
    let (mut harness, _config_path) = TestHarness::builder()
        .size(egui::vec2(1280.0, 800.0))
        .eframe(test_util::harness::build_app);
    harness.inner.step();

    let out_of_range = gt_types::PointIdx::new(2);
    let points = gt_test_utils::fixtures::nav_points_with_a_latitude_out_of_range(6, out_of_range);
    let fi = ui_tests::push_points_as(
        &mut harness.inner,
        "out_of_range.gtd",
        &points,
        None,
        gt_loaded_files::FileHistory::None,
    );

    {
        let mut shared = harness.inner.state().shared.borrow_mut();
        shared.highlight.toggle_sticky(gt_ui_types::DataPointRef {
            track: TrackRef::new(fi, TrackIdx::new(0)),
            category: gt_types::DataCategory::Tpv,
            point_index: out_of_range,
        });
        shared.zoom_to_visible_request = true;
    }
    harness.inner.run_steps(30);

    harness.snapshot_with_color_tolerance("app_point_window_coordinate_out_of_range");
}

/// Snapshot of the app zoomed into the cluster of Sahara desert tracks from
/// the gold dataset. All other tracks (antimeridian, southern hemisphere, etc.)
/// are hidden so only the closely-spaced Sahara tracks fill the map.
#[test]
fn snapshot_app_sahara_tracks() {
    let (mut harness, _config_path) = TestHarness::builder()
        .size(egui::vec2(1280.0, 800.0))
        .eframe(test_util::harness::build_app_on_captured_tiles);
    harness.inner.step();

    test_util::harness::drop_file_and_wait_for_load(
        &mut harness.inner,
        TestDroppedFile::bytes(GOLD_BYTES, "gold.gtd"),
    );

    // Identify the Sahara tracks by latitude: they are all centred around
    // 23°N 13°E.
    let sahara_tracks: Vec<TrackRef> = {
        let state = harness.inner.state().shared.borrow();
        state.loaded_files[0]
            .tracks
            .iter()
            .enumerate()
            .filter(|(_, t)| {
                t.geometry
                    .measured()
                    .is_some_and(|geometry| geometry.bounding_box.lat.south().as_degrees() > 20.0)
            })
            .map(|(i, _)| TrackRef {
                fi: FileIdx::new(0),
                index: TrackIdx::new(i),
            })
            .collect()
    };

    {
        let mut state = harness.inner.state().shared.borrow_mut();
        state.tree.show_only_tracks(&sahara_tracks);
        state.zoom_to_visible_request = true;
    }

    harness.inner.run_steps(60);

    ui_tests::assert_the_capture_covers_the_map(&mut harness, "app_sahara_tracks");
    harness.snapshot_with_color_tolerance("app_sahara_tracks");
}

/// Snapshot of the demo trip along the Paris quays: a single track with a
/// 59 s tunnel fix-loss rendered as a dashed ghost stretch, custom and
/// event markers, and multi-constellation satellite data driving the
/// fix-quality colors. This is the screenshot embedded in README.md.
#[test]
fn snapshot_app_demo_trip() {
    let (mut harness, _config_path) = TestHarness::builder()
        .size(egui::vec2(1280.0, 800.0))
        .eframe(test_util::harness::build_app_on_captured_tiles);
    harness.inner.step();

    test_util::harness::drop_file_and_wait_for_load(
        &mut harness.inner,
        TestDroppedFile::bytes(DEMO_BYTES, "demo_trip.gtd"),
    );

    {
        let mut state = harness.inner.state().shared.borrow_mut();
        state.zoom_to_visible_request = true;
    }

    harness.inner.hover_at(egui::Pos2 { x: 480., y: 200. });

    // The app repaints continuously (map + background jobs). Run many frames
    // so the map zoom and plot layout converge before we snapshot.
    harness.inner.run_steps(60);

    ui_tests::assert_the_capture_covers_the_map(&mut harness, "app_demo_trip");
    harness.snapshot_with_color_tolerance("app_demo_trip");
}

/// Channels plot as their own toggleable category: the Channels toggle
/// reveals the accel chip and its component lines, the chip hides them
/// again, and both toggles round-trip. The demo trip carries a 25 Hz accel
/// channel, so the snapshot shows real IMU-shaped lines beneath the metrics.
#[test]
fn snapshot_app_plot_channels() {
    let (mut harness, _config_path) = TestHarness::builder()
        .size(egui::vec2(1280.0, 800.0))
        .eframe(test_util::harness::build_app);
    harness.inner.step();

    test_util::harness::drop_file_and_wait_for_load(
        &mut harness.inner,
        TestDroppedFile::bytes(DEMO_BYTES, "demo_trip.gtd"),
    );
    harness.inner.run_steps(5);

    // Hidden by default: no channel chip until the section is revealed.
    assert!(
        harness.inner.query_by_label_contains("accel (g)").is_none(),
        "channel chips stay hidden while the section is collapsed"
    );

    harness.inner.get_by_label_contains("Channels").click();
    harness.inner.run_steps(3);
    assert!(
        harness
            .inner
            .state()
            .shared
            .borrow()
            .plot_state
            .show_channels,
        "the toggle reveals the channel section"
    );
    harness.inner.get_by_label_contains("accel (g)");

    // Declutter: keep only velocity and the channel visible so the snapshot
    // reads clearly (the accel lines sit near 1 g among km/h magnitudes).
    {
        let state = harness.inner.state_mut();
        let mut shared = state.shared.borrow_mut();
        let vis = &mut shared.plot_state.metric_vis;
        use strum::IntoEnumIterator as _;
        for kind in gt_types::MetricKind::iter() {
            vis.set(kind, kind == gt_types::MetricKind::Velocity);
        }
    }
    harness.inner.run_steps(5);
    harness.snapshot_with_color_tolerance("app_plot_channels");

    // The chip toggles the channel's lines off without collapsing the section.
    harness.inner.get_by_label_contains("accel (g)").click();
    harness.inner.run_steps(3);
    let shared = harness.inner.state().shared.borrow();
    assert!(
        !shared.plot_state.channel_vis.is_visible("accel"),
        "clicking the chip hides the channel"
    );
    assert!(
        shared.plot_state.show_channels,
        "the section stays revealed"
    );
}

/// Light-theme plot render with a spread of series enabled. The plot canvas is
/// pure-ish white on a light theme, where the dark-mode series palette was
/// invisible. This is the baseline that guards the theme-aware `metric_color`
/// light variants so a regression there fails CI.
#[test]
fn snapshot_app_plot_light() {
    let (mut harness, _config_path) = TestHarness::builder()
        .size(egui::vec2(1280.0, 800.0))
        .eframe(test_util::harness::build_app);
    harness.inner.step();

    test_util::harness::drop_file_and_wait_for_load(
        &mut harness.inner,
        TestDroppedFile::bytes(DEMO_BYTES, "demo_trip.gtd"),
    );

    // Force the light theme (startup settings default to system/dark in tests).
    harness.inner.ctx.set_theme(egui::ThemePreference::Light);

    // Enable a spread of seen/fix series across constellations so the snapshot
    // exercises the light palette's constellation coding and the seen-vs-fix
    // depth separation. (Util/slip families are advanced-gated and covered by
    // the contrast test.)
    {
        use gt_types::MetricKind as M;
        let shown = [
            M::SatsSeen,
            M::SatsFix,
            M::GpsSeen,
            M::GpsFix,
            M::GlonassSeen,
            M::GalileoSeen,
            M::BeidouSeen,
            M::Velocity,
        ];
        let state = harness.inner.state_mut();
        let mut shared = state.shared.borrow_mut();
        let vis = &mut shared.plot_state.metric_vis;
        use strum::IntoEnumIterator as _;
        for kind in M::iter() {
            vis.set(kind, shown.contains(&kind));
        }
    }
    harness.inner.run_steps(8);

    harness.snapshot_with_color_tolerance("app_plot_light");
}

/// The vector channel's per-component hues and the chip's hover legend, on
/// a fixture whose y-scale keeps the three accel lines visibly apart (the
/// demo-trip snapshot squeezes them into one line against velocity's
/// scale). The tooltip maps each component color square to its name.
#[test]
fn snapshot_app_plot_channel_components() {
    let (mut harness, _config_path) = TestHarness::builder()
        .size(egui::vec2(1280.0, 800.0))
        .eframe(test_util::harness::build_app);
    harness.inner.step();
    test_util::harness::drop_file_and_wait_for_load(
        &mut harness.inner,
        TestDroppedFile::bytes(ui_tests::accel_channel_gtd_bytes(0.9), "accel.gtd"),
    );
    harness.inner.run_steps(5);

    harness.inner.get_by_label_contains("Channels").click();
    harness.inner.run_steps(3);
    // Metric lines (satellite counts, heading at 20 deg) dwarf the accel
    // values. Hide them all so the y-scale lets the three component lines
    // separate visibly.
    {
        let state = harness.inner.state_mut();
        let mut shared = state.shared.borrow_mut();
        for kind in <gt_types::MetricKind as strum::IntoEnumIterator>::iter() {
            shared.plot_state.metric_vis.set(kind, false);
        }
    }
    harness.inner.run_steps(2);
    harness.inner.get_by_label_contains("accel (g)").hover();
    // Tooltips appear after egui's hover delay.
    for _ in 0..60 {
        harness.inner.run();
    }
    harness.snapshot_with_color_tolerance("app_plot_channel_components");
}

/// The channel chip's right-click menu carries one color entry per
/// component plus the reset - the editing surface that stays open, unlike
/// a hover tooltip.
#[test]
fn channel_chip_menu_offers_component_colors() {
    let (mut harness, _config_path) = TestHarness::builder()
        .size(egui::vec2(1280.0, 800.0))
        .eframe(test_util::harness::build_app);
    harness.inner.step();
    test_util::harness::drop_file_and_wait_for_load(
        &mut harness.inner,
        TestDroppedFile::bytes(ui_tests::accel_channel_gtd_bytes(0.9), "accel.gtd"),
    );
    harness.inner.run_steps(5);
    harness.inner.get_by_label_contains("Channels").click();
    harness.inner.run_steps(3);

    harness
        .inner
        .get_by_label_contains("accel (g)")
        .click_secondary();
    harness.inner.step();
    for label in ["Color of accel.x", "Color of accel.y", "Color of accel.z"] {
        assert!(
            harness.inner.query_by_label_contains(label).is_some(),
            "the chip menu should offer {label}"
        );
    }
    assert!(
        harness.inner.query_by_label("Reset colors").is_none(),
        "no reset without an override"
    );

    // With an override in place, the reset entry appears.
    harness.inner.key_press(egui::Key::Escape);
    harness.inner.run_steps(2);
    harness
        .inner
        .state_mut()
        .shared
        .borrow_mut()
        .plot_state
        .channel_component_colors
        .insert(
            "accel".to_owned(),
            vec![None, Some(egui::Color32::from_rgb(255, 0, 200)), None],
        );
    harness
        .inner
        .get_by_label_contains("accel (g)")
        .click_secondary();
    harness.inner.step();
    harness.inner.get_by_label("Reset colors").click_accesskit();
    harness.inner.run_steps(2);
    assert!(
        harness
            .inner
            .state()
            .shared
            .borrow()
            .plot_state
            .channel_component_colors
            .is_empty(),
        "reset must drop the channel's overrides"
    );
}

/// A user-picked component color reaches every surface at once: the line,
/// the chip's bar strip, and the hover legend square all draw `accel.y` in
/// the override colour.
#[test]
fn snapshot_app_plot_channel_color_override() {
    let (mut harness, _config_path) = TestHarness::builder()
        .size(egui::vec2(1280.0, 800.0))
        .eframe(test_util::harness::build_app);
    harness.inner.step();
    test_util::harness::drop_file_and_wait_for_load(
        &mut harness.inner,
        TestDroppedFile::bytes(ui_tests::accel_channel_gtd_bytes(0.9), "accel.gtd"),
    );
    harness.inner.run_steps(5);

    harness.inner.get_by_label_contains("Channels").click();
    harness.inner.run_steps(3);
    {
        let state = harness.inner.state_mut();
        let mut shared = state.shared.borrow_mut();
        for kind in <gt_types::MetricKind as strum::IntoEnumIterator>::iter() {
            shared.plot_state.metric_vis.set(kind, false);
        }
        shared.plot_state.channel_component_colors.insert(
            "accel".to_owned(),
            vec![None, Some(egui::Color32::from_rgb(255, 0, 200)), None],
        );
    }
    harness.inner.run_steps(2);
    harness.inner.get_by_label_contains("accel (g)").hover();
    for _ in 0..60 {
        harness.inner.run();
    }
    harness.snapshot_with_color_tolerance("app_plot_channel_color_override");
}

#[test]
fn snapshot_app_three_overlapping_files() {
    let (mut harness, _config_path) = TestHarness::builder()
        .size(egui::vec2(1280.0, 800.0))
        .eframe(test_util::harness::build_app_on_captured_tiles);
    harness.inner.step();
    ui_tests::load_three_overlapping_files(&mut harness.inner);
    assert_eq!(harness.inner.state().shared.borrow().loaded_files.len(), 3);

    {
        let mut state = harness.inner.state().shared.borrow_mut();
        state.zoom_to_visible_request = true;
    }
    harness.inner.run_steps(70);
    ui_tests::assert_the_capture_covers_the_map(&mut harness, "app_three_overlapping_files");
    // The side panel's italic recording name has one anti-aliased pixel at
    // (55, 357) that egui draws 91 or 84 gray, the second value in two of
    // seven runs of the whole test suite.
    harness.snapshot_with_tolerance(
        "app_three_overlapping_files",
        gt_test_utils::CROSS_BACKEND_COLOR_TOLERANCE,
        1,
    );
}

/// A recording whose clock offset holds near −234 ms, with one sample carrying
/// a 1 h 09 m recording gap - the `gnss.h5.gtd` case, where the receiver
/// reported its pre-gap GPS epoch for the first fix after resuming.
fn clock_excursion_gtd_bytes() -> Vec<u8> {
    use geotrace_sdk::{Angle, Duration as SdkDuration, NavFileBuilder, NavFix, NavFixTime};

    let start = ui_tests::base_time();
    let mut recorder = NavFileBuilder::new().open();
    for i in 0..61i64 {
        let gps = start + SdkDuration::seconds(i);
        let ahead_ms = if i == 10 { 4_127_054 } else { 234 };
        recorder.add_nav_fix(
            NavFix::builder()
                .time(NavFixTime::Both {
                    gps,
                    sys: gps + SdkDuration::milliseconds(ahead_ms),
                })
                .lat(Angle::degrees(51.5 + i as f64 * 0.0002))
                .lon(Angle::degrees(-0.1 - i as f64 * 0.00015))
                .heading(Angle::degrees(270.0))
                .eph_m(2.4)
                .build(),
        );
    }
    let nav_file = recorder.finish().expect("valid nav file");
    let mut bytes = Vec::new();
    nav_file.write(&mut bytes).expect("write bytes");
    bytes
}

/// The clock offset excursion overlay: the offset line keeps the track's own
/// sub-second scale and stops on either side of the sample carrying the
/// recording gap. That sample is marked with a down-pointing indicator at the
/// bottom edge, on a stub from the baseline.
#[test]
fn snapshot_app_plot_clock_excursion() {
    let gtd_bytes = clock_excursion_gtd_bytes();
    let mut harness = Harness::builder()
        .with_wait_for_pending_images(false)
        .build_eframe(test_util::harness::transient_app);
    test_util::harness::drop_file_and_wait_for_load(
        &mut harness,
        TestDroppedFile::bytes(gtd_bytes.as_slice(), "ride.gtd"),
    );

    {
        let state = harness.state_mut();
        let mut shared = state.shared.borrow_mut();
        let vis = &mut shared.plot_state.metric_vis;
        use strum::IntoEnumIterator as _;
        for kind in gt_types::MetricKind::iter() {
            vis.set(kind, kind == gt_types::MetricKind::ClockDeltaMs);
        }
    }
    harness.run_steps(5);

    let mut harness = gt_test_utils::TestHarness::from_harness(harness);
    harness.snapshot_with_color_tolerance("app_plot_clock_excursion");
}

/// The plot's snap error series from an injected completed run: the mint
/// line breaks over the unsnapped stretch and cross markers sit on the
/// baseline there. Only Eph stays enabled alongside so the snapshot shows
/// the claimed-accuracy vs. observed-deviation overlay the metric exists for.
#[test]
fn snapshot_app_plot_snap_error() {
    let gtd_bytes = ui_tests::minimal_gtd_bytes();
    let mut harness = Harness::builder()
        .with_wait_for_pending_images(false)
        .build_eframe(test_util::harness::transient_app);
    test_util::harness::drop_file_and_wait_for_load(
        &mut harness,
        TestDroppedFile::bytes(gtd_bytes.as_slice(), "ride.gtd"),
    );
    let track = gt_types::TrackRef::new(gt_types::FileIdx::new(0), gt_types::TrackIdx::new(0));
    test_util::snap::inject_completed_run(&mut harness, track);
    harness.run_steps(5);

    {
        let state = harness.state_mut();
        let mut shared = state.shared.borrow_mut();
        let vis = &mut shared.plot_state.metric_vis;
        use strum::IntoEnumIterator as _;
        for kind in gt_types::MetricKind::iter() {
            vis.set(
                kind,
                matches!(
                    kind,
                    gt_types::MetricKind::Eph | gt_types::MetricKind::SnapError
                ),
            );
        }
    }
    harness.run_steps(5);

    let mut harness = gt_test_utils::TestHarness::from_harness(harness);
    harness.snapshot_with_color_tolerance("app_plot_snap_error");
}

/// Every affected track the map indicator lists, as its label and its lines.
fn listed_track_warnings(harness: &Harness<'_, App>) -> Vec<(String, Vec<String>)> {
    harness
        .state()
        .space_weather_warning
        .track_warnings()
        .iter()
        .map(|warning| (warning.track_label.clone(), warning.lines.clone()))
        .collect()
}

/// The Kp line is drawn from the archive across the whole span the plot
/// shows: it runs past both ends of the recording into the margins, and
/// breaks over the day nothing is archived for while the recording runs
/// straight through it.
#[test]
fn snapshot_app_plot_context_line_spans_the_archived_days() {
    let gtd_bytes = gt_test_utils::synthetic_gtd_bytes(SyntheticGtdSpec {
        start: ui_tests::base_time(),
        point_count: 61,
        step_secs: 3600,
        start_lat_deg: 51.5,
        start_lon_deg: -0.1,
        lat_step_deg: 0.0002,
        lon_step_deg: -0.00015,
        heading_deg: 270.0,
        speed_kmh: 22.0,
        eph_m: 2.4,
        sats_seen: 10,
        sats_in_fix: 8,
    });
    let mut harness = Harness::builder()
        .with_wait_for_pending_images(false)
        .build_eframe(test_util::harness::transient_app);
    test_util::harness::drop_file_and_wait_for_load(
        &mut harness,
        TestDroppedFile::bytes(gtd_bytes.as_slice(), "ride.gtd"),
    );

    // The recording spans 22nd to 26th May 2025. The 24th stays unarchived,
    // and the days either side of the recording carry the margins.
    let dir = tempfile::tempdir().expect("temp dir");
    let store = gt_store::Store::open_in(dir.path())
        .open_or_create_archive::<SolarStore>()
        .expect("archive");
    let recorded = ui_tests::base_time().date_naive();
    for offset in [-1_i64, 0, 2, 3, 4] {
        let day = recorded + chrono::TimeDelta::days(offset);
        test_util::day_archive::archive_kp_day(&store, day, 2.0);
    }
    let ctx = harness.ctx.clone();
    harness.state_mut().geomagnetic_indices = crate::app::solar::GeomagneticIndexScheduler::new(
        ctx,
        Some(store),
        gt_solar::DEFAULT_BASE_URL.to_owned(),
        gt_fetch::TransportSource::Offline,
        gt_pending_writes::PendingWrites::default(),
    );

    {
        let state = harness.state_mut();
        let mut shared = state.shared.borrow_mut();
        let vis = &mut shared.plot_state.metric_vis;
        for kind in gt_types::MetricKind::iter() {
            vis.set(kind, kind == gt_types::MetricKind::Kp);
        }
    }
    harness.run_steps(5);
    // The archived storm days raise the space weather warning, whose toast
    // has its own snapshot.
    harness.state_mut().toasts.dismiss_all_toasts();
    harness.run_steps(2);

    let mut harness = gt_test_utils::TestHarness::from_harness(harness);
    harness.snapshot_with_color_tolerance("app_plot_context_line");
}

/// A geomagnetic day archived after the recording was loaded reaches it: the
/// warning line behind the map indicator appears and the load toast is raised
/// once, however many frames follow.
#[test]
fn a_storm_day_archived_after_the_load_warns_on_the_map() {
    let gtd_bytes = ui_tests::minimal_gtd_bytes();
    let mut harness = Harness::builder()
        .with_wait_for_pending_images(false)
        .build_eframe(test_util::harness::transient_app);
    test_util::harness::drop_file_and_wait_for_load(
        &mut harness,
        TestDroppedFile::bytes(gtd_bytes.as_slice(), "ride.gtd"),
    );
    harness.run_steps(2);
    assert!(
        harness
            .state()
            .space_weather_warning
            .track_warnings()
            .is_empty(),
        "no archived day overlaps the recording yet"
    );

    let dir = tempfile::tempdir().expect("temp dir");
    let store = gt_store::Store::open_in(dir.path())
        .open_or_create_archive::<SolarStore>()
        .expect("archive");
    test_util::day_archive::archive_kp_day(&store, ui_tests::base_time().date_naive(), 2.0);
    let ctx = harness.ctx.clone();
    harness.state_mut().geomagnetic_indices = crate::app::solar::GeomagneticIndexScheduler::new(
        ctx,
        Some(store),
        gt_solar::DEFAULT_BASE_URL.to_owned(),
        gt_fetch::TransportSource::Offline,
        gt_pending_writes::PendingWrites::default(),
    );
    harness.run_steps(2);

    assert_eq!(
        listed_track_warnings(&harness),
        [(
            "ride.gtd".to_owned(),
            vec!["Geomagnetic storm (≥5): Kp 5, G1".to_owned()]
        )],
        "the period the recording's fixes fall in is what it carries"
    );
    assert_eq!(harness.state().toasts.len(), 1);

    harness.run_steps(2);
    assert_eq!(
        harness.state().toasts.len(),
        1,
        "a later frame does not raise the toast again"
    );
}

/// The quiet-time window before a loaded recording arrives after it: the map
/// indicator states the deviation and the load toast is raised once, however
/// many frames follow.
#[test]
fn a_tec_window_archived_after_the_load_warns_on_the_map() {
    let gtd_bytes = ui_tests::minimal_gtd_bytes();
    let mut harness = Harness::builder()
        .with_wait_for_pending_images(false)
        .build_eframe(test_util::harness::transient_app);
    test_util::harness::drop_file_and_wait_for_load(
        &mut harness,
        TestDroppedFile::bytes(gtd_bytes.as_slice(), "ride.gtd"),
    );
    harness.run_steps(2);
    assert!(
        harness
            .state()
            .space_weather_warning
            .track_warnings()
            .is_empty(),
        "no archived day overlaps the recording yet"
    );

    let dir = tempfile::tempdir().expect("temp dir");
    let store = gt_store::Store::open_in(dir.path())
        .open_or_create_archive::<IonexStore>()
        .expect("archive");
    let recorded = ui_tests::base_time().date_naive();
    test_util::day_archive::archive_tec_day(&store, recorded, 35.0);
    for days_before in 1..=gt_ionex::quiet_time::BACKGROUND_WINDOW_DAYS as i64 {
        test_util::day_archive::archive_tec_day(
            &store,
            recorded - chrono::TimeDelta::days(days_before),
            20.0,
        );
    }
    let ctx = harness.ctx.clone();
    harness.state_mut().tec_maps = crate::app::tec::TecMapScheduler::new(
        ctx,
        Some(store),
        gt_ionex::MirrorList::default(),
        None,
        gt_fetch::TransportSource::Offline,
        gt_pending_writes::PendingWrites::default(),
    );
    harness.run_steps(2);

    assert_eq!(
        listed_track_warnings(&harness),
        [(
            "ride.gtd".to_owned(),
            vec![
                "ΔTEC (> +43%): +75% from the 27-day median, moderate ionospheric storm \
                 (W = 3), 24h"
                    .to_owned(),
                "TEC over track: 35–35 TECU".to_owned(),
            ]
        )],
        "every epoch of the recording's day stands well above the median of the 27 days before it"
    );
    assert_eq!(harness.state().toasts.len(), 1);

    harness.run_steps(2);
    assert_eq!(
        harness.state().toasts.len(),
        1,
        "a later frame does not raise the toast again"
    );

    let indices_dir = tempfile::tempdir().expect("temp dir");
    let indices = gt_store::Store::open_in(indices_dir.path())
        .open_or_create_archive::<SolarStore>()
        .expect("archive");
    test_util::day_archive::archive_kp_day(&indices, recorded - chrono::TimeDelta::days(1), 5.0);
    let ctx = harness.ctx.clone();
    harness.state_mut().geomagnetic_indices = crate::app::solar::GeomagneticIndexScheduler::new(
        ctx,
        Some(indices),
        gt_solar::DEFAULT_BASE_URL.to_owned(),
        gt_fetch::TransportSource::Offline,
        gt_pending_writes::PendingWrites::default(),
    );
    harness.run_steps(2);

    assert_eq!(
        listed_track_warnings(&harness),
        [(
            "ride.gtd".to_owned(),
            vec![
                "ΔTEC (> +43%): +75% from the 27-day median, moderate ionospheric storm \
                 (W = 3), 24h, after a G5 storm 21h before"
                    .to_owned(),
                "TEC over track: 35–35 TECU".to_owned(),
            ]
        )],
        "the day of indices archived after the load qualifies the deviation"
    );
}

/// A harness with a recording loaded and an archive of flares behind it,
/// returning the temp directory the archive lives in.
fn harness_with_archived_flares<'a>(
    archived: &[(i64, &[(u32, &str)])],
) -> (Harness<'a, App>, tempfile::TempDir) {
    let gtd_bytes = gt_test_utils::synthetic_gtd_bytes(SyntheticGtdSpec {
        start: ui_tests::base_time(),
        point_count: 61,
        step_secs: 3600,
        start_lat_deg: 51.5,
        start_lon_deg: -0.1,
        lat_step_deg: 0.0002,
        lon_step_deg: -0.00015,
        heading_deg: 270.0,
        speed_kmh: 22.0,
        eph_m: 2.4,
        sats_seen: 10,
        sats_in_fix: 8,
    });
    let mut harness = Harness::builder()
        .with_wait_for_pending_images(false)
        .build_eframe(test_util::harness::transient_app);
    test_util::harness::drop_file_and_wait_for_load(
        &mut harness,
        TestDroppedFile::bytes(gtd_bytes.as_slice(), "ride.gtd"),
    );

    let dir = tempfile::tempdir().expect("temp dir");
    let store = gt_store::Store::open_in(dir.path())
        .open_or_create_archive::<FlareStore>()
        .expect("archive");
    let recorded = ui_tests::base_time().date_naive();
    for &(offset, peaks) in archived {
        test_util::day_archive::archive_flare_day(
            &store,
            recorded + chrono::TimeDelta::days(offset),
            peaks,
        );
    }
    let ctx = harness.ctx.clone();
    harness.state_mut().solar_flares = crate::app::flares::SolarFlareScheduler::new(
        ctx,
        Some(store),
        gt_flare::DEFAULT_BASE_URL.to_owned(),
        gt_flare::ApiKey::new("test-key"),
        gt_fetch::TransportSource::Offline,
        gt_pending_writes::PendingWrites::default(),
    );
    harness.run_steps(5);
    // The archived flares raise the space weather warning, whose toast has
    // its own snapshot.
    harness.state_mut().toasts.dismiss_all_toasts();
    harness.run_steps(2);
    (harness, dir)
}

/// The markers are drawn from the archive across the whole span the plot
/// shows, so they reach past both ends of the recording, and each one takes
/// the colour of its class.
#[test]
fn snapshot_app_plot_solar_flare_markers() {
    // The plot shows the middle of the recording, so the day before it
    // carries a flare that is archived and outside the view.
    let (mut harness, _dir) = harness_with_archived_flares(&[
        (-1, &[(6, "C4.5")]),
        (0, &[(20, "C4.5")]),
        (1, &[(6, "M9.0"), (14, "X2.2")]),
        (2, &[(10, "X5.8")]),
    ]);

    {
        let state = harness.state_mut();
        let mut shared = state.shared.borrow_mut();
        let vis = &mut shared.plot_state.metric_vis;
        for kind in gt_types::MetricKind::iter() {
            vis.set(kind, kind == gt_types::MetricKind::Eph);
        }
    }
    harness.run_steps(5);

    let mut harness = gt_test_utils::TestHarness::from_harness(harness);
    harness.snapshot_with_color_tolerance("app_plot_solar_flare_markers");
}

#[test]
fn snapshot_app_environment_chip_hover() {
    let (mut harness, _archived) = harness_with_archived_flares(&[(0, &[(9, "X2.2")])]);
    // Tall enough for the whole tooltip to fit under the chip row.
    harness.set_size(egui::vec2(800.0, 900.0));
    harness.run_steps(3);
    harness.get_by_label(gt_flare::text::LAYER_LABEL).hover();
    // Tooltips appear after egui's hover delay.
    harness.run_steps(60);

    let mut harness = gt_test_utils::TestHarness::from_harness(harness);
    harness.snapshot_with_color_tolerance("app_environment_chip_hover");
}

/// With nothing archived the flare chip renders disabled - visible, not
/// hidden - and an archived day enables it.
#[test]
fn solar_flare_chip_is_disabled_until_a_day_is_archived() {
    let (harness, _empty) = harness_with_archived_flares(&[]);
    let chip = harness.get_by_label(gt_flare::text::LAYER_LABEL);
    assert!(
        chip.accesskit_node().is_disabled(),
        "the chip must render disabled while no flare is archived"
    );

    let (harness, _archived) = harness_with_archived_flares(&[(0, &[(9, "X2.2")])]);
    let chip = harness.get_by_label(gt_flare::text::LAYER_LABEL);
    assert!(
        !chip.accesskit_node().is_disabled(),
        "an archived flare enables the chip"
    );
}

/// Without a completed run the snap error chip renders disabled - visible,
/// not hidden - and enabling runs changes nothing else about the chip row.
#[test]
fn snap_error_chip_is_disabled_until_a_run_completes() {
    let gtd_bytes = ui_tests::minimal_gtd_bytes();
    let mut harness = Harness::builder()
        .with_wait_for_pending_images(false)
        .build_eframe(test_util::harness::transient_app);
    test_util::harness::drop_file_and_wait_for_load(
        &mut harness,
        TestDroppedFile::bytes(gtd_bytes.as_slice(), "ride.gtd"),
    );
    let track = gt_types::TrackRef::new(gt_types::FileIdx::new(0), gt_types::TrackIdx::new(0));
    harness.run_steps(3);

    let chip = harness.get_by_label("Snap error (m)");
    assert!(
        chip.accesskit_node().is_disabled(),
        "the chip must render disabled while no run has completed"
    );

    test_util::snap::inject_completed_run(&mut harness, track);
    harness.run_steps(3);
    let chip = harness.get_by_label("Snap error (m)");
    assert!(
        !chip.accesskit_node().is_disabled(),
        "a completed run enables the chip"
    );
}

/// `snap_error_view` resolves each sent point's `PointIdx` to its plot time
/// and mirrors the kind. Unsnapped points keep `error_m: None`.
#[test]
fn snap_error_view_resolves_point_times_and_kinds() {
    let mut harness = Harness::builder()
        .with_wait_for_pending_images(false)
        .build_eframe(test_util::harness::transient_app);
    harness.step();
    let track = ui_tests::push_file_with_travel_mode(&mut harness, "ride.gtd", None);
    test_util::snap::inject_completed_run(&mut harness, track);

    let view = harness.state_mut().snap_error_view();
    let points = view
        .points_by_track
        .get(&track)
        .expect("the completed run must reach the plot view");
    assert_eq!(points.len(), 60, "one entry per injected sent point");

    let shared = harness.state().shared.borrow();
    let loaded = track
        .resolve(shared.loaded_files.files())
        .expect("track present");
    let first_time = loaded.points[0].tpv.time().as_secs_f64();
    assert!(
        (points[0].x_secs - first_time).abs() < f64::EPSILON,
        "x must be the point's own plot time"
    );
    assert_eq!(points[0].kind, gt_ui_types::SnapErrorKind::Snapped);
    assert_eq!(points[20].kind, gt_ui_types::SnapErrorKind::Unsnapped);
    assert_eq!(points[20].error_m, None);
    assert_eq!(points[21].kind, gt_ui_types::SnapErrorKind::Unsnapped);
    assert_eq!(points[25].kind, gt_ui_types::SnapErrorKind::Interpolated);
    assert!(points[25].error_m.is_some());
}
