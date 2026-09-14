use chrono::{DateTime, Utc};
use egui::{Button, Grid, Label, RichText, TextEdit};
use egui_extras::{Column, TableBuilder, TableRow};
use egui_phosphor::regular::NOTE as ICON_NOTE;
use egui_phosphor::regular::PAPERCLIP as ICON_PAPERCLIP;
use egui_phosphor::regular::TRASH as ICON_TRASH;
use gt_fmt::UTC_MINUTE_FORMAT;
use gt_log_view::LogAttachmentRef;
use gt_pending_writes::WriteAccess;
use gt_side_panel::widgets::{self, MetadataView};
use gt_store::{ChannelSummary, DatabaseRef, NavPointTimeRange, RecordingEntry, TrackState};
use gt_ui_theme::EM_DASH;
use gt_ui_theme::buttons::{self, FramelessIconButton, SortHeaderButton};
use gt_ui_theme::labels;
use strum::{EnumCount as _, IntoEnumIterator as _};

use super::{HistorySort, OpenShelf, RenameEdit, ShelfTracks, SortColumn};
use crate::app::history_db::{DeleteReason, HistoryWorker};
use crate::app::read_only_session::READ_ONLY_RECORDING_HISTORY_HOVER;

/// The scrolling recordings table, laid out like a file manager's list: the
/// metadata columns (date, duration, points, size, logs, actions) take the
/// width of their own header or of the widest cell they draw for the stored
/// recordings (see [`MetadataColumnFloors`]), and the identity column fills
/// whatever width is left, clipping long names. Its width is a function of the window's width, so
/// the table is always exactly as wide as the window.
///
/// Identity is sized as a [`Column::exact`] computed from the floors the other
/// columns take (see [`identity_column_width`]). A [`Column::remainder`] that is
/// not the *last* column ratchets in `egui_extras`: it feeds its clipped width
/// back into its own minimum every frame, so it can never shrink again, which
/// stops the window from being made narrower and lets it creep wider.
pub(super) fn history_table(
    ui: &mut egui::Ui,
    HistoryTable {
        max_listing_height,
        visible,
        entries,
        entries_revision,
        loaded_metas,
        worker,
        rename,
        shelf,
        shelf_raised_the_delete,
        sort,
        write_access,
    }: HistoryTable<'_>,
) {
    request_the_track_table_of_the_open_shelf(worker, shelf);
    let listing = listing_rows(visible, shelf.as_ref());
    let row_height = ui.text_style_height(&egui::TextStyle::Body) + 6.0;
    // The header row and the gap under it come out of the listing's budget
    // before the scrolling body gets what is left.
    let max_scroll_height =
        (max_listing_height - row_height - ui.spacing().item_spacing.y).max(0.0);

    let floors = metadata_column_floors(ui, entries, entries_revision);
    let identity_width = identity_column_width(ui, floors);

    let mut table = TableBuilder::new(ui)
        .id_salt("history_list")
        .striped(true)
        // Cells lay out in a row (no vertical wrapping): dates stay on one
        // line and the action buttons sit side by side.
        .cell_layout(egui::Layout::left_to_right(egui::Align::Center))
        // Don't shrink to content: the table fills the window's width, which the
        // computed identity column already accounts for.
        .auto_shrink([false, true])
        .max_scroll_height(max_scroll_height)
        // `egui_extras` keeps a 200px floor by default, which on a short window
        // pushes the footer off the bottom. The body takes exactly the height
        // it is given.
        .min_scrolled_height(0.0)
        // Identity fills the leftover width (see above) and clips long names.
        .column(Column::exact(identity_width).clip(true));

    // The metadata columns, every sortable column except identity, which was
    // added above, and the action column after them. They are not resizable:
    // there is nothing to gain from resizing a date or a byte count, and it
    // keeps the table's width fully determined by the window. Each takes the
    // floor of the widest cell it can draw (see [`MetadataColumnFloors`]).
    for floor in SortColumn::iter()
        .filter_map(|column| floors.of_sortable_column(column))
        .chain(std::iter::once(floors.action))
    {
        table = table.column(Column::auto().resizable(false).at_least(floor));
    }

    table
        .header(row_height, |mut header| {
            // Driven off `SortColumn`, so a new sortable column cannot be added
            // without a header appearing.
            for column in SortColumn::iter() {
                header.col(|ui| {
                    // Identity is the term-explained column. Every other one is
                    // a plain sortable header.
                    let term = (column == SortColumn::Identity).then_some(crate::terms::IDENTITY);
                    sort_header(ui, column, sort, term);
                });
            }
            header.col(|_| {});
        })
        .body(|body| {
            body.rows(row_height, listing.len(), |mut row| {
                // In-range by construction. `get` guards anyway.
                let Some(listing_row) = listing.get(row.index()) else {
                    return;
                };
                match listing_row {
                    ListingRow::Recording(entry) => {
                        let already_loaded =
                            loaded_metas.iter().any(|m| m.same_recording(&entry.meta));
                        render_row(
                            &mut row,
                            entry,
                            already_loaded,
                            worker,
                            rename,
                            shelf,
                            write_access,
                        );
                    }
                    ListingRow::Shelf(shelf_row) => {
                        let recording = shelf.as_ref().map(|open| &open.recording);
                        render_shelf_row(
                            &mut row,
                            ShelfRowRender {
                                shelf_row,
                                recording,
                                worker,
                                raised_the_delete: shelf_raised_the_delete,
                                write_access,
                            },
                        );
                    }
                }
            });
        });
}

/// The width left for the identity column: what the table has, less the gap
/// between each pair of columns and the floor every other column takes.
///
/// It lands on the pixel grid, always down, never up: half a pixel over sets the
/// listing scrolling sideways, and the scroll area's drag then takes the hover
/// off the rows.
fn identity_column_width(ui: &egui::Ui, floors: MetadataColumnFloors) -> f32 {
    // What `egui_extras` lays the table's columns out in.
    let table_width = ui.available_width() - ui.spacing().scroll.allocated_width();
    let gaps = ui.spacing().item_spacing.x * COLUMN_GAP_COUNT;
    let left_over = (table_width - gaps - floors.total()).max(IDENTITY_MIN_WIDTH);
    let pixels_per_point = ui.pixels_per_point();
    (left_over * pixels_per_point).floor() / pixels_per_point
}

/// The width each metadata column keeps, whatever the listing has scrolled into
/// view: the widest cell that column draws for the recordings the database
/// holds, filtered out of the listing or not.
///
/// An `egui_extras` auto column takes the width of the cells laid out this
/// frame, and the body draws the rows in view alone. Without these floors the
/// columns, and the identity column that fills what they leave, change width as
/// the user scrolls.
///
/// A line of an open shelf draws a subset of the same cells: its track's
/// nav-point count under Points, its two controls under the actions, and
/// nothing under Date, Duration, Size and Logs.
#[derive(Clone, Copy)]
struct MetadataColumnFloors {
    date: f32,
    duration: f32,
    points: f32,
    size: f32,
    logs: f32,
    action: f32,
}

impl MetadataColumnFloors {
    /// A floor covers the column's header as well as its cells, and lands on a
    /// whole pixel: the identity column takes the width these leave, and a
    /// fraction of a pixel over sends the table past the window's edge.
    fn measure(ui: &egui::Ui, entries: &[RecordingEntry]) -> Self {
        let widest = |column: SortColumn, cell_width: fn(&egui::Ui, &RecordingEntry) -> f32| {
            let widest = entries
                .iter()
                .map(|entry| cell_width(ui, entry))
                .fold(buttons::sort_header_width(ui, column.title()), f32::max);
            whole_pixels(ui, widest)
        };
        Self {
            date: widest(SortColumn::Date, |ui, entry| {
                label_width(ui, &started_at_text(entry.meta.time_range))
            }),
            duration: widest(SortColumn::Duration, |ui, entry| {
                label_width(ui, &duration_text(entry.meta.time_range))
            }),
            points: widest(SortColumn::Points, points_cell_width),
            size: widest(SortColumn::Size, |ui, entry| {
                label_width(ui, &gt_fmt::format_bytes(entry.meta.gtd_size_bytes))
            }),
            logs: widest(SortColumn::Logs, |ui, entry| {
                attached_logs_label(entry).map_or(0.0, |label| buttons::button_width(ui, &label))
            }),
            action: whole_pixels(ui, action_column_width(ui)),
        }
    }

    /// What every column but identity takes.
    fn total(self) -> f32 {
        self.date + self.duration + self.points + self.size + self.logs + self.action
    }

    /// The floor of a sortable column, and [`None`] for identity, which takes
    /// the width the other columns leave.
    fn of_sortable_column(self, column: SortColumn) -> Option<f32> {
        match column {
            SortColumn::Identity => None,
            SortColumn::Date => Some(self.date),
            SortColumn::Duration => Some(self.duration),
            SortColumn::Points => Some(self.points),
            SortColumn::Size => Some(self.size),
            SortColumn::Logs => Some(self.logs),
        }
    }
}

/// The floors are kept until the listing they were measured from, or the scale
/// they were measured at, changes. [`MetadataColumnFloors::measure`] lays out a
/// galley per cell of every stored recording.
fn metadata_column_floors(
    ui: &egui::Ui,
    entries: &[RecordingEntry],
    entries_revision: u64,
) -> MetadataColumnFloors {
    let measured_for = FloorsMeasuredFor {
        entries_revision,
        pixels_per_point_bits: ui.pixels_per_point().to_bits(),
    };
    let pass = ui.ctx().cumulative_pass_nr();
    let id = ui.id().with("history_column_floors");
    let cached = ui
        .data(|d| d.get_temp::<MeasuredColumnFloors>(id))
        .filter(|cached| cached.measured_for == measured_for);
    if let Some(cached) = cached
        && (cached.measured_again || cached.first_measured_on_pass == pass)
    {
        return cached.floors;
    }
    let floors = MetadataColumnFloors::measure(ui, entries);
    let measured = MeasuredColumnFloors {
        measured_for,
        floors,
        first_measured_on_pass: cached.map_or(pass, |cached| cached.first_measured_on_pass),
        measured_again: cached.is_some(),
    };
    ui.data_mut(|d| d.insert_temp(id, measured));
    floors
}

/// The floors the cache holds, what they were measured from, and the pass the
/// first measurement of them ran in.
///
/// The floors are measured a second time, in the pass after the first
/// measurement. epaint reports a glyph it lays out for the first time from the
/// font's own advance, and the pixel-snapped width the cell draws with from the
/// next pass on, which are 0.375px apart on the delete icon at the default
/// scale.
#[derive(Clone, Copy)]
struct MeasuredColumnFloors {
    measured_for: FloorsMeasuredFor,
    floors: MetadataColumnFloors,
    first_measured_on_pass: u64,
    measured_again: bool,
}

/// What [`MetadataColumnFloors`] were measured from. A change to either leaves
/// the measured widths stale.
#[derive(Clone, Copy, PartialEq)]
struct FloorsMeasuredFor {
    entries_revision: u64,
    pixels_per_point_bits: u32,
}

/// The width the action column reserves: the widest cell it can hold, which is
/// the shelf's closing line with "Unshelve all" beside the delete icon.
fn action_column_width(ui: &egui::Ui) -> f32 {
    let between_the_two_controls = ui.spacing().item_spacing.x;
    let recording_row = buttons::button_width(ui, OPEN_RECORDING_LABEL)
        + between_the_two_controls
        + buttons::button_width(ui, DELETE_RECORDING_LABEL);
    let shelf_closing_line = buttons::button_width(ui, UNSHELVE_ALL_LABEL)
        + between_the_two_controls
        + FramelessIconButton::new(ICON_TRASH).width(ui);
    recording_row.max(shelf_closing_line)
}

/// The width the Points column needs for `entry`: its own nav-point count with
/// the shelved-track note beside it, or the widest count a shelf line under it
/// states, whichever is wider.
pub(super) fn points_cell_width(ui: &egui::Ui, entry: &RecordingEntry) -> f32 {
    let (count, shelved_note) = points_cell_texts(entry);
    let recording_row = label_width(ui, &count)
        + shelved_note.map_or(0.0, |note| {
            ui.spacing().item_spacing.x + label_width(ui, &note)
        });
    widest_counts_a_shelf_line_states(entry.meta.nav_point_count)
        .map(|count| label_width(ui, &gt_store::format_count_suffix(count)))
        .fold(recording_row, f32::max)
}

/// The counts whose formatted form can be the widest a shelf line under a
/// recording of `nav_points` states.
///
/// A shelved track spans at most its recording's own points, and
/// [`gt_store::format_count_suffix`] does not widen with the count: 999_900
/// gives "999.9k", where the 1_000_000 above it gives "1m". Each band the
/// counts reach contributes the highest count in it and the count a tenth of a
/// step below, which is the one written with a decimal.
fn widest_counts_a_shelf_line_states(nav_points: u64) -> impl Iterator<Item = u64> {
    COUNT_FORM_BANDS
        .into_iter()
        .filter(move |band| nav_points >= band.first)
        .flat_map(move |CountFormBand { first, past_last }| {
            let highest = nav_points.min(past_last.saturating_sub(1));
            [highest, highest.saturating_sub(first / 10)]
        })
}

/// A run of counts [`gt_store::format_count_suffix`] writes in one form.
#[derive(Clone, Copy)]
struct CountFormBand {
    first: u64,
    past_last: u64,
}

fn label_width(ui: &egui::Ui, text: &str) -> f32 {
    labels::text_width(ui, text, egui::TextStyle::Body)
}

/// `width` rounded up to a whole pixel.
fn whole_pixels(ui: &egui::Ui, width: f32) -> f32 {
    let pixels_per_point = ui.pixels_per_point();
    (width * pixels_per_point).ceil() / pixels_per_point
}

/// One line of the History listing: a stored recording, or a line of the shelf
/// open under it.
enum ListingRow<'a> {
    Recording(&'a RecordingEntry),
    Shelf(ShelfRow),
}

/// A line of the shelf open under a recording's row.
enum ShelfRow {
    /// The shelf's closing line, which unshelves every track above it.
    EveryShelvedTrack { stored_rows: Vec<usize> },
    /// The worker's read of the recording's stored track table is in flight.
    Reading,
    /// One shelved track, addressed by its row in that table.
    ShelvedTrack {
        stored_row: usize,
        nav_point_count: u64,
    },
}

/// The listing's lines: every visible recording, each followed by the lines of
/// its shelf while the shelf is open on it.
fn listing_rows<'a>(
    visible: &'a [&'a RecordingEntry],
    shelf: Option<&OpenShelf>,
) -> Vec<ListingRow<'a>> {
    let mut listing = Vec::with_capacity(visible.len());
    for entry in visible {
        listing.push(ListingRow::Recording(entry));
        let Some(open) = shelf.filter(|open| open.recording == entry.db_ref) else {
            continue;
        };
        let ShelfTracks::Read(tracks) = &open.tracks else {
            listing.push(ListingRow::Shelf(ShelfRow::Reading));
            continue;
        };
        let mut stored_rows = Vec::new();
        for (stored_row, track) in tracks.iter().enumerate() {
            if track.state != TrackState::Shelved {
                continue;
            }
            stored_rows.push(stored_row);
            listing.push(ListingRow::Shelf(ShelfRow::ShelvedTrack {
                stored_row,
                nav_point_count: track.end.saturating_sub(track.start),
            }));
        }
        if !stored_rows.is_empty() {
            listing.push(ListingRow::Shelf(ShelfRow::EveryShelvedTrack {
                stored_rows,
            }));
        }
    }
    listing
}

/// Opening the History window on a large database still costs one listing
/// query: the listing reads a track table only for the recording whose shelf is
/// open.
fn request_the_track_table_of_the_open_shelf(
    worker: &HistoryWorker,
    shelf: &mut Option<OpenShelf>,
) {
    let Some(open) = shelf.as_mut() else {
        return;
    };
    if !matches!(open.tracks, ShelfTracks::Unrequested) {
        return;
    }
    worker.load_stored_track_table(open.recording.clone());
    open.tracks = ShelfTracks::Requested;
}

/// The caret that opens a recording's shelf, grayed out for a recording whose
/// tracks are all live.
fn shelf_caret(ui: &mut egui::Ui, entry: &RecordingEntry, shelf: &mut Option<OpenShelf>) {
    let open = shelf
        .as_ref()
        .is_some_and(|open| open.recording == entry.db_ref);
    let has_shelved_tracks = entry.shelved_tracks > 0;
    let hover = match (has_shelved_tracks, open) {
        (false, _) => NO_SHELVED_TRACKS_HOVER,
        (true, false) => SHOW_SHELVED_TRACKS_HOVER,
        (true, true) => HIDE_SHELVED_TRACKS_HOVER,
    };
    let clicked = FramelessIconButton::new(widgets::expand_arrow(open))
        .enabled(has_shelved_tracks)
        .hover_text_ui(ui, hover)
        .clicked();
    if clicked {
        *shelf = (!open).then(|| OpenShelf {
            recording: entry.db_ref.clone(),
            tracks: ShelfTracks::Unrequested,
        });
    }
}

/// What one line of the open shelf draws, and where it reports a press.
struct ShelfRowRender<'a> {
    shelf_row: &'a ShelfRow,
    /// The recording the shelf is open on, and [`None`] on the frame the shelf
    /// closes under the row being drawn.
    recording: Option<&'a DatabaseRef>,
    worker: &'a HistoryWorker,
    /// Set to the shelf's recording when the closing line's delete is pressed.
    /// The History window raises the confirmation over it.
    raised_the_delete: &'a mut Option<DatabaseRef>,
    write_access: WriteAccess,
}

/// One line of an open shelf: the track's number and nav-point count with the
/// button that unshelves it, or the closing line, whose two buttons unshelve
/// every track the shelf lists and delete every one of them permanently.
///
/// A track's number is its stored row plus one. That holds for the life of the
/// recording: a permanent delete leaves a tombstone in its row, and the rows
/// after it keep their position. The shelf lists "#3" when rows 1 and 2 are a
/// live and a deleted track.
fn render_shelf_row(
    row: &mut TableRow<'_, '_>,
    ShelfRowRender {
        shelf_row,
        recording,
        worker,
        raised_the_delete,
        write_access,
    }: ShelfRowRender<'_>,
) {
    row.col(|ui| {
        ui.add_space(ui.spacing().indent);
        match shelf_row {
            ShelfRow::Reading => {
                ui.spinner();
            }
            ShelfRow::ShelvedTrack { stored_row, .. } => {
                ui.label(format!("#{}", stored_row.saturating_add(1)));
            }
            ShelfRow::EveryShelvedTrack { stored_rows } => {
                let count = stored_rows.len();
                ui.weak(format!(
                    "{count} shelved {}",
                    gt_fmt::pluralize(count, "track", "tracks")
                ));
            }
        }
    });
    // Date and Duration: a stored track table states neither.
    row.col(|_| {});
    row.col(|_| {});
    row.col(|ui| {
        if let ShelfRow::ShelvedTrack {
            nav_point_count, ..
        } = shelf_row
        {
            ui.label(gt_store::format_count_suffix(*nav_point_count));
        }
    });
    // Size and Logs: both are per recording.
    row.col(|_| {});
    row.col(|_| {});
    row.col(|ui| {
        let writes_recordings = write_access.allows_writing();
        let (label, hover, rows) = match shelf_row {
            ShelfRow::Reading => return,
            ShelfRow::ShelvedTrack { stored_row, .. } => {
                (UNSHELVE_LABEL, UNSHELVE_HOVER, vec![*stored_row])
            }
            ShelfRow::EveryShelvedTrack { stored_rows } => {
                (UNSHELVE_ALL_LABEL, UNSHELVE_ALL_HOVER, stored_rows.clone())
            }
        };
        let clicked = ui
            .add_enabled(writes_recordings, Button::new(label).small())
            .on_hover_text(hover)
            .on_disabled_hover_text(READ_ONLY_RECORDING_HISTORY_HOVER)
            .clicked();
        if clicked && let Some(recording) = recording {
            worker.set_tracks_shelved(recording.clone(), rows, false);
        }

        let ShelfRow::EveryShelvedTrack { .. } = shelf_row else {
            return;
        };
        let delete = FramelessIconButton::new(
            RichText::new(ICON_TRASH).color(gt_ui_theme::warning_amber(ui.visuals().dark_mode)),
        )
        .enabled(writes_recordings)
        .hover_text_ui(
            ui,
            if writes_recordings {
                DELETE_SHELVED_HOVER
            } else {
                READ_ONLY_RECORDING_HISTORY_HOVER
            },
        );
        if delete.clicked()
            && let Some(recording) = recording
        {
            *raised_the_delete = Some(recording.clone());
        }
    });
}

/// A clickable table header that orders the list by `column`.
///
/// The active column shows a caret pointing the way its values run. Clicking
/// it reverses that, clicking any other column switches to it. `term`, when
/// given, is the column's glossary explanation.
fn sort_header(ui: &mut egui::Ui, column: SortColumn, sort: &mut HistorySort, term: Option<&str>) {
    let active = sort.column == column;
    let next = if active {
        sort.direction.reversed()
    } else {
        column.initial_direction()
    };

    let mut header = SortHeaderButton::new(column.title());
    if active {
        header = header.active_direction_caret(sort.direction.caret());
    }
    if let Some(term) = term {
        header = header.term_explanation(term);
    }

    let clicked = header
        .show(
            ui,
            egui::Layout::left_to_right(egui::Align::Center),
            column.order_hint(next),
        )
        .clicked();

    if clicked {
        sort.clicked(column);
    }
}

/// What one render of the recordings table draws, and what it edits while the
/// user works in it.
pub(super) struct HistoryTable<'a> {
    /// Height the whole listing may take, header row included. What is left
    /// after the header bounds the scrolling body.
    pub max_listing_height: f32,
    /// The rows the filters left, in the order the sort put them.
    pub visible: &'a [&'a RecordingEntry],
    /// Every stored recording, whether the filters left it in `visible` or not.
    /// Filtering moves no column: the floors are measured from all of them.
    pub entries: &'a [RecordingEntry],
    /// Bumped by [`super::HistoryWindow::set_entries`], which is what makes the
    /// measured column widths stale.
    pub entries_revision: u64,
    /// The recordings already in the window, whose rows cannot be opened
    /// again.
    pub loaded_metas: &'a [gt_store::RecordingMeta],
    pub worker: &'a HistoryWorker,
    pub rename: &'a mut Option<RenameEdit>,
    /// The recording whose shelved tracks the listing shows under its row. The
    /// caret in a row's identity cell opens and closes it.
    pub shelf: &'a mut Option<OpenShelf>,
    /// Set to the shelf's recording when the closing line's delete is pressed.
    pub shelf_raised_the_delete: &'a mut Option<DatabaseRef>,
    pub sort: &'a mut HistorySort,
    pub write_access: WriteAccess,
}

fn render_row(
    row: &mut TableRow<'_, '_>,
    entry: &RecordingEntry,
    already_loaded: bool,
    worker: &HistoryWorker,
    rename: &mut Option<RenameEdit>,
    shelf: &mut Option<OpenShelf>,
    write_access: WriteAccess,
) {
    // Identity column: the shelf caret, then the inline editor when this row is
    // being renamed and the normal cell otherwise.
    row.col(|ui| {
        shelf_caret(ui, entry, shelf);
        if rename
            .as_ref()
            .is_some_and(|r| r.identity == entry.db_ref.identity)
        {
            render_rename_editor(ui, rename, worker);
        } else {
            identity_cell(ui, entry, worker, rename, write_access);
        }
    });

    breakdown_cell(row, entry, SortColumn::Date, |ui| {
        ui.label(started_at_text(entry.meta.time_range));
    });

    breakdown_cell(row, entry, SortColumn::Duration, |ui| {
        ui.label(duration_text(entry.meta.time_range));
    });

    breakdown_cell(row, entry, SortColumn::Points, |ui| {
        let (count, shelved_note) = points_cell_texts(entry);
        ui.label(count);
        if let Some(note) = shelved_note {
            ui.weak(note);
        }
    });

    breakdown_cell(row, entry, SortColumn::Size, |ui| {
        ui.label(gt_fmt::format_bytes(entry.meta.gtd_size_bytes));
    });

    row.col(|ui| {
        attached_logs_cell(ui, entry, worker);
    });

    row.col(|ui| {
        let open = ui.add_enabled(!already_loaded, Button::new(OPEN_RECORDING_LABEL).small());
        if already_loaded {
            open.on_hover_text("Already loaded");
        } else if open.clicked() {
            worker.open(entry.db_ref.clone());
        }
        if ui
            .add_enabled(
                write_access.allows_writing(),
                Button::new(DELETE_RECORDING_LABEL).small(),
            )
            .on_hover_text("Permanently delete this recording from history")
            .on_disabled_hover_text(READ_ONLY_RECORDING_HISTORY_HOVER)
            .clicked()
        {
            worker.delete_recordings(vec![entry.db_ref.clone()], DeleteReason::Manual);
        }
    });
}

/// The Logs column of a History row: how many logs the recording stores, and
/// the menu listing them by name with an action that loads one. A recording
/// storing no log shows an empty cell.
fn attached_logs_cell(ui: &mut egui::Ui, entry: &RecordingEntry, worker: &HistoryWorker) {
    let Some(label) = attached_logs_label(entry) else {
        return;
    };
    ui.menu_button(label, |ui| {
        for listed in &entry.log_attachments {
            ui.horizontal(|ui| {
                let name = listed.attachment.name.as_str();
                ui.add(Label::new(name).truncate());
                if ui
                    .button(OPEN_LOG_LABEL)
                    .on_hover_text(OPEN_LOG_HOVER)
                    .clicked()
                {
                    worker.load_attached_log(
                        LogAttachmentRef {
                            recording: entry.db_ref.clone(),
                            id: listed.id,
                        },
                        name.to_owned(),
                    );
                    ui.close();
                }
            });
        }
    })
    .response
    .on_hover_text(ATTACHED_LOGS_HOVER);
}

/// What the Points cell of a recording row states: the recording's nav-point
/// count, and the note of how many of its tracks are shelved for a recording
/// that has one.
fn points_cell_texts(entry: &RecordingEntry) -> (String, Option<String>) {
    let count = gt_store::format_count_suffix(entry.meta.nav_point_count);
    let shelved_note = (entry.shelved_tracks > 0)
        .then(|| format!("({}/{} shelved)", entry.shelved_tracks, entry.total_tracks));
    (count, shelved_note)
}

/// The Logs cell's label, and [`None`] for a recording storing no log.
fn attached_logs_label(entry: &RecordingEntry) -> Option<String> {
    let count = entry.log_attachments.len();
    (count > 0).then(|| format!("{ICON_PAPERCLIP} {count}"))
}

/// When the recording started, an em dash for one with no time range.
pub(super) fn started_at_text(time_range: Option<NavPointTimeRange>) -> String {
    time_range.map_or_else(
        || EM_DASH.to_owned(),
        |range| {
            DateTime::<Utc>::from_timestamp_micros(range.start_us())
                .unwrap_or_default()
                .format(UTC_MINUTE_FORMAT)
                .to_string()
        },
    )
}

/// How long the recording ran, an em dash for one with no time range.
pub(super) fn duration_text(time_range: Option<NavPointTimeRange>) -> String {
    time_range.map_or_else(
        || EM_DASH.to_owned(),
        |range| super::format_duration(chrono::Duration::microseconds(range.duration_us())),
    )
}

/// Both ends of the recording's time range, an em dash for one without.
pub(super) fn time_range_text(time_range: Option<NavPointTimeRange>) -> String {
    time_range.map_or_else(
        || EM_DASH.to_owned(),
        |range| {
            let start =
                DateTime::<Utc>::from_timestamp_micros(range.start_us()).unwrap_or_default();
            let end = DateTime::<Utc>::from_timestamp_micros(range.end_us()).unwrap_or_default();
            gt_fmt::format_time_range(start, end)
        },
    )
}

/// Render one of a row's value cells and give the whole cell - the text and the
/// blank space beside it - the recording's data breakdown as hover text.
///
/// Nothing inside a value cell senses hover or click of its own, so covering
/// the cell's whole rect is what makes the breakdown reachable.
fn breakdown_cell(
    row: &mut TableRow<'_, '_>,
    entry: &RecordingEntry,
    column: SortColumn,
    content: impl FnOnce(&mut egui::Ui),
) {
    row.col(|ui| {
        let cell = ui.max_rect();
        content(ui);
        ui.interact(cell, breakdown_cell_id(entry, column), egui::Sense::hover())
            .on_hover_ui(|ui| data_breakdown_ui(ui, entry));
    });
}

/// The widget id of a row's breakdown cell.
///
/// The recording's database reference identifies the row and `column` separates
/// the cells within it, so no two breakdown cells in the table share an id and
/// none collides with a neighbour's interaction state.
pub(super) fn breakdown_cell_id(entry: &RecordingEntry, column: SortColumn) -> egui::Id {
    egui::Id::new((
        "history_row_breakdown",
        entry.db_ref.identity.as_str(),
        entry.db_ref.group_name.as_str(),
        column,
    ))
}

/// What the recording holds, as hover detail for a History row: its exact span,
/// its shape on disk, and a count per kind of data - including the ad-hoc sensor
/// channels, which no table column reveals.
pub(super) fn data_breakdown_ui(ui: &mut egui::Ui, entry: &RecordingEntry) {
    let meta = &entry.meta;
    ui.label(time_range_text(meta.time_range));

    Grid::new("history_breakdown_counts")
        .num_columns(2)
        .spacing([12.0, 4.0])
        .show(ui, |ui| {
            let mut row = |label: &str, value: String| {
                ui.label(RichText::new(label).weak());
                ui.label(value);
                ui.end_row();
            };
            row("Duration", duration_text(meta.time_range));
            row("Size", gt_fmt::format_bytes(meta.gtd_size_bytes));
            row("Tracks", track_count_text(entry));
            row("Nav points", format_stored_count(meta.nav_point_count));
            row(
                "Satellite reports",
                format_stored_count(meta.sat_report_count),
            );
            row("Markers", format_stored_count(meta.marker_count));
            row(
                "Event markers",
                format_stored_count(meta.event_marker_count),
            );
        });

    if entry.shelved_tracks > 0 {
        ui.label(
            RichText::new(
                "Shelved tracks came from 'shelve filtered data'. \
                 Use 'Delete shelved data' to drop them permanently.",
            )
            .small()
            .color(ui.visuals().weak_text_color()),
        );
    }

    channels_breakdown_ui(ui, &entry.channels);
}

/// The recording's ad-hoc sensor channels, one row each: name (with a vector
/// channel's component labels), unit, and sample count. Long channel lists are
/// truncated so the hover cannot outgrow the screen.
///
/// A recording with no channels renders an explicit none row.
fn channels_breakdown_ui(ui: &mut egui::Ui, channels: &[ChannelSummary]) {
    ui.add_space(4.0);
    if channels.is_empty() {
        ui.label(RichText::new("No custom channels").color(ui.visuals().weak_text_color()));
        return;
    }

    let count = channels.len();
    ui.label(
        RichText::new(format!(
            "{count} custom {}",
            gt_fmt::pluralize(count, "channel", "channels")
        ))
        .strong(),
    );
    Grid::new("history_breakdown_channels")
        .num_columns(3)
        .spacing([12.0, 4.0])
        .show(ui, |ui| {
            for channel in channels.iter().take(MAX_HOVER_CHANNELS) {
                ui.vertical(|ui| {
                    // A vertical inside a grid cell has no width of its own to
                    // wrap against, so let the labels size the column instead.
                    ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Extend);
                    ui.label(channel_title(channel));
                    // The description goes under the name so the columns stay
                    // aligned.
                    if let Some(description) = &channel.description {
                        ui.label(
                            RichText::new(description)
                                .small()
                                .color(ui.visuals().weak_text_color()),
                        );
                    }
                });
                ui.label(
                    RichText::new(channel.unit.as_deref().unwrap_or(EM_DASH))
                        .color(ui.visuals().weak_text_color()),
                );
                ui.label(
                    RichText::new(format!(
                        "{} {}",
                        format_stored_count(channel.sample_count),
                        gt_fmt::pluralize(
                            usize::try_from(channel.sample_count).unwrap_or(usize::MAX),
                            "sample",
                            "samples",
                        )
                    ))
                    .color(ui.visuals().weak_text_color()),
                );
                ui.end_row();
            }
        });
    if let Some(hidden) = count.checked_sub(MAX_HOVER_CHANNELS).filter(|n| *n > 0) {
        ui.label(RichText::new(format!("and {hidden} more")).color(ui.visuals().weak_text_color()));
    }
}

/// A channel's name, with a vector channel's component labels appended:
/// `accel (x, y, z)`. A scalar channel is just its name.
pub(super) fn channel_title(channel: &ChannelSummary) -> String {
    if channel.components.is_empty() {
        return channel.name.clone();
    }
    format!("{} ({})", channel.name, channel.components.join(", "))
}

/// The recording's track count, noting how many of them are shelved.
pub(super) fn track_count_text(entry: &RecordingEntry) -> String {
    if entry.shelved_tracks > 0 {
        format!("{} ({} shelved)", entry.total_tracks, entry.shelved_tracks)
    } else {
        entry.total_tracks.to_string()
    }
}

/// Thousands-separated form of one of the database's `u64` counters.
fn format_stored_count(n: u64) -> String {
    gt_fmt::format_count(usize::try_from(n).unwrap_or(usize::MAX))
}

/// Render the inline identity-rename editor in the identity column. Commits on
/// Enter, cancels on focus loss (click-away or Escape). Either way the editor
/// closes. A no-op commit (empty, or unchanged from the displayed name) does not
/// send a rename. `rename` is guaranteed `Some` by the caller.
fn render_rename_editor(
    ui: &mut egui::Ui,
    rename: &mut Option<RenameEdit>,
    worker: &HistoryWorker,
) {
    let Some(edit) = rename.as_mut() else {
        return;
    };
    let enter = ui.input(|i| i.key_pressed(egui::Key::Enter));
    let resp = ui.add(
        TextEdit::singleline(&mut edit.buffer)
            .desired_width(f32::INFINITY)
            .hint_text("Identity"),
    );
    if resp.lost_focus() {
        let old = std::mem::take(&mut edit.identity);
        let new = edit.buffer.trim().to_owned();
        let unchanged = new == super::identity_display_parts(&old).0;
        *rename = None;
        if enter && !new.is_empty() && !unchanged {
            worker.rename_identity(old, new);
        }
    } else {
        // Keep focus in the freshly-opened editor until the user commits or
        // clicks away.
        resp.request_focus();
    }
}

/// Open the inline rename editor for a recording's identity.
fn begin_rename(rename: &mut Option<RenameEdit>, entry: &RecordingEntry) {
    let identity = entry.db_ref.identity.clone();
    let buffer = super::identity_display_parts(&identity).0.to_owned();
    *rename = Some(RenameEdit { identity, buffer });
}

/// The identity column of a History row. Double-clicking the cell opens the
/// inline rename editor. Right-clicking offers Rename and Delete.
fn identity_cell(
    ui: &mut egui::Ui,
    entry: &RecordingEntry,
    worker: &HistoryWorker,
    rename: &mut Option<RenameEdit>,
    write_access: WriteAccess,
) {
    let writes_recordings = write_access.allows_writing();
    let identity = entry.db_ref.identity.as_str();
    let (display_name, is_auto) = super::identity_display_parts(identity);
    // The full identity is the hover's first line, so leave it out of the view:
    // the note icon and rows are for the SDK's title/device/notes only. Every
    // recording has an identity, so including it would badge every row.
    let travel_mode = entry.travel_mode.as_deref().map(super::travel_mode_display);
    let meta = MetadataView {
        title: entry.title.as_deref(),
        device: entry.device.as_deref(),
        travel_mode: travel_mode.as_deref(),
        identity: None,
        notes: entry.notes.as_deref(),
    };
    let has_metadata = widgets::has_metadata_details(&meta);
    let label = ui
        .horizontal(|ui| {
            if is_auto {
                ui.label(
                    RichText::new("auto")
                        .small()
                        .color(ui.visuals().weak_text_color()),
                );
            }
            if has_metadata {
                ui.label(RichText::new(ICON_NOTE).weak());
            }
            // The label itself senses clicks: it is the rename target. Its
            // elided-text tooltip is off: the cell's hover already leads with
            // the full identity.
            ui.add(
                Label::new(display_name)
                    .truncate()
                    .show_tooltip_when_elided(false)
                    .sense(egui::Sense::click()),
            )
        })
        .inner
        // Right-click always offers the menu, and a session that may write
        // also renames on a double-click, so the cell reads as interactive.
        .on_hover_cursor(if writes_recordings {
            egui::CursorIcon::PointingHand
        } else {
            egui::CursorIcon::Default
        })
        .on_hover_ui(|ui| {
            ui.label(identity);
            widgets::metadata_detail_rows(ui, &meta);
            // The same breakdown hover as the value cells.
            ui.separator();
            data_breakdown_ui(ui, entry);
            ui.separator();
            ui.label(
                RichText::new(if writes_recordings {
                    "Double-click to rename"
                } else {
                    READ_ONLY_RECORDING_HISTORY_HOVER
                })
                .small()
                .color(ui.visuals().weak_text_color()),
            );
        });
    if label.double_clicked() && writes_recordings {
        begin_rename(rename, entry);
    }
    label.context_menu(|ui| {
        if ui
            .add_enabled(writes_recordings, Button::new("Rename"))
            .on_disabled_hover_text(READ_ONLY_RECORDING_HISTORY_HOVER)
            .clicked()
        {
            begin_rename(rename, entry);
            ui.close();
        }
        if ui
            .add_enabled(writes_recordings, Button::new("Delete"))
            .on_disabled_hover_text(READ_ONLY_RECORDING_HISTORY_HOVER)
            .clicked()
        {
            worker.delete_recordings(vec![entry.db_ref.clone()], DeleteReason::Manual);
            ui.close();
        }
    });
}

/// Gaps `egui_extras` leaves between the table's columns, one fewer than the
/// columns themselves: one per sortable column, and the actions after them.
const COLUMN_GAP_COUNT: f32 = SortColumn::COUNT as f32;

/// The three forms [`gt_store::format_count_suffix`] writes: plain digits under
/// a thousand, thousands under a million, millions above it.
const COUNT_FORM_BANDS: [CountFormBand; 3] = [
    CountFormBand {
        first: 0,
        past_last: 1_000,
    },
    CountFormBand {
        first: 1_000,
        past_last: 1_000_000,
    },
    CountFormBand {
        first: 1_000_000,
        past_last: u64::MAX,
    },
];

pub(super) const OPEN_RECORDING_LABEL: &str = "Open";

const DELETE_RECORDING_LABEL: &str = "Delete";

pub(super) const UNSHELVE_LABEL: &str = "Unshelve";

pub(super) const UNSHELVE_ALL_LABEL: &str = "Unshelve all";

const UNSHELVE_HOVER: &str = "Put this track back in the recording";

const UNSHELVE_ALL_HOVER: &str = "Put every shelved track back in the recording";

const DELETE_SHELVED_HOVER: &str = "Permanently delete every shelved track of this recording";

const SHOW_SHELVED_TRACKS_HOVER: &str = "List the shelved tracks of this recording";

const HIDE_SHELVED_TRACKS_HOVER: &str = "Close the list of shelved tracks";

const NO_SHELVED_TRACKS_HOVER: &str = "This recording has no shelved tracks";

/// Identity's floor: room for the shelf caret and the first characters of the
/// name. A floor high enough to keep the name readable leaves the action column
/// past the window's right edge, where the horizontal scroll area cuts Open and
/// Delete off. The metadata columns, the action column among them, take their
/// content width first. Identity reaches this floor in a window too narrow for
/// them, and the listing then scrolls sideways.
const IDENTITY_MIN_WIDTH: f32 = 64.0;

pub(in crate::app) const OPEN_LOG_LABEL: &str = "Open log";

const OPEN_LOG_HOVER: &str = "Load this log into the log viewer";

const ATTACHED_LOGS_HOVER: &str = "The logs stored with this recording";

/// How many channels the hover lists before summarizing the rest, so a
/// recording carrying dozens of them still produces a readable tooltip.
pub(super) const MAX_HOVER_CHANNELS: usize = 8;
