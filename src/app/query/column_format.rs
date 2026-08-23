//! How one match-table column prints: the unit its header names, and the fixed
//! decimals its cells line up on.

use chrono::{DateTime, Utc};
use egui::{Align, CursorIcon, Label, Layout, RichText, Sense, TextStyle, TextWrapMode};
use geotrace_sdk_units::{ChannelUnit, Unit};
use gt_query::{Construct, Quantity, QueryMetric};
use gt_query_run::MICROS_PER_SEC;
use gt_ui_theme::{DEGREE_SIGN, EM_DASH};

use super::value_bar::ValueBar;

/// Decimals a channel sample prints, matching the plot's channel readout.
const CHANNEL_DECIMALS: usize = 3;

/// Integer digits a value column budgets for: its width is then the same
/// whichever rows are on screen. Four covers every metric a column holds
/// today - a wider value is cut off where its column ends.
const BUDGETED_INTEGER_DIGITS: usize = 4;

/// How one column of a match table prints its values.
#[derive(Debug, Clone, Copy)]
pub(super) struct ColumnFormat<'a> {
    /// What the header names the column in, absent where the cells hold times
    /// or bare numbers.
    unit: Option<&'a str>,
    /// Factor from the base unit the evaluator works in to `unit`.
    from_base: f64,
    /// Decimals every cell prints, so a column lines up on its decimal point.
    decimals: usize,
    kind: ColumnKind,
}

/// What a column's cells hold.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ColumnKind {
    /// A wall-clock time, to the millisecond where the values are timed finer
    /// than the points are.
    TimeOfDay {
        millis: bool,
    },
    Number,
    /// Nothing printable. Only [`Quantity::Condition`] lands here, and no
    /// metric carries it.
    Blank,
}

impl<'a> ColumnFormat<'a> {
    /// A column of numbers in `unit`, scaled out of the base unit.
    fn number(unit: Option<&'a str>, from_base: f64, decimals: usize) -> Self {
        Self {
            unit,
            from_base,
            decimals,
            kind: ColumnKind::Number,
        }
    }

    /// A column of wall-clock times, to the second.
    pub(super) fn time_of_day() -> Self {
        Self::time(false)
    }

    /// A column of wall-clock times carrying the milliseconds that separate
    /// values timed finer than the points are.
    pub(super) fn time_of_day_with_millis() -> Self {
        Self::time(true)
    }

    /// A column whose cells stay empty.
    fn blank() -> Self {
        Self {
            unit: None,
            from_base: 1.0,
            decimals: 0,
            kind: ColumnKind::Blank,
        }
    }

    fn time(millis: bool) -> Self {
        Self {
            unit: None,
            from_base: 1.0,
            decimals: 0,
            kind: ColumnKind::TimeOfDay { millis },
        }
    }

    /// How the column for `metric` prints, from the quantity it measures.
    pub(super) fn of_metric(metric: QueryMetric) -> Self {
        match metric.quantity() {
            Quantity::Timestamp => Self::time_of_day(),
            Quantity::Angle | Quantity::Direction => {
                Self::number(Some(DEGREE_SIGN), Unit::DEG.from_base(), 1)
            }
            Quantity::Speed => Self::number(Some("km/h"), Unit::KM_PER_H.from_base(), 1),
            Quantity::Acceleration => Self::number(Some("m/s²"), Unit::M_PER_S2.from_base(), 2),
            Quantity::Length => Self::number(Some("m"), Unit::M.from_base(), 1),
            Quantity::Duration => Self::number(Some("s"), Unit::S.from_base(), 3),
            Quantity::Count => Self::number(None, 1.0, 0),
            // A published scale (the Kp scale, TEC units) carries no unit, and
            // three decimals keep an interpolated value from printing as a
            // full float expansion.
            Quantity::Index => Self::number(None, 1.0, 3),
            Quantity::Ratio => Self::number(Some("%"), Unit::PERCENT.from_base(), 0),
            Quantity::Rate => Self::number(Some("/min"), Unit::PER_MIN.from_base(), 2),
            Quantity::Condition => Self::blank(),
        }
    }

    /// How a channel's sample columns print: the values converted back to the
    /// unit the track declared them in.
    pub(super) fn of_channel_unit(unit: Option<&'a ChannelUnit>) -> Self {
        Self::number(
            unit.map(ChannelUnit::label),
            unit.and_then(ChannelUnit::as_recognized)
                .map_or(1.0, Unit::from_base),
            CHANNEL_DECIMALS,
        )
    }

    /// One cell's text: the value in the header's unit, or the em dash where
    /// the run has no value for it.
    pub(super) fn cell_text(self, value: Option<f64>) -> String {
        let Some(value) = value else {
            return EM_DASH.to_owned();
        };
        match self.kind {
            ColumnKind::TimeOfDay { millis } => time_of_day(value, millis),
            ColumnKind::Number => {
                let scaled = value * self.from_base;
                let decimals = self.decimals;
                format!("{scaled:.decimals$}")
            }
            ColumnKind::Blank => EM_DASH.to_owned(),
        }
    }

    /// The widest text a cell of this column prints, for sizing the column
    /// once instead of measuring every row.
    fn widest_cell_text(self) -> String {
        match self.kind {
            ColumnKind::TimeOfDay { millis } => time_of_day(0.0, millis),
            ColumnKind::Number => {
                let mut widest = String::from("-");
                widest.extend(std::iter::repeat_n('0', BUDGETED_INTEGER_DIGITS));
                if self.decimals > 0 {
                    widest.push('.');
                    widest.extend(std::iter::repeat_n('0', self.decimals));
                }
                widest
            }
            ColumnKind::Blank => EM_DASH.to_owned(),
        }
    }

    /// How wide this column has to be for its header and any value it prints.
    pub(super) fn column_width(self, ui: &egui::Ui, name: &str) -> f32 {
        let cells = text_width(ui, &self.widest_cell_text(), &TextStyle::Monospace);
        let name = text_width(ui, name, &TextStyle::Body);
        let unit = self
            .unit
            .map_or(0.0, |unit| text_width(ui, unit, &TextStyle::Small));
        cells.max(name).max(unit)
    }

    /// The layout a cell and its header read in: numbers align on their right
    /// edge, times on their left.
    fn cell_layout(self) -> Layout {
        match self.kind {
            ColumnKind::Number => Layout::right_to_left(Align::Center),
            ColumnKind::TimeOfDay { .. } | ColumnKind::Blank => {
                Layout::left_to_right(Align::Center)
            }
        }
    }

    /// Whether this column's cells hold magnitudes to compare across the run,
    /// which the bar behind a cell states. A time of day names an instant, and
    /// a blank column holds nothing.
    pub(super) fn holds_magnitudes(self) -> bool {
        match self.kind {
            ColumnKind::Number => true,
            ColumnKind::TimeOfDay { .. } | ColumnKind::Blank => false,
        }
    }

    /// One value cell, aligned the way its column reads. The digits of a
    /// column line up on their decimal point: the text is monospace. The value
    /// reads over its bar, which is painted first.
    pub(super) fn value_ui(self, ui: &mut egui::Ui, value: Option<f64>, bar: Option<ValueBar>) {
        if let Some(bar) = bar {
            bar.paint(ui);
        }
        ui.with_layout(self.cell_layout(), |ui| {
            ui.add(
                Label::new(RichText::new(self.cell_text(value)).monospace())
                    .wrap_mode(TextWrapMode::Extend),
            );
        });
    }

    /// This column's name for a copied table, carrying the unit its values are
    /// in: the copy has no second header line to name it on.
    pub(super) fn header_with_unit(self, name: &str) -> String {
        match self.unit {
            Some(unit) => format!("{name} ({unit})"),
            None => name.to_owned(),
        }
    }

    /// The column header: `name` over the unit its cells are in. A `doc`
    /// underlines the name and explains the metric on hover, the way the
    /// editor explains it under the pointer.
    pub(super) fn header_ui(self, ui: &mut egui::Ui, name: &str, doc: Option<&'static Construct>) {
        let align = match self.kind {
            ColumnKind::Number => Align::Max,
            ColumnKind::TimeOfDay { .. } | ColumnKind::Blank => Align::Min,
        };
        // The header text extends to its full width, setting the width of the
        // column under it.
        ui.with_layout(Layout::top_down(align), |ui| {
            let name = RichText::new(name).strong();
            match doc {
                Some(construct) => {
                    ui.add(
                        Label::new(name.underline())
                            .wrap_mode(TextWrapMode::Extend)
                            .sense(Sense::hover()),
                    )
                    .on_hover_cursor(CursorIcon::Help)
                    .on_hover_ui(|ui| super::construct_tooltip_ui(ui, construct));
                }
                None => {
                    ui.add(Label::new(name).wrap_mode(TextWrapMode::Extend));
                }
            }
            if let Some(unit) = self.unit {
                ui.add(
                    Label::new(RichText::new(unit).weak().small()).wrap_mode(TextWrapMode::Extend),
                );
            }
        });
    }
}

/// The height [`ColumnFormat::header_ui`] lays out to: the name over the unit,
/// with the padding a value row adds around its text.
pub(super) fn header_height(ui: &egui::Ui) -> f32 {
    ui.text_style_height(&TextStyle::Body)
        + ui.spacing().item_spacing.y
        + ui.text_style_height(&TextStyle::Small)
        + super::results::ROW_PADDING
}

/// The width one line of `text` lays out to in `style`.
pub(super) fn text_width(ui: &egui::Ui, text: &str, style: &TextStyle) -> f32 {
    let font = style.resolve(ui.style());
    ui.painter()
        .layout_no_wrap(text.to_owned(), font, egui::Color32::PLACEHOLDER)
        .size()
        .x
}

/// Seconds since the Unix epoch as a wall-clock time of day.
fn time_of_day(seconds: f64, millis: bool) -> String {
    let pattern = if millis { "%H:%M:%S%.3f" } else { "%H:%M:%S" };
    wall_clock(seconds).map_or_else(|| EM_DASH.to_owned(), |t| t.format(pattern).to_string())
}

/// The instant `unix_secs` names, absent for a timestamp outside the
/// representable range. Shared with the name rows, which state their span in
/// the same wall clock the cells below them read in.
pub(super) fn wall_clock(unix_secs: f64) -> Option<DateTime<Utc>> {
    DateTime::<Utc>::from_timestamp_micros((unix_secs * MICROS_PER_SEC) as i64)
}

#[cfg(test)]
mod tests {
    use rstest::rstest;

    use super::*;

    /// Every quantity a metric can carry, printed from a base-unit value:
    /// the cell holds the bare number, the header holds the unit.
    #[rstest]
    #[case::speed(QueryMetric::Velocity, 10.0, "36.0", Some("km/h"))]
    #[case::direction(QueryMetric::Heading, 271.53, "271.5", Some("°"))]
    #[case::length(QueryMetric::Eph, 2.44, "2.4", Some("m"))]
    #[case::acceleration(QueryMetric::Accel, 1.234, "1.23", Some("m/s²"))]
    #[case::duration(QueryMetric::ClockDelta, 0.012_5, "0.013", Some("s"))]
    #[case::count(QueryMetric::SatsFix, 7.0, "7", None)]
    #[case::ratio(QueryMetric::UtilGps, 0.5, "50", Some("%"))]
    #[case::rate(QueryMetric::SlipAll, 2.0, "2.00", Some("/min"))]
    #[case::index(QueryMetric::Tec, 112.483_333_333_333_33, "112.483", None)]
    #[case::index_whole(QueryMetric::Kp, 5.0, "5.000", None)]
    fn a_metric_column_prints_its_value_without_the_unit(
        #[case] metric: QueryMetric,
        #[case] base_value: f64,
        #[case] cell: &str,
        #[case] unit: Option<&str>,
    ) {
        let format = ColumnFormat::of_metric(metric);
        assert_eq!(format.cell_text(Some(base_value)), cell);
        assert_eq!(format.unit, unit);
    }

    #[test]
    fn a_time_column_prints_the_time_of_day() {
        let format = ColumnFormat::of_metric(QueryMetric::Time);
        assert_eq!(format.cell_text(Some(45_296.0)), "12:34:56");
        assert_eq!(format.unit, None);
    }

    #[test]
    fn a_value_the_run_does_not_have_prints_as_the_em_dash() {
        assert_eq!(
            ColumnFormat::of_metric(QueryMetric::Eph).cell_text(None),
            EM_DASH
        );
        assert_eq!(
            ColumnFormat::of_metric(QueryMetric::Time).cell_text(None),
            EM_DASH
        );
    }

    #[test]
    fn a_channel_column_converts_back_to_the_declared_unit() {
        let milligravity = Unit::from_label("mg").expect("mg is recognized");
        let unit = ChannelUnit::recognized(milligravity);
        let format = ColumnFormat::of_channel_unit(Some(&unit));
        assert_eq!(
            format.cell_text(Some(80.0 * milligravity.to_base())),
            "80.000"
        );
        assert_eq!(format.unit, Some("mg"));
    }

    /// A unit the SDK does not recognize has no conversion, so the samples
    /// print as the recorder wrote them.
    #[test]
    fn a_custom_channel_unit_prints_its_values_unscaled() {
        let unit = ChannelUnit::custom("rpm").expect("valid custom unit");
        let format = ColumnFormat::of_channel_unit(Some(&unit));
        assert_eq!(format.cell_text(Some(1500.0)), "1500.000");
        assert_eq!(format.unit, Some("rpm"));
    }
}
