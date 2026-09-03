use egui::InteractOptions;
use egui_phosphor::regular::CLOCK as ICON_CLOCK;
use gt_types::{LoadedTrack, TrackMetadata};

use crate::widgets;

/// The header over the track numbers, which are drawn bare.
const NUMBER_COLUMN_HEADER: &str = "#";

/// The header over the distances, which are drawn without their unit.
const DISTANCE_COLUMN_HEADER: &str = "km";

/// The header over the durations, which are drawn as `h:mm:ss`.
const DURATION_COLUMN_HEADER: &str = ICON_CLOCK;

/// The cells of one track row, in the Visible section and in the tree. The
/// units are stated once in the header row, not repeated per cell.
pub struct TrackColumnCells {
    number: String,
    /// `None` for a track with no measured geometry, drawn as an em dash.
    distance_km: Option<String>,
    duration: String,
}

impl TrackColumnCells {
    pub fn for_track(track: &LoadedTrack) -> Self {
        Self {
            number: track.metadata.index.to_string(),
            distance_km: track
                .geometry
                .measured()
                .map(|geometry| gt_fmt::format_kilometers(geometry.distance_km)),
            duration: gt_fmt::DurationClockFormat::HoursMinutesSeconds
                .format_seconds(track.metadata.duration.num_seconds()),
        }
    }

    fn distance(&self) -> &str {
        self.distance_km.as_deref().unwrap_or(gt_ui_theme::EM_DASH)
    }

    /// What a screen reader announces for the row: the drawn cells, the
    /// distance with the unit its header states.
    pub fn accessibility_label(&self) -> String {
        let distance = match &self.distance_km {
            Some(km) => format!("{km} km"),
            None => self.distance().to_owned(),
        };
        format!("#{}  {distance}  {}", self.number, self.duration)
    }

    pub fn paint(
        &self,
        ui: &mut egui::Ui,
        TrackColumnWidths {
            number,
            distance,
            duration,
        }: TrackColumnWidths,
        color: egui::Color32,
    ) {
        let font = egui::TextStyle::Body.resolve(ui.style());
        let align = egui::Align2::RIGHT_CENTER;
        paint_column_cell(ui, number, &self.number, &font, color, align);
        paint_column_cell(ui, distance, self.distance(), &font, color, align);
        paint_column_cell(ui, duration, &self.duration, &font, color, align);
    }
}

#[derive(Clone, Copy)]
pub struct TrackColumnWidths {
    number: f32,
    distance: f32,
    duration: f32,
}

impl TrackColumnWidths {
    /// The width of the widest cell of each column, over the header and the
    /// tracks the caller draws.
    pub fn measure<'c>(ui: &egui::Ui, cells: impl Iterator<Item = &'c TrackColumnCells>) -> Self {
        let cell_font = egui::TextStyle::Body.resolve(ui.style());
        let header_font = egui::TextStyle::Small.resolve(ui.style());
        let mut widths = Self {
            number: widgets::text_width(ui, NUMBER_COLUMN_HEADER, &header_font),
            distance: widgets::text_width(ui, DISTANCE_COLUMN_HEADER, &header_font),
            duration: widgets::text_width(ui, DURATION_COLUMN_HEADER, &header_font),
        };
        let Self {
            number,
            distance,
            duration,
        } = &mut widths;
        for row in cells {
            *number = number.max(widgets::text_width(ui, &row.number, &cell_font));
            *distance = distance.max(widgets::text_width(ui, row.distance(), &cell_font));
            *duration = duration.max(widgets::text_width(ui, &row.duration, &cell_font));
        }
        widths
    }
}

/// Paints `text` at `align` in a cell `width` wide and returns the cell. The
/// values of a column line up under each other: the row's layout advances past
/// each cell.
pub fn paint_column_cell(
    ui: &mut egui::Ui,
    width: f32,
    text: &str,
    font: &egui::FontId,
    color: egui::Color32,
    align: egui::Align2,
) -> egui::Rect {
    let (_, rect) = ui.allocate_space(egui::vec2(width, ui.spacing().interact_size.y));
    ui.painter()
        .text(align.pos_in_rect(&rect), align, text, font.clone(), color);
    rect
}

/// The header row over the columns of the track rows below it. A hairline
/// under it spans the columns, from the number column's left edge to the
/// duration column's right edge. `leading_space` puts each header over its
/// column: it is the width those rows spend before their first cell.
pub fn render_header(
    ui: &mut egui::Ui,
    leading_space: f32,
    TrackColumnWidths {
        number,
        distance,
        duration,
    }: TrackColumnWidths,
) {
    let font = egui::TextStyle::Small.resolve(ui.style());
    let color = ui.visuals().text_color();
    let align = egui::Align2::CENTER_CENTER;
    let row = ui.horizontal(|ui| {
        ui.add_space(leading_space);
        let first = paint_column_cell(ui, number, NUMBER_COLUMN_HEADER, &font, color, align);
        paint_column_cell(ui, distance, DISTANCE_COLUMN_HEADER, &font, color, align);
        let last = paint_column_cell(ui, duration, DURATION_COLUMN_HEADER, &font, color, align);
        first.left()..=last.right()
    });
    ui.painter().hline(
        row.inner,
        row.response.rect.bottom(),
        ui.visuals().widgets.noninteractive.bg_stroke,
    );
}

/// What colors a track row's cells, the conditions in the order they win.
#[derive(Clone, Copy)]
pub struct TrackRowCellColor {
    pub panel_hovered: bool,
    pub selected: bool,
    pub passes_filter: bool,
}

impl TrackRowCellColor {
    pub fn resolve(self, ui: &egui::Ui) -> egui::Color32 {
        if self.panel_hovered {
            gt_ui_theme::HIGHLIGHT_BLUE
        } else if self.selected {
            ui.visuals().selection.stroke.color
        } else if self.passes_filter {
            ui.visuals().text_color()
        } else {
            ui.visuals().weak_text_color()
        }
    }
}

/// Fills the shape the row reserved before it drew its cells, while the row is
/// selected or hovered.
pub fn paint_row_background(
    ui: &egui::Ui,
    background: egui::layers::ShapeIdx,
    response: &egui::Response,
    selected: bool,
) {
    let fill = if selected {
        ui.visuals().selection.bg_fill
    } else if response.hovered() {
        ui.visuals().widgets.hovered.weak_bg_fill
    } else {
        return;
    };
    let corner_radius = ui.visuals().widgets.hovered.corner_radius;
    ui.painter().set(
        background,
        egui::Shape::rect_filled(response.rect, corner_radius, fill),
    );
}

/// The controls a track row draws, each registered as the row draws it.
///
/// A control shows its own tooltip while the pointer is inside its rect,
/// whether or not egui marks the control as hovered. The row shows its own
/// tooltip only while the pointer is over none of them.
#[derive(Default)]
pub struct TrackRowControls {
    pointer_is_over_a_control: bool,
}

impl TrackRowControls {
    pub fn register(&mut self, control: &egui::Response) {
        self.pointer_is_over_a_control |= control.contains_pointer();
    }
}

/// Draws a track row as one interactive surface. `add_contents` fills the row
/// over its whole width and registers every control it draws. Each of those
/// controls keeps its own clicks: the row's sense registers below them. The
/// returned response has the row's accessibility label, and the track stats
/// tooltip while the pointer is over none of the row's controls.
pub fn render_row_as_one_surface<T>(
    ui: &mut egui::Ui,
    metadata: &TrackMetadata,
    cells: &TrackColumnCells,
    is_selected: bool,
    add_contents: impl FnOnce(&mut egui::Ui, &mut TrackRowControls) -> T,
) -> (egui::Response, T) {
    // Claimed before the row draws: its fill belongs behind the cells.
    let background = ui.painter().add(egui::Shape::Noop);
    let row_width = ui.available_width();
    let mut controls = TrackRowControls::default();
    let row = ui.horizontal(|ui| {
        ui.set_min_width(row_width);
        add_contents(ui, &mut controls)
    });
    let response = ui.interact_opt(
        row.response.rect,
        row.response.id,
        egui::Sense::click(),
        InteractOptions { move_to_top: false },
    );
    let response = if controls.pointer_is_over_a_control {
        response
    } else {
        response.on_hover_ui(|ui| widgets::track_tooltip_rows(ui, metadata))
    };
    response.widget_info(|| {
        egui::WidgetInfo::selected(
            egui::WidgetType::SelectableLabel,
            true,
            is_selected,
            cells.accessibility_label(),
        )
    });
    paint_row_background(ui, background, &response, is_selected);
    (response, row.inner)
}
