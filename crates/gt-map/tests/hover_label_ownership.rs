//! Which hover labels the map has open, frame by frame, and in which order it
//! stacks them where several layers reach the pointer at once.
//!
//! A case counts the layers a frame left open at [`egui::Order::Tooltip`] and
//! reads each one's lines out of the accesskit tree: every map label draws
//! there, and none of them go through egui's hover checks.
//!
//! A case draws one frame past the one a label opens on, and then reads the
//! stacking order off the screen positions the labels settled at: egui lays a
//! tooltip out away from where it will be drawn until the frame after it
//! opens.

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

/// The margin `egui_kittest` lays a `ui_state` harness out with, which insets
/// the map from the viewport.
const HARNESS_OUTER_MARGIN_PT: f32 = 8.0;

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

/// The table of the fix at [`CENTRE_FIX`], which is the fix the pointer
/// reaches at [`viewport_center`].
fn the_centre_fix_label() -> String {
    format!(
        "Time\n2023-11-14 22:28:20\n\
         Lat\n55.676000° N\n\
         Lon\n12.580000° E\n\
         Speed\n{EM_DASH}\n\
         Heading\n{EM_DASH}"
    )
}

/// The snapped edge's label is anchored at the response for the whole map,
/// which draws it in the map's bottom-left corner.
fn assert_the_snapped_edge_label_is_in_the_map_corner(map: &RenderedMap) {
    let drawn_at = map
        .hover_labels()
        .into_iter()
        .find(|label| label.text == THE_SNAPPED_EDGE_LABEL)
        .map(|edge| (edge.rect.left(), edge.rect.bottom()));
    assert_eq!(
        drawn_at,
        Some((HARNESS_OUTER_MARGIN_PT, support::VIEWPORT.y)),
        "the edge label is drawn in the map's bottom-left corner"
    );
}

/// The layer a case puts under the log hexagon, whose label the hexagon's
/// stacks over.
#[derive(Clone, Copy)]
enum TheLayerUnderTheHexagon {
    InterferenceCell,
    DrawnFix,
}

/// The hexagon's label stands at the pointer and the label of the layer under
/// it right below, in the order the map draws the two layers.
#[rstest]
#[case::over_an_interference_cell(
    TheLayerUnderTheHexagon::InterferenceCell,
    THE_INTERFERENCE_CELL_LABEL.to_owned(),
    "hover_label_a_log_hexagon_over_an_interference_cell"
)]
#[case::over_a_drawn_fix(
    TheLayerUnderTheHexagon::DrawnFix,
    the_centre_fix_label(),
    "hover_label_a_log_hexagon_over_a_drawn_fix"
)]
fn snapshot_a_log_hexagon_stacks_its_label_over_the_label_of_the_layer_under_it(
    #[case] under: TheLayerUnderTheHexagon,
    #[case] the_label_underneath: String,
    #[case] snapshot_name: &str,
) {
    let files = a_recording_of(FIX_COUNT, WALKING_STEP_DEGREES);
    let centre = fix_position(&files, CENTRE_FIX);
    let log = a_log_over(&files);
    let matches = matches_over(&files, &log, CENTRE_FIX..CENTRE_FIX + 1);
    let scene = RenderedMapScene::of(files)
        .with_log_matches(matches)
        .centred_on(centre);
    let mut map = match under {
        TheLayerUnderTheHexagon::InterferenceCell => scene
            .showing_the_interference_layer(an_interference_cell_around(centre))
            .hiding_the_fix_icons(),
        TheLayerUnderTheHexagon::DrawnFix => scene,
    }
    .draw();

    map.move_pointer_to(viewport_center());
    map.draw_one_more_frame();
    map.draw_one_more_frame();

    assert_eq!(
        map.hover_label_texts_top_to_bottom(),
        [THE_LOG_HEXAGON_LABEL.to_owned(), the_label_underneath]
    );
    map.snapshot(snapshot_name);
}

/// A fix over an interference cell stacks its table over the cell's label: the
/// track line draws over the cells.
#[test]
fn snapshot_a_fix_stacks_its_table_over_the_label_of_the_interference_cell_under_it() {
    let files = a_recording_of(FIX_COUNT, WALKING_STEP_DEGREES);
    let centre = fix_position(&files, CENTRE_FIX);
    let mut map = RenderedMapScene::of(files)
        .showing_the_interference_layer(an_interference_cell_around(centre))
        .centred_on(centre)
        .draw();

    map.move_pointer_to(viewport_center());
    map.draw_one_more_frame();
    map.draw_one_more_frame();

    assert_eq!(
        map.hover_label_texts_top_to_bottom(),
        [
            the_centre_fix_label(),
            THE_INTERFERENCE_CELL_LABEL.to_owned()
        ]
    );
    map.snapshot("hover_label_a_fix_over_an_interference_cell");
}

/// A fix and an event marker at one position show the compound label, which
/// stacks over the interference cell's label the way the individual labels do.
#[test]
fn snapshot_the_compound_label_stacks_over_the_label_of_the_interference_cell_under_it() {
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
    let mut map = RenderedMapScene::of(files)
        .showing_the_interference_layer(an_interference_cell_around(centre))
        .centred_on(centre)
        .draw();

    map.move_pointer_to(viewport_center());
    map.draw_one_more_frame();
    map.draw_one_more_frame();

    assert_eq!(
        map.hover_label_texts_top_to_bottom(),
        [the_compound_label, THE_INTERFERENCE_CELL_LABEL.to_owned()]
    );
    map.snapshot("hover_label_the_compound_label_over_an_interference_cell");
}

/// What a case draws over the snapped edge, whose own label keeps the map's
/// corner whatever else labels the pointer.
#[derive(Clone, Copy)]
enum TheLayerOverTheSnappedEdge {
    LogHexagon,
    DrawnFix,
    InterferenceCell,
}

/// The layer over the snapped edge labels the pointer, and the edge labels the
/// map's bottom-left corner.
#[rstest]
#[case::a_log_hexagon(
    TheLayerOverTheSnappedEdge::LogHexagon,
    THE_LOG_HEXAGON_LABEL.to_owned(),
    "hover_label_a_log_hexagon_over_a_snapped_edge"
)]
#[case::a_drawn_fix(
    TheLayerOverTheSnappedEdge::DrawnFix,
    the_centre_fix_label(),
    "hover_label_a_fix_over_a_snapped_edge"
)]
#[case::an_interference_cell(
    TheLayerOverTheSnappedEdge::InterferenceCell,
    THE_INTERFERENCE_CELL_LABEL.to_owned(),
    "hover_label_an_interference_cell_over_a_snapped_edge"
)]
fn snapshot_the_snapped_edge_labels_the_map_corner_while_the_layer_over_it_labels_the_pointer(
    #[case] over: TheLayerOverTheSnappedEdge,
    #[case] the_label_at_the_pointer: String,
    #[case] snapshot_name: &str,
) {
    let files = a_recording_of(FIX_COUNT, WALKING_STEP_DEGREES);
    let centre = fix_position(&files, CENTRE_FIX);
    let log = a_log_over(&files);
    let matches = matches_over(&files, &log, CENTRE_FIX..CENTRE_FIX + 1);
    let scene = RenderedMapScene::of(files)
        .with_snapped_tracks(a_snapped_edge_through(centre))
        .centred_on(centre);
    let mut map = match over {
        TheLayerOverTheSnappedEdge::LogHexagon => {
            scene.with_log_matches(matches).hiding_the_fix_icons()
        }
        TheLayerOverTheSnappedEdge::DrawnFix => scene,
        TheLayerOverTheSnappedEdge::InterferenceCell => scene
            .showing_the_interference_layer(an_interference_cell_around(centre))
            .hiding_the_fix_icons(),
    }
    .draw();

    map.move_pointer_to(viewport_center());
    map.draw_one_more_frame();
    map.draw_one_more_frame();

    assert_eq!(
        map.hover_label_texts_top_to_bottom(),
        [the_label_at_the_pointer, THE_SNAPPED_EDGE_LABEL.to_owned()]
    );
    assert_the_snapped_edge_label_is_in_the_map_corner(&map);
    map.snapshot(snapshot_name);
}

/// The snapped edge alone labels the map's bottom-left corner, far from the
/// pointer that reached it.
#[test]
fn snapshot_the_snapped_edge_label_is_drawn_in_the_map_corner_and_not_beside_the_pointer() {
    /// How far from the pointer the label is drawn, in points. The pointer
    /// rests at the middle of an 800 by 600 viewport and the label in its
    /// bottom-left corner.
    const DISTANCE_FROM_THE_POINTER_PT: f32 = 300.0;

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

    assert_eq!(map.hover_label_texts(), [THE_SNAPPED_EDGE_LABEL]);
    assert_the_snapped_edge_label_is_in_the_map_corner(&map);
    let labels = map.hover_labels();
    let edge = labels.first().expect("the edge label is open");
    let distance = edge.rect.distance_to_pos(pointer);
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

/// The interference cell's label draws while a popup holds the map: a popup
/// replaces the label of a recorded element, and leaves the labels of the
/// layers under the track alone.
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
/// event marker at one position. Nothing is open on the arrival frame: the
/// labels of the recorded elements state the previous frame's hit test, which
/// found nothing where the pointer came from.
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

/// A fix opens its table the frame after the pointer reaches it, while the
/// snapped edge under it labels the corner from the arrival frame on: the
/// edge's hit test runs on the frame it draws.
#[test]
fn snapshot_the_fix_table_opens_the_frame_after_the_pointer_reaches_a_fix_on_a_snapped_edge() {
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
    assert_eq!(
        map.hover_label_texts(),
        [THE_SNAPPED_EDGE_LABEL],
        "the arrival frame already shows the fix's table"
    );

    map.draw_one_more_frame();
    map.draw_one_more_frame();
    assert_eq!(
        map.hover_label_texts_top_to_bottom(),
        [the_fix_label, THE_SNAPPED_EDGE_LABEL.to_owned()]
    );
    map.snapshot("hover_label_a_fix_that_the_pointer_reached_on_a_snapped_edge");
}
