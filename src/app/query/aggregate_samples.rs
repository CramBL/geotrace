//! The channel samples behind a match's aggregate columns: what the picked
//! match lists under itself, and the control that opens that listing.

use egui::{Button, RichText, Sense, TextStyle};
use egui_extras::{Column, TableBuilder};
use egui_phosphor::regular::CARET_DOWN as ICON_CARET_DOWN;
use egui_phosphor::regular::CARET_RIGHT as ICON_CARET_RIGHT;
use geotrace_sdk_units::ChannelUnit;
use gt_query::ChannelTimeline;
use gt_ui_theme::labels::CountLine;

use super::column_format::{self, ColumnFormat};
use super::match_row::MatchKey;
use super::results::ROW_PADDING;

/// Samples a channel's table shows at most before it scrolls. The listing sits
/// between the picked match's caption and its rows, and shows fewer in a tab
/// with less room for it.
pub(crate) const VISIBLE_SAMPLE_ROWS: usize = 6;

/// The label of the control opening the listing.
pub(crate) const TOGGLE_LABEL: &str = "Samples";

const TIME_COLUMN_NAME: &str = "time";

/// One channel's samples under a match: the samples every aggregate column
/// naming that channel reduced, and how they read.
#[derive(Debug)]
pub(super) struct ReducedChannel {
    /// The channel as the query names it, without the leading `@`.
    name: String,
    /// One header per value column: a vector channel's component labels, or the
    /// channel's own name for a scalar channel.
    column_names: Vec<String>,
    /// The unit the track declared its samples in, which the value columns
    /// print in.
    unit: Option<ChannelUnit>,
    /// The samples reduced, in timestamp order and in the evaluator's base
    /// units.
    samples: ChannelTimeline,
}

impl ReducedChannel {
    /// A scalar channel has no component labels: its one value column reads
    /// under the channel's own name.
    pub(super) fn new(
        name: &str,
        components: &[String],
        unit: Option<ChannelUnit>,
        samples: ChannelTimeline,
    ) -> Self {
        let column_names = if components.is_empty() {
            vec![name.to_owned()]
        } else {
            components.to_vec()
        };
        Self {
            name: name.to_owned(),
            column_names,
            unit,
            samples,
        }
    }

    fn sample_count(&self) -> usize {
        self.samples.times.len()
    }

    /// What this channel's name line and the table under it lay out to with
    /// `rows` of its samples on display.
    fn height(&self, ui: &egui::Ui, rows: usize) -> f32 {
        let spacing = ui.spacing().item_spacing.y;
        ui.text_style_height(&TextStyle::Body)
            + spacing
            + column_format::header_height(ui)
            + spacing
            + SampleRowHeights::of(ui).listing(self.sample_count().min(rows))
    }

    /// The channel's name and sample count, then a row per sample: when it was
    /// recorded, and what each of its components read. The table scrolls past
    /// `rows_shown`.
    fn ui(&self, ui: &mut egui::Ui, table_index: usize, rows_shown: usize) {
        let count = self.sample_count();
        ui.label(
            CountLine::new(ui)
                .words(&format!("@{}", self.name))
                .dot()
                .count(count, gt_fmt::pluralize(count, "sample", "samples"))
                .into_job(),
        );
        let time = ColumnFormat::time_of_day_with_millis();
        let value = ColumnFormat::of_channel_unit(self.unit.as_ref());
        let heights = SampleRowHeights::of(ui);
        let header_height = column_format::header_height(ui);
        let time_width = time.column_width(ui, TIME_COLUMN_NAME);
        let value_widths: Vec<f32> = self
            .column_names
            .iter()
            .map(|name| value.column_width(ui, name))
            .collect();
        let mut table = TableBuilder::new(ui)
            .id_salt(("query_aggregate_samples", table_index))
            .striped(true)
            .sense(Sense::hover())
            .auto_shrink([false, true])
            .min_scrolled_height(0.0)
            .max_scroll_height(heights.listing(rows_shown))
            .column(Column::exact(time_width));
        for width in &value_widths {
            table = table.column(Column::exact(*width));
        }
        // The trailing column holds no value: it stretches the striping across
        // the full width of the table.
        table = table.column(Column::remainder());

        table
            .header(header_height, |mut header| {
                header.col(|ui| time.header_ui(ui, TIME_COLUMN_NAME, None));
                for name in &self.column_names {
                    header.col(|ui| value.header_ui(ui, name, None));
                }
                header.col(|_| {});
            })
            .body(|body| {
                body.rows(heights.row, count, |mut row| {
                    let sample = row.index();
                    row.col(|ui| {
                        time.value_ui(ui, self.samples.times.get(sample).copied(), None);
                    });
                    for component in 0..self.column_names.len() {
                        row.col(|ui| {
                            value.value_ui(ui, self.samples.value(sample, component), None);
                        });
                    }
                    row.col(|_| {});
                });
            });
    }
}

/// Which match's samples an [`AggregateSampleExpansion`] holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct GatheredMatch {
    /// The run the match belongs to, as the session numbered it. A rerun
    /// gathers again: its matches read data that may have changed.
    run: u64,
    match_key: MatchKey,
}

/// Whether the picked match lists the samples its aggregate columns reduced,
/// and the samples themselves. They are gathered once per match, and only while
/// the listing is open.
#[derive(Debug, Default)]
pub(super) struct AggregateSampleExpansion {
    open: bool,
    /// The match `channels` was gathered from, `None` while nothing is
    /// gathered.
    gathered: Option<GatheredMatch>,
    /// One entry per channel the match's aggregate columns name, in table
    /// order.
    channels: Vec<ReducedChannel>,
}

impl AggregateSampleExpansion {
    /// The samples under the match `match_key` of `run`, gathered unless they
    /// are the ones already held. A closed listing holds none.
    pub(super) fn refresh(
        &mut self,
        run: u64,
        match_key: MatchKey,
        gather: impl FnOnce() -> Vec<ReducedChannel>,
    ) {
        if !self.open {
            self.gathered = None;
            self.channels = Vec::new();
            return;
        }
        let wanted = GatheredMatch { run, match_key };
        if self.gathered == Some(wanted) {
            return;
        }
        self.gathered = Some(wanted);
        self.channels = gather();
    }

    /// What the listing lays out to with `rows` of every channel's samples on
    /// display.
    fn height(&self, ui: &egui::Ui, rows: usize) -> f32 {
        self.channels
            .iter()
            .map(|channel| channel.height(ui, rows))
            .sum()
    }

    /// What the listing takes with every table showing as many samples as it
    /// ever does, which the tab reserves for it above the picked match's rows.
    pub(super) fn wanted_height(&self, ui: &egui::Ui) -> f32 {
        self.height(ui, VISIBLE_SAMPLE_ROWS)
    }

    /// Sample rows every table shows in `room`: as many as fit it, and never
    /// more than [`VISIBLE_SAMPLE_ROWS`]. The rest of the samples are reached
    /// by scrolling.
    fn rows_shown(&self, ui: &egui::Ui, room: f32) -> usize {
        (0..=VISIBLE_SAMPLE_ROWS)
            .rev()
            .find(|rows| self.height(ui, *rows) <= room)
            .unwrap_or(0)
    }

    /// The control opening the listing, on the line naming the picked match.
    /// Disabled with `disabled_reason` where the match has no samples behind
    /// it.
    pub(super) fn toggle_ui(&mut self, ui: &mut egui::Ui, disabled_reason: Option<&str>) {
        let caret = if self.open {
            ICON_CARET_DOWN
        } else {
            ICON_CARET_RIGHT
        };
        let button = Button::new(RichText::new(format!("{caret} {TOGGLE_LABEL}")).small());
        let response = ui.add_enabled(disabled_reason.is_none(), button);
        let hover = if self.open {
            "Hide the channel samples this match's aggregate columns reduced"
        } else {
            "Show the channel samples this match's aggregate columns reduced"
        };
        match disabled_reason {
            Some(reason) => {
                response.on_disabled_hover_text(reason);
            }
            None => {
                if response.on_hover_text(hover).clicked() {
                    self.open = !self.open;
                }
            }
        }
    }

    /// One table per channel the match's aggregate columns reduced, drawn
    /// within `room`.
    pub(super) fn ui(&self, ui: &mut egui::Ui, room: f32) {
        let rows_shown = self.rows_shown(ui, room);
        for (table_index, channel) in self.channels.iter().enumerate() {
            channel.ui(ui, table_index, rows_shown);
        }
    }
}

/// The heights one channel's listing lays its rows out at.
#[derive(Clone, Copy)]
struct SampleRowHeights {
    /// One row's own height, the same as a row of the match's own table.
    row: f32,
    /// What a row takes on display: rows are separated by the item spacing.
    stride: f32,
}

impl SampleRowHeights {
    fn of(ui: &egui::Ui) -> Self {
        let row = ui.text_style_height(&TextStyle::Monospace) + ROW_PADDING;
        Self {
            row,
            stride: row + ui.spacing().item_spacing.y,
        }
    }

    /// What `rows` rows take on display.
    fn listing(self, rows: usize) -> f32 {
        self.stride * rows as f32
    }
}

#[cfg(test)]
mod tests {
    use rstest::rstest;

    use super::*;

    fn one_channel() -> Vec<ReducedChannel> {
        vec![ReducedChannel::new("accel", &[], None, accel_samples())]
    }

    fn accel_samples() -> ChannelTimeline {
        ChannelTimeline {
            times: vec![0.0, 0.5],
            values: vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0],
            columns: 3,
        }
    }

    #[rstest]
    #[case::scalar(&[], &["accel"])]
    #[case::vector(&["x", "y", "z"], &["x", "y", "z"])]
    fn a_channels_value_columns_read_under_its_components_or_its_own_name(
        #[case] components: &[&str],
        #[case] column_names: &[&str],
    ) {
        let components: Vec<String> = components.iter().map(|c| (*c).to_owned()).collect();

        let channel = ReducedChannel::new("accel", &components, None, accel_samples());

        assert_eq!(channel.column_names, column_names);
        assert_eq!(channel.sample_count(), 2);
    }

    /// Nothing is gathered until the listing is opened: a closed listing holds
    /// no samples. A second frame of the same match reads what is held.
    #[test]
    fn the_samples_are_gathered_once_per_match_while_the_listing_is_open() {
        let mut expansion = AggregateSampleExpansion::default();
        let first = MatchKey::of_first_track(7);
        let second = MatchKey::of_first_track(9);
        let mut gathered = 0;

        expansion.refresh(1, first, || {
            gathered += 1;
            one_channel()
        });
        assert_eq!(gathered, 0);
        assert!(expansion.channels.is_empty());

        expansion.open = true;
        expansion.refresh(1, first, || {
            gathered += 1;
            one_channel()
        });
        expansion.refresh(1, first, || {
            gathered += 1;
            one_channel()
        });
        assert_eq!(gathered, 1);
        assert_eq!(expansion.channels.len(), 1);

        // Another match of the same run, then a rerun of that match.
        expansion.refresh(1, second, || {
            gathered += 1;
            one_channel()
        });
        assert_eq!(gathered, 2);
        expansion.refresh(2, second, || {
            gathered += 1;
            one_channel()
        });
        assert_eq!(gathered, 3);

        expansion.open = false;
        expansion.refresh(2, second, || {
            gathered += 1;
            one_channel()
        });
        assert_eq!(gathered, 3);
        assert!(expansion.channels.is_empty());
    }
}
