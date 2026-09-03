//! Which hover labels the map has open, frame by frame, where two layers reach
//! the pointer at once.
//!
//! A case counts the layers a frame left open at [`egui::Order::Tooltip`] and
//! reads each one's lines out of the accesskit tree: every map label draws
//! there, and none of them go through egui's hover checks.
//!
//! A case snapshots one frame later than the count it asserts: egui paints a
//! label from the frame after the one where its layer opens.

mod support;

use egui_phosphor::regular::CROSSHAIR as ICON_CROSSHAIR;
use egui_phosphor::regular::FLAG as ICON_FLAG;
use gt_ui_theme::EM_DASH;
use rstest::rstest;
use support::{
    RenderedMap, RenderedMapScene, WALKING_STEP_DEGREES, a_log_over, a_recording_of,
    a_snapped_edge_through, an_interference_cell_around, fix_position, matches_over,
    viewport_center, with_an_event_marker_on_a_fix,
};

/// Fixes of the walking recording every case but the last draws.
const FIX_COUNT: usize = 30;

/// The fix the camera is held on, which puts it at [`viewport_center`].
const CENTRE_FIX: usize = 15;

/// How far north of the track the bare-map point sits, in points.
const NORTH_OF_THE_TRACK_OFFSET_PT: f32 = 150.0;

/// A point of bare map north of the track: out of reach of every fix, and
/// inside the interference cell where a case draws one.
fn bare_map_north_of_the_track() -> egui::Pos2 {
    viewport_center() + egui::vec2(0.0, -NORTH_OF_THE_TRACK_OFFSET_PT)
}

/// The label of the edge from [`a_snapped_edge_through`].
const THE_SNAPPED_EDGE_LABEL: &str = "H.C. Andersens Boulevard\n\
    Road class\nTertiary\n\
    Speed limit\n50 km/h\n\
    Surface\nPaved smooth";

/// The label of the cell from [`an_interference_cell_around`].
const THE_INTERFERENCE_CELL_LABEL: &str = "2 of 100 aircraft reported low navigation accuracy\n\
    2.0% over 2023-11-14 (UTC)";

/// The label of the hexagon over the centre fix, listing the one entry
/// [`a_log_over`] wrote there.
const THE_LOG_HEXAGON_LABEL: &str = "22:28:20  tracklogd[311]: heading hold engaged";

/// The interference cell's label closes on the frame after a snapped edge over
/// it takes the pointer. The interference layer reads the snapped renderer's
/// flag one frame late: the interference cells paint first.
///
/// No frame paints the two labels together. The arrival frame has both open and
/// paints only the cell's, whose layer opened a frame earlier. The next frame
/// paints the edge's label alone.
#[test]
fn snapshot_the_interference_cell_label_closes_the_frame_after_a_snapped_edge_takes_the_pointer() {
    let files = a_recording_of(FIX_COUNT, WALKING_STEP_DEGREES);
    let centre = fix_position(&files, CENTRE_FIX);
    let mut map = RenderedMapScene::of(files)
        .showing_the_interference_layer(an_interference_cell_around(centre))
        .with_snapped_tracks(a_snapped_edge_through(centre))
        .hiding_the_fix_icons()
        .centred_on(centre)
        .draw();

    map.move_pointer_to(bare_map_north_of_the_track());
    map.draw_one_more_frame();
    assert_eq!(map.hover_label_texts(), [THE_INTERFERENCE_CELL_LABEL]);

    map.move_pointer_to(viewport_center());
    assert_eq!(
        map.hover_label_texts(),
        [THE_INTERFERENCE_CELL_LABEL, THE_SNAPPED_EDGE_LABEL]
    );
    map.snapshot("hover_label_the_pointer_arrives_on_a_snapped_edge_over_a_cell");

    map.draw_one_more_frame();
    assert_eq!(map.hover_label_texts(), [THE_SNAPPED_EDGE_LABEL]);
    map.snapshot("hover_label_the_snapped_edge_owns_the_pointer_over_a_cell");
}

#[derive(Clone, Copy)]
enum TheLayerUnderTheHexagon {
    SnappedEdge,
    InterferenceCell,
}

/// The label of the layer under a log hexagon closes on the frame after the
/// hexagon takes the pointer: both layers read the hexagon hit test of the
/// previous frame. The arrival frame has both labels open. Neither paints on
/// that frame.
#[rstest]
#[case::over_a_snapped_edge(
    TheLayerUnderTheHexagon::SnappedEdge,
    [THE_LOG_HEXAGON_LABEL, THE_SNAPPED_EDGE_LABEL],
    "hover_label_a_log_hexagon_over_a_snapped_edge"
)]
#[case::over_an_interference_cell(
    TheLayerUnderTheHexagon::InterferenceCell,
    [THE_INTERFERENCE_CELL_LABEL, THE_LOG_HEXAGON_LABEL],
    "hover_label_a_log_hexagon_over_an_interference_cell"
)]
fn snapshot_a_log_hexagon_takes_the_pointer_from_the_layer_under_it(
    #[case] under: TheLayerUnderTheHexagon,
    #[case] labels_on_the_arrival_frame: [&str; 2],
    #[case] snapshot_name: &str,
) {
    let files = a_recording_of(FIX_COUNT, WALKING_STEP_DEGREES);
    let centre = fix_position(&files, CENTRE_FIX);
    let log = a_log_over(&files);
    let matches = matches_over(&files, &log, CENTRE_FIX..CENTRE_FIX + 1);
    let scene = RenderedMapScene::of(files)
        .with_log_matches(matches)
        .hiding_the_fix_icons()
        .centred_on(centre);
    let mut map = match under {
        TheLayerUnderTheHexagon::SnappedEdge => {
            scene.with_snapped_tracks(a_snapped_edge_through(centre))
        }
        TheLayerUnderTheHexagon::InterferenceCell => {
            scene.showing_the_interference_layer(an_interference_cell_around(centre))
        }
    }
    .draw();

    map.move_pointer_to(viewport_center());
    assert_eq!(map.hover_label_texts(), labels_on_the_arrival_frame);

    map.draw_one_more_frame();
    assert_eq!(map.hover_label_texts(), [THE_LOG_HEXAGON_LABEL]);
    map.snapshot(snapshot_name);
}

/// A fix under a log hexagon draws no label of its own: the hexagon takes the
/// pointer on the frame it is reached, and from the next frame the fix is the
/// hover candidate with its label suppressed.
#[test]
fn snapshot_a_log_hexagon_leaves_the_fix_under_it_without_a_label() {
    let the_fix_label = format!(
        "Time\n2023-11-14 22:28:20\n\
         Lat\n55.676000° N\n\
         Lon\n12.580000° E\n\
         Speed\n{EM_DASH}\n\
         Heading\n{EM_DASH}"
    );
    let files = a_recording_of(FIX_COUNT, WALKING_STEP_DEGREES);
    let centre = fix_position(&files, CENTRE_FIX);

    let mut without_the_hexagon = RenderedMapScene::of(files).centred_on(centre).draw();
    without_the_hexagon.move_pointer_to(viewport_center());
    without_the_hexagon.draw_one_more_frame();
    assert_eq!(
        without_the_hexagon.hover_label_texts(),
        [the_fix_label.as_str()],
        "the fix at the centre of the viewport draws its own label"
    );

    let files = a_recording_of(FIX_COUNT, WALKING_STEP_DEGREES);
    let log = a_log_over(&files);
    let matches = matches_over(&files, &log, CENTRE_FIX..CENTRE_FIX + 1);
    let mut map = RenderedMapScene::of(files)
        .with_log_matches(matches)
        .centred_on(centre)
        .draw();

    map.move_pointer_to(viewport_center());
    assert_eq!(map.hover_label_texts(), [THE_LOG_HEXAGON_LABEL]);

    map.draw_one_more_frame();
    assert_eq!(map.hover_label_texts(), [THE_LOG_HEXAGON_LABEL]);
    map.snapshot("hover_label_a_log_hexagon_over_a_drawn_fix");
}

/// The snapped edge's label is drawn in the map's bottom-left corner. Its
/// renderer anchors the label at the response for the whole map and not at the
/// pointer.
#[test]
fn snapshot_the_snapped_edge_label_is_drawn_in_the_map_corner_and_not_beside_the_pointer() {
    /// How far from the pointer the label is drawn, in points. The pointer
    /// rests at the middle of an 800 by 600 viewport and the label in its
    /// bottom-left corner.
    const DISTANCE_FROM_THE_POINTER_PT: f32 = 300.0;

    /// The margin `egui_kittest` lays a `ui_state` harness out with, which
    /// insets the map from the viewport.
    const HARNESS_OUTER_MARGIN_PT: f32 = 8.0;

    let files = a_recording_of(FIX_COUNT, WALKING_STEP_DEGREES);
    let centre = fix_position(&files, CENTRE_FIX);
    let mut map = RenderedMapScene::of(files)
        .with_snapped_tracks(a_snapped_edge_through(centre))
        .hiding_the_fix_icons()
        .centred_on(centre)
        .draw();

    let pointer = viewport_center();
    map.move_pointer_to(pointer);
    map.draw_one_more_frame();

    let labels = map.hover_labels();
    let label = labels.first().expect("the edge label is open");
    assert_eq!(label.text, THE_SNAPPED_EDGE_LABEL);
    assert_eq!(
        (label.rect.left(), label.rect.bottom()),
        (HARNESS_OUTER_MARGIN_PT, support::VIEWPORT.y),
        "the edge label is drawn in the map's bottom-left corner"
    );
    let distance = label.rect.distance_to_pos(pointer);
    assert!(
        distance > DISTANCE_FROM_THE_POINTER_PT,
        "the edge label is drawn {distance} pt from the pointer"
    );
    map.snapshot("hover_label_a_snapped_edge_alone");
}

/// The popup a case opens over the map before it moves the pointer onto the
/// interference cell.
#[derive(Clone, Copy)]
enum PopupOverTheMap {
    /// What a click opens where a fix and an event marker sit at one position.
    Disambiguation,
    /// What a secondary click opens on the element under the pointer. A
    /// secondary click on bare map opens nothing: the menu closes itself on the
    /// frame it finds no element.
    ContextMenu,
}

impl PopupOverTheMap {
    fn open_on(self, map: &mut RenderedMap, target: egui::Pos2) {
        match self {
            Self::Disambiguation => map.click_at(target),
            Self::ContextMenu => map.secondary_click_at(target),
        }
    }

    fn is_open(self, map: &RenderedMap) -> bool {
        match self {
            Self::Disambiguation => map.disambiguation_popup_is_open(),
            Self::ContextMenu => map.any_popup_is_open(),
        }
    }
}

/// The interference cell's label draws while a popup holds the map: neither the
/// disambiguation popup nor the context menu reaches the interference layer's
/// own hit test.
#[rstest]
#[case::the_disambiguation_popup(
    PopupOverTheMap::Disambiguation,
    "hover_label_an_interference_cell_under_the_disambiguation_popup"
)]
#[case::the_context_menu(
    PopupOverTheMap::ContextMenu,
    "hover_label_an_interference_cell_under_the_context_menu"
)]
fn snapshot_the_interference_cell_label_draws_while_a_popup_holds_the_map(
    #[case] popup: PopupOverTheMap,
    #[case] snapshot_name: &str,
) {
    let files =
        with_an_event_marker_on_a_fix(a_recording_of(FIX_COUNT, WALKING_STEP_DEGREES), CENTRE_FIX);
    let centre = fix_position(&files, CENTRE_FIX);
    let mut map = RenderedMapScene::of(files)
        .showing_the_interference_layer(an_interference_cell_around(centre))
        .centred_on(centre)
        .draw();

    map.move_pointer_to(viewport_center());
    map.draw_one_more_frame();
    popup.open_on(&mut map, viewport_center());
    map.draw_one_more_frame();
    assert!(popup.is_open(&map), "the click opened no popup");

    map.move_pointer_to(bare_map_north_of_the_track());
    map.draw_one_more_frame();
    assert!(popup.is_open(&map), "the pointer move closed the popup");
    assert_eq!(map.hover_label_texts(), [THE_INTERFERENCE_CELL_LABEL]);

    map.draw_one_more_frame();
    assert_eq!(map.hover_label_texts(), [THE_INTERFERENCE_CELL_LABEL]);
    map.snapshot(snapshot_name);
}

/// The compound label opens the frame after the pointer reaches a fix and an
/// event marker at one position. Nothing is open on the arrival frame. The
/// renderers read the hover candidates of the previous frame, and the compound
/// label draws only once those candidates are ambiguous.
#[test]
fn snapshot_the_compound_label_opens_the_frame_after_the_pointer_reaches_two_elements() {
    let the_compound_label = format!(
        "{ICON_CROSSHAIR}  GNSS fix  22:28:20\n\
         Time\n2023-11-14 22:28:20\n\
         Lat\n55.676000° N\n\
         Lon\n12.580000° E\n\
         Speed\n{EM_DASH}\n\
         Heading\n{EM_DASH}\n\
         {ICON_FLAG}  power/boot"
    );
    let files =
        with_an_event_marker_on_a_fix(a_recording_of(FIX_COUNT, WALKING_STEP_DEGREES), CENTRE_FIX);
    let centre = fix_position(&files, CENTRE_FIX);
    let mut map = RenderedMapScene::of(files).centred_on(centre).draw();

    map.move_pointer_to(bare_map_north_of_the_track());
    map.draw_one_more_frame();

    map.move_pointer_to(viewport_center());
    assert_eq!(
        map.hover_label_texts(),
        Vec::<String>::new(),
        "the arrival frame has a label open"
    );
    map.snapshot("hover_label_the_pointer_arrives_on_a_fix_and_a_marker");

    map.draw_one_more_frame();
    assert_eq!(map.hover_label_texts(), [the_compound_label.as_str()]);

    map.draw_one_more_frame();
    assert_eq!(map.hover_label_texts(), [the_compound_label.as_str()]);
    map.snapshot("hover_label_the_compound_label_over_a_fix_and_a_marker");
}

/// The fix's label replaces the snapped edge's on the frame after the pointer
/// reaches the fix, and no frame has both open: the snapped renderer stops
/// hit-testing while a recorded element is the hover candidate.
#[test]
fn snapshot_the_fix_label_replaces_the_snapped_edge_label_with_no_frame_showing_both() {
    /// Fixes of a recording sparse enough for the snapped edge to run on past
    /// the last of them, where the pointer reaches the edge alone.
    const SPARSE_FIX_COUNT: usize = 4;

    /// How far east of the last fix the pointer rests on the bare edge, in
    /// points. The fit puts the fixes about 200 pt apart, and a fix takes the
    /// pointer within 20 pt.
    const BARE_EDGE_OFFSET_PT: f32 = 120.0;

    let the_fix_label = format!(
        "Time\n2023-11-14 22:16:20\n\
         Lat\n55.676000° N\n\
         Lon\n12.568000° E\n\
         Speed\n{EM_DASH}\n\
         Heading\n{EM_DASH}"
    );
    let files = a_recording_of(SPARSE_FIX_COUNT, WALKING_STEP_DEGREES);
    let last_fix = SPARSE_FIX_COUNT - 1;
    let centre = fix_position(&files, last_fix);
    let mut map = RenderedMapScene::of(files)
        .with_snapped_tracks(a_snapped_edge_through(centre))
        .centred_on(centre)
        .draw();

    map.move_pointer_to(viewport_center() + egui::vec2(BARE_EDGE_OFFSET_PT, 0.0));
    map.draw_one_more_frame();
    assert_eq!(map.hover_label_texts(), [THE_SNAPPED_EDGE_LABEL]);

    map.move_pointer_to(viewport_center());
    assert_eq!(map.hover_label_texts(), [THE_SNAPPED_EDGE_LABEL]);

    map.draw_one_more_frame();
    assert_eq!(map.hover_label_texts(), [the_fix_label.as_str()]);

    map.draw_one_more_frame();
    assert_eq!(map.hover_label_texts(), [the_fix_label.as_str()]);
    map.snapshot("hover_label_a_fix_that_took_the_pointer_from_a_snapped_edge");
}
