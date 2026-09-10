use std::path::PathBuf;

use egui_kittest::{Harness, kittest::NodeT as _, kittest::Queryable as _};
use gt_test_utils::{By, HarnessInteraction as _, TestHarness};
use strum::IntoEnumIterator as _;

use crate::app::App;
use crate::app::settings_ui::{self, SettingsPage};
use crate::app::test_util;
use crate::app::test_util::harness::TestDroppedFile;
use crate::app::ui_tests;

impl SettingsPage {
    /// The control this page renders last, which is the first to fall out of
    /// the window when the page grows.
    fn last_control_label(self) -> &'static str {
        match self {
            Self::Processing => "Restore defaults",
            Self::Analysis => "Mark backward time steps",
            Self::AircraftInterference
            | Self::GeomagneticIndices
            | Self::IonosphericTec
            | Self::SolarFlares => crate::app::backfill_ui::DOWNLOAD_HISTORY_LABEL,
            Self::SnapToRoad => "GPS accuracy",
            Self::Interface => "Mapbox token",
            Self::Application => crate::app::environment_storage_ui::AUTO_PRUNE_LABEL,
        }
    }

    /// Gated with the snapshot test that uses it: without `self-update` the
    /// Application page renders one row fewer and no baseline matches.
    #[cfg(feature = "self-update")]
    fn snapshot_file_stem(self) -> &'static str {
        match self {
            Self::Processing => "settings_window_processing",
            Self::Analysis => "settings_window_analysis",
            Self::AircraftInterference => "settings_window_aircraft_interference",
            Self::GeomagneticIndices => "settings_window_geomagnetic_indices",
            Self::IonosphericTec => "settings_window_ionospheric_tec",
            Self::SolarFlares => "settings_window_solar_flares",
            Self::SnapToRoad => "settings_window_snap_to_road",
            Self::Interface => "settings_window_interface",
            Self::Application => "settings_window_application",
        }
    }
}

/// Opens the settings window so every page renders the same way on every run:
/// both download ranges are pinned and one snap option is set.
fn harness_with_settings_window_open<'a>() -> (TestHarness<'a, App>, PathBuf) {
    let (mut harness, config_path) = TestHarness::builder()
        .size(egui::vec2(940.0, 720.0))
        .eframe(test_util::harness::build_app);
    harness.inner.step();
    // Search radius set (its drag value active), the other two unset (grayed,
    // never hidden): the snap page shows both states of its optional rows.
    harness.inner.state_mut().snap_settings.search_radius_m = Some(25.0);
    harness.inner.state_mut().settings_open = true;
    ui_tests::pin_settings_dates(harness.inner.state_mut());
    (harness, config_path)
}

// The Application page renders a `self-update`-only row (the update check), so
// the window's appearance depends on that feature. Gating the snapshot on it means
// the reference image can only ever be generated and compared in the same
// configuration CI uses (`just test` / `just test-snapshots` both enable it).
// Without this, regenerating snapshots in a build that lacks the feature would
// silently drop that page and break macOS CI. Any future feature-dependent
// snapshot must be gated the same way.
#[cfg(feature = "self-update")]
#[test]
fn snapshot_settings_pages() {
    let (mut harness, _config_path) = harness_with_settings_window_open();
    for page in SettingsPage::iter() {
        harness.inner.state_mut().settings_page = page;
        harness.run();
        harness.snapshot_with_color_tolerance(page.snapshot_file_stem());
    }
}

/// The window opens at one size that holds every page at default content, and
/// keeps that size as the page changes.
#[test]
fn settings_window_keeps_one_size_across_pages() {
    let (mut harness, _config_path) = harness_with_settings_window_open();
    let mut opened_size = None;
    for page in SettingsPage::iter() {
        harness.inner.state_mut().settings_page = page;
        harness.run();

        let window_rect = harness
            .inner
            .ctx
            .memory(|m| m.area_rect(egui::Id::new(settings_ui::WINDOW_ID)))
            .expect("the settings window is open");
        let last_control = harness
            .inner
            .get_by_label_contains(page.last_control_label())
            .rect();
        assert!(
            last_control.max.y <= window_rect.max.y,
            "{:?} overflows the window: {} ends at {}, the window at {}",
            page,
            page.last_control_label(),
            last_control.max.y,
            window_rect.max.y
        );

        let opened_size = *opened_size.get_or_insert(window_rect.size());
        assert_eq!(
            window_rect.size(),
            opened_size,
            "{page:?} resized the window"
        );
    }
}

/// Types `query` into the settings window's search field. The field is focused
/// by its own id: the app behind the window renders text fields of its own,
/// which [`HarnessInteraction::type_into_text_input`] would match as well.
fn type_into_settings_search(harness: &mut TestHarness<'_, App>, query: &str) {
    harness.inner.ctx.memory_mut(|memory| {
        memory.request_focus(egui::Id::new(settings_ui::search::QUERY_FIELD_ID));
    });
    harness.run();
    harness
        .inner
        .input_mut()
        .events
        .push(egui::Event::Text(query.to_owned()));
    harness.run();
}

/// A renamed row must not leave the search behind: every label a page declares
/// searchable is one the page renders.
#[test]
fn every_settings_page_renders_the_labels_it_declares() {
    let (mut harness, _config_path) = harness_with_settings_window_open();
    for page in SettingsPage::iter() {
        harness.inner.state_mut().settings_page = page;
        harness.run();

        let window_rect = harness
            .inner
            .ctx
            .memory(|m| m.area_rect(egui::Id::new(settings_ui::WINDOW_ID)))
            .expect("the settings window is open");
        for label in page.searchable_labels() {
            assert!(
                harness
                    .inner
                    .query_all_by_label_contains(label)
                    .any(|node| window_rect.contains_rect(node.rect())),
                "{page:?} declares {label:?} searchable but renders no such label"
            );
        }
    }
}

/// A source page's reference link opens the reference window on that source's
/// material.
#[rstest::rstest]
#[case(
    SettingsPage::GeomagneticIndices,
    gt_solar::reference::GEOMAGNETIC_ACTIVITY
)]
#[case(SettingsPage::IonosphericTec, gt_ionex::reference::IONOSPHERIC_TEC)]
#[case(SettingsPage::SolarFlares, gt_flare::reference::SOLAR_FLARES)]
#[case(
    SettingsPage::AircraftInterference,
    gt_jam::reference::AIRCRAFT_INTERFERENCE
)]
fn a_source_page_opens_its_reference_window(
    #[case] page: SettingsPage,
    #[case] document: gt_ui_types::reference::ReferenceDocument,
) {
    let (mut harness, _config_path) = harness_with_settings_window_open();
    harness.inner.state_mut().settings_page = page;
    harness.run();

    harness
        .inner
        .get_by_label_contains(document.link_question)
        .click();
    harness.run();

    assert!(harness.inner.state().reference_window.is_open());
    assert!(
        harness
            .inner
            .query_all_by_label_contains(document.title)
            .next()
            .is_some(),
        "the reference window shows its title"
    );
}

#[test]
fn an_empty_query_lists_every_page_in_the_rail() {
    let (mut harness, _config_path) = harness_with_settings_window_open();
    harness.run();
    for page in SettingsPage::iter() {
        assert!(
            harness
                .inner
                .query_all_by_label_contains(page.rail_label())
                .next()
                .is_some(),
            "{page:?} is missing from the rail"
        );
    }
}

#[rstest::rstest]
#[case::lowercase("elevation")]
#[case::mixed_case("ElevAtion")]
fn clicking_a_search_match_opens_its_page(#[case] query: &str) {
    let (mut harness, _config_path) = harness_with_settings_window_open();
    harness.run();
    type_into_settings_search(&mut harness, query);

    assert!(
        harness
            .inner
            .query_all_by_label_contains(SettingsPage::SnapToRoad.rail_label())
            .next()
            .is_none(),
        "a page the query does not reach stays out of the rail"
    );
    assert_eq!(
        harness.inner.state().settings_page,
        SettingsPage::Processing
    );

    harness
        .inner
        .get_by_label_contains("Elevation mask")
        .click();
    harness.run();
    assert_eq!(harness.inner.state().settings_page, SettingsPage::Analysis);
}

/// One Escape press dismisses one level: the query first, the window second.
#[test]
fn escape_clears_the_query_before_it_closes_the_window() {
    let (mut harness, _config_path) = harness_with_settings_window_open();
    harness.run();
    type_into_settings_search(&mut harness, "elevation");

    ui_tests::press_escape(&mut harness.inner);
    harness.run();
    assert!(
        harness.inner.state().settings_open,
        "the first Escape clears the query and leaves the window open"
    );
    assert!(
        harness
            .inner
            .query_all_by_label_contains(SettingsPage::SnapToRoad.rail_label())
            .next()
            .is_some(),
        "the cleared query restores the whole rail"
    );

    ui_tests::press_escape(&mut harness.inner);
    harness.run();
    assert!(
        !harness.inner.state().settings_open,
        "the second Escape closes the window"
    );
}

#[test]
fn snapshot_settings_window_search_matches() {
    let (mut harness, _config_path) = harness_with_settings_window_open();
    harness.run();
    type_into_settings_search(&mut harness, "clock");
    harness.run();
    harness.snapshot_with_color_tolerance("settings_window_search_matches");
}

/// Load one recording and give it the metadata the name template draws on.
fn load_recording_with_metadata(harness: &mut Harness<App>) {
    test_util::harness::drop_file_and_wait_for_load(
        harness,
        TestDroppedFile::bytes(ui_tests::minimal_gtd_bytes(), "ride.gtd"),
    );
    let mut shared = harness.state().shared.borrow_mut();
    if let Some(file) = shared.loaded_files.get_mut(0) {
        file.metadata.title = Some("Morning ride".to_owned());
        file.metadata.device = Some("u-blox F9P".to_owned());
    }
    shared.recording_name_template = "{title} - {device}".to_owned();
}

/// A stored recording for the guide's preview line to fall back on.
fn stored_recording_entry(identity: &str, title: &str) -> gt_store::RecordingEntry {
    let mut entry = test_util::listing::entry_with_identity(identity);
    entry.total_tracks = 1;
    entry.title = Some(title.to_owned());
    entry.device = Some("u-blox F9P".to_owned());
    entry
}

/// The template guide opens while the field has focus and previews the template
/// twice: with every token filled by its own name, and on a real recording. A
/// loaded recording is the preview's source even when history holds one too.
#[test]
fn name_template_guide_previews_the_loaded_recording() {
    let mut harness = Harness::builder()
        .with_wait_for_pending_images(false)
        .with_size(egui::vec2(820.0, 620.0))
        .build_eframe(test_util::harness::transient_app);
    harness.step();
    load_recording_with_metadata(&mut harness);
    harness
        .state_mut()
        .history_window
        .set_entries(vec![stored_recording_entry(
            "auto:stored.gtd",
            "Stored ride",
        )]);
    harness.state_mut().settings_open = true;
    harness.state_mut().settings_page = SettingsPage::Interface;
    harness.run_steps(3);
    assert!(
        harness.query_by_label("title - device").is_none(),
        "the guide must stay closed until the field takes focus"
    );

    harness.get_by_label_contains("Recording name").focus();
    harness.run_steps(3);

    harness.get_by_label("title - device");
    ui_tests::node_outside_the_side_panel(&harness, "Morning ride - u-blox F9P");
}

/// With nothing loaded, the preview falls back to the most recent recording in
/// history, which takes its file name from its identity.
#[test]
fn name_template_guide_previews_a_history_recording() {
    let mut harness = Harness::builder()
        .with_wait_for_pending_images(false)
        .with_size(egui::vec2(820.0, 620.0))
        .build_eframe(test_util::harness::transient_app);
    harness.step();
    {
        let mut older = stored_recording_entry("auto:older.gtd", "Older ride");
        older.meta.time_range = gt_store::NavPointTimeRange::covering(&[1_000]);
        let mut newest = stored_recording_entry("auto:newest.gtd", "Newest ride");
        newest.meta.time_range = gt_store::NavPointTimeRange::covering(&[2_000]);
        harness
            .state_mut()
            .history_window
            .set_entries(vec![older, newest]);
        harness.state().shared.borrow_mut().recording_name_template =
            "{title} - {identity} - {filename}".to_owned();
    }
    harness.state_mut().settings_open = true;
    harness.state_mut().settings_page = SettingsPage::Interface;
    harness.run_steps(3);

    harness.get_by_label_contains("Recording name").focus();
    harness.run_steps(3);

    harness.get_by_label("Newest ride - newest.gtd - newest.gtd");
}

/// Both previews follow the field as it is typed in.
#[test]
fn name_template_guide_previews_follow_the_typed_template() {
    let mut harness = Harness::builder()
        .with_wait_for_pending_images(false)
        .with_size(egui::vec2(820.0, 620.0))
        .build_eframe(test_util::harness::transient_app);
    harness.step();
    load_recording_with_metadata(&mut harness);
    harness.state_mut().settings_open = true;
    harness.state_mut().settings_page = SettingsPage::Interface;
    harness.run_steps(3);

    let field = harness.get_by_label_contains("Recording name");
    field.focus();
    field.type_text(" ({identity})");
    harness.run_steps(3);

    assert_eq!(
        harness.state().shared.borrow().recording_name_template,
        "{title} - {device} ({identity})"
    );
    harness.get_by_label("title - device (identity)");
    ui_tests::node_outside_the_side_panel(&harness, "Morning ride - u-blox F9P (ride.gtd)");
}

/// With no recording loaded and none in history, the preview line explains why
/// it is empty.
#[test]
fn name_template_guide_explains_a_missing_recording() {
    let mut harness = Harness::builder()
        .with_wait_for_pending_images(false)
        .with_size(egui::vec2(820.0, 620.0))
        .build_eframe(test_util::harness::transient_app);
    harness.step();
    harness.state_mut().settings_open = true;
    harness.state_mut().settings_page = SettingsPage::Interface;
    harness.run_steps(3);

    harness.get_by_label_contains("Recording name").focus();
    harness.run_steps(3);

    harness.get_by_label("No recording loaded or in history");
}

/// The Interface page's token field drives the token the map fetches satellite
/// tiles with. It is the page's last text field, below the name template.
#[test]
fn the_interface_page_edits_the_token_the_map_reads() {
    let mut harness = Harness::builder()
        .with_wait_for_pending_images(false)
        .with_size(egui::vec2(820.0, 620.0))
        .build_eframe(test_util::harness::transient_app);
    harness.step();
    harness.state_mut().settings_open = true;
    harness.state_mut().settings_page = SettingsPage::Interface;
    harness.run_steps(3);

    let field = harness.bottommost_matching(By::new().role(egui::accesskit::Role::TextInput));
    field.focus();
    field.type_text("token-from-settings");
    harness.run_steps(2);
    harness.key_press(egui::Key::Enter);
    harness.run_steps(2);

    assert_eq!(harness.state().map.mapbox_token(), "token-from-settings");
}

/// The page grays the satellite layer until a token is set, per DESIGN.md. Its
/// entry is the topmost "Satellite" match: the map's own ungated picker renders
/// the same entry lower on screen.
#[test]
fn the_interface_page_gates_the_satellite_layer_on_a_token() {
    let mut harness = Harness::builder()
        .with_wait_for_pending_images(false)
        .with_size(egui::vec2(820.0, 620.0))
        .build_eframe(test_util::harness::transient_app);
    harness.step();
    harness.state_mut().settings_open = true;
    harness.state_mut().settings_page = SettingsPage::Interface;
    harness.run_steps(3);

    assert!(
        harness
            .topmost_matching(By::new().label_contains("Satellite"))
            .accesskit_node()
            .is_disabled()
    );

    harness.state_mut().map.set_mapbox_token("tok".to_owned());
    harness.run_steps(2);
    harness
        .topmost_matching(By::new().label_contains("Satellite"))
        .click();
    harness.run_steps(2);

    assert_eq!(harness.state().map.layer(), gt_map::MapLayer::Satellite);
}

/// The guide as it shows while the user edits the template: the token list, an
/// example, and both preview lines. The preview recording comes from history, so
/// no track ink renders behind the window.
#[test]
fn snapshot_recording_name_template_guide() {
    let (mut harness, _config_path) = TestHarness::builder()
        .size(egui::vec2(820.0, 620.0))
        .eframe(test_util::harness::build_app);
    harness.inner.step();
    harness
        .inner
        .state_mut()
        .history_window
        .set_entries(vec![stored_recording_entry(
            "auto:ride.gtd",
            "Morning ride",
        )]);
    harness
        .inner
        .state()
        .shared
        .borrow_mut()
        .recording_name_template = "{title} - {device}".to_owned();
    harness.inner.state_mut().settings_open = true;
    harness.inner.state_mut().settings_page = SettingsPage::Interface;
    harness.inner.run_steps(3);
    harness
        .inner
        .get_by_label_contains("Recording name")
        .focus();
    harness.inner.run_steps(3);
    harness.snapshot_with_color_tolerance("recording_name_template_guide");
}
