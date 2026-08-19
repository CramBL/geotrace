use egui_kittest::kittest::{By, Queryable as _};
use gt_solar::reference::GEOMAGNETIC_ACTIVITY;
use gt_test_utils::{HarnessInteraction as _, TestHarness};
use gt_ui_types::reference::{
    Abbreviation, ColumnWidth, ReferenceBlock, ReferenceDocument, ReferenceTable, TableCell,
    TableColumn,
};
use rstest::rstest;

use super::{ReferenceWindow, WRAPPING_COLUMN_WIDTH, decode_frame};

/// Room for the window at its default size.
const HARNESS_SIZE: egui::Vec2 = egui::vec2(1120.0, 820.0);

/// Far enough to reach the end of any document the window holds.
const SCROLL_TO_END_POINTS: f32 = 6000.0;

/// Frames the scroll's smooth animation takes to come to rest.
const SCROLL_SETTLE_FRAMES: usize = 12;

fn harness_showing<'a>(
    document: ReferenceDocument,
    dark_mode: bool,
) -> TestHarness<'a, ReferenceWindow> {
    let mut window = ReferenceWindow::new();
    window.open(document);
    let mut harness = TestHarness::builder()
        .size(HARNESS_SIZE)
        .theme(dark_mode)
        .ui_state(
            |ui: &mut egui::Ui, window: &mut ReferenceWindow| window.show(ui.ctx()),
            window,
        );
    harness.run();
    harness
}

/// Scrolls the window's content to its end, where the sources footer sits.
fn scroll_to_end(harness: &mut TestHarness<'_, ReferenceWindow>, document: ReferenceDocument) {
    let Some(window_rect) = harness.inner.window_rect(document.title) else {
        panic!("the reference window is open");
    };
    harness.inner.hover_at(window_rect.center());
    harness
        .inner
        .input_mut()
        .events
        .push(egui::Event::MouseWheel {
            unit: egui::MouseWheelUnit::Point,
            delta: egui::vec2(0.0, -SCROLL_TO_END_POINTS),
            phase: egui::TouchPhase::Move,
            modifiers: egui::Modifiers::NONE,
        });
    harness.inner.run_steps(SCROLL_SETTLE_FRAMES);
}

/// The urls of the [`egui::OutputCommand::OpenUrl`] commands in the output of
/// the frame just run.
fn opened_urls(harness: &TestHarness<'_, ReferenceWindow>) -> Vec<String> {
    harness
        .inner
        .output()
        .platform_output
        .commands
        .iter()
        .filter_map(|command| match command {
            egui::OutputCommand::OpenUrl(open_url) => Some(open_url.url.clone()),
            egui::OutputCommand::CopyText(_) | egui::OutputCommand::CopyImage(_) => None,
        })
        .collect()
}

#[derive(Debug, Clone, Copy)]
enum DocumentPosition {
    Top,
    End,
}

/// Both themes at both ends of the document. The end holds the illustration
/// pair, the query example in the editor's syntax colors, and the sources
/// footer.
#[rstest]
#[case(true, DocumentPosition::Top, "reference_window_geomagnetic")]
#[case(false, DocumentPosition::Top, "reference_window_geomagnetic_light")]
#[case(true, DocumentPosition::End, "reference_window_geomagnetic_end")]
#[case(false, DocumentPosition::End, "reference_window_geomagnetic_end_light")]
fn snapshot_reference_window_geomagnetic(
    #[case] dark_mode: bool,
    #[case] position: DocumentPosition,
    #[case] snapshot_name: &str,
) {
    let mut harness = harness_showing(GEOMAGNETIC_ACTIVITY, dark_mode);
    if matches!(position, DocumentPosition::End) {
        scroll_to_end(&mut harness, GEOMAGNETIC_ACTIVITY);
    }
    harness.snapshot(snapshot_name);
}

#[test]
fn hovering_an_abbreviation_shows_its_full_form_under_the_help_cursor() {
    let mut harness = harness_showing(GEOMAGNETIC_ACTIVITY, true);
    let first_abbreviation = harness
        .inner
        .topmost_matching(By::new().label("GNSS"))
        .rect()
        .center();
    harness.inner.hover_at_and_settle(first_abbreviation, 3);
    assert!(
        harness
            .inner
            .query_by_label_contains("Global Navigation Satellite System")
            .is_some(),
        "the hover shows the full form"
    );
    assert_eq!(
        harness.inner.output().platform_output.cursor_icon,
        egui::CursorIcon::Help
    );
}

#[test]
fn the_sources_footer_links_every_source() {
    let mut harness = harness_showing(GEOMAGNETIC_ACTIVITY, true);
    scroll_to_end(&mut harness, GEOMAGNETIC_ACTIVITY);
    for source in GEOMAGNETIC_ACTIVITY.sources {
        assert!(
            harness
                .inner
                .query_all(By::new().label(source.name))
                .next()
                .is_some(),
            "the footer links {:?}",
            source.name
        );
    }
}

#[test]
fn hovering_a_source_shows_its_url() {
    let mut harness = harness_showing(GEOMAGNETIC_ACTIVITY, true);
    scroll_to_end(&mut harness, GEOMAGNETIC_ACTIVITY);
    let Some(source) = GEOMAGNETIC_ACTIVITY.sources.first() else {
        panic!("the document lists sources");
    };
    harness
        .inner
        .hover_and_settle(By::new().label(source.name), 3);
    assert!(
        harness.inner.query_by_label_contains(source.url).is_some(),
        "the hover shows {:?}",
        source.url
    );
}

/// A clicked hyperlink adds an [`egui::OutputCommand::OpenUrl`] to the
/// platform output, and eframe's `links` feature opens that url in the system
/// browser.
#[test]
fn clicking_a_source_opens_its_url() {
    let mut harness = harness_showing(GEOMAGNETIC_ACTIVITY, true);
    scroll_to_end(&mut harness, GEOMAGNETIC_ACTIVITY);
    let Some(source) = GEOMAGNETIC_ACTIVITY.sources.first() else {
        panic!("the document lists sources");
    };
    harness.inner.get_by_label(source.name).click();
    harness.step();
    assert_eq!(opened_urls(&harness), vec![source.url]);
}

#[test]
fn closing_the_window_drops_the_document() {
    let mut harness = harness_showing(GEOMAGNETIC_ACTIVITY, true);
    harness.inner.get_by_label("Close window").click();
    harness.run();
    assert!(!harness.state().is_open());
}

/// The committed illustrations decode with the codecs the app builds in.
#[test]
fn every_illustration_frame_decodes() {
    for block in GEOMAGNETIC_ACTIVITY.blocks {
        if let ReferenceBlock::Illustration(illustration) = block {
            for frame in illustration.frames {
                assert!(
                    decode_frame(frame).is_some(),
                    "{} decodes",
                    frame.asset_name
                );
            }
        }
    }
}

/// The queries the material offers run as written, so a change to the query
/// language cannot leave the reference material handing out a broken example.
#[test]
fn every_query_example_checks_clean() {
    for text in GEOMAGNETIC_ACTIVITY.query_examples() {
        let query = gt_query::parse(text)
            .unwrap_or_else(|diagnostic| panic!("{text:?} parses: {diagnostic:?}"));
        gt_query::check(&query, &gt_query::ChannelSchema::new())
            .unwrap_or_else(|diagnostic| panic!("{text:?} checks: {diagnostic:?}"));
    }
}

/// A cell long enough to need a second line in the column it belongs to.
const WRAPPING_CELL_PROSE: &str = "A cell whose sentence runs past the width of the column it \
                                   belongs to, ending on [GNSS].";

const WRAPPING_TABLE_DOCUMENT: ReferenceDocument = ReferenceDocument {
    title: "Wrapping table",
    link_question: "How does a wrapping table affect GNSS?",
    blocks: &[ReferenceBlock::Table(ReferenceTable {
        title: "One wrapping column",
        columns: &[TableColumn {
            header: "Effects",
            width: ColumnWidth::Wraps,
        }],
        rows: &[&[TableCell::Prose(WRAPPING_CELL_PROSE)]],
    })],
    abbreviations: &[Abbreviation {
        short_form: "GNSS",
        full_form: "Global Navigation Satellite System",
    }],
    sources: &[],
};

/// Prose in a column with a width of its own wraps inside that width.
#[test]
fn a_long_cell_wraps_within_its_column() {
    let harness = harness_showing(WRAPPING_TABLE_DOCUMENT, true);
    let cell = harness
        .inner
        .get_by_label_contains("A cell whose sentence")
        .rect();
    let last_span = harness.inner.get_by_label("GNSS").rect();

    assert!(
        cell.width() <= WRAPPING_COLUMN_WIDTH,
        "the cell laid out in {} points, wider than the column",
        cell.width()
    );
    assert!(
        last_span.top() > cell.top(),
        "the cell ends on the line it starts on"
    );
}
