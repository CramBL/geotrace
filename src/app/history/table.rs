use std::cell::Cell;

use chrono::{DateTime, Utc};
use egui::{Button, Grid, Label, RichText, TextEdit};
use egui_extras::{Column, TableBuilder, TableRow};
use egui_phosphor::regular::NOTE as ICON_NOTE;
use egui_phosphor::regular::PAPERCLIP as ICON_PAPERCLIP;
use gt_fmt::UTC_MINUTE_FORMAT;
use gt_log_view::LogAttachmentRef;
use gt_pending_writes::WriteAccess;
use gt_side_panel::widgets::{MetadataView, has_metadata_details, metadata_detail_rows};
use gt_store::{ChannelSummary, NavPointTimeRange, RecordingEntry};
use gt_ui_theme::EM_DASH;
use gt_ui_theme::buttons::SortHeaderButton;
use strum::{EnumCount as _, IntoEnumIterator as _};

use super::{HistorySort, RenameEdit, SortColumn};
use crate::app::history_db::{DeleteReason, HistoryWorker};
use crate::app::read_only_session::READ_ONLY_RECORDING_HISTORY_HOVER;

/// The scrolling recordings table, laid out like a file manager's list: the
/// metadata columns (date, duration, points, size, actions) size to their
/// fixed-format content, and the identity column fills whatever width is left,
/// clipping long names. Its width is a function of the window's width, so the
/// table is always exactly as wide as the window.
///
/// Identity is sized as a [`Column::exact`] recomputed each frame. A
/// [`Column::remainder`] that is not the *last* column ratchets in `egui_extras`:
/// it feeds its clipped width back into its own minimum every frame, so it can
/// never shrink again, which stops the window from being made narrower and lets
/// it creep wider.
pub(super) fn history_table(
    ui: &mut egui::Ui,
    HistoryTable {
        max_listing_height,
        visible,
        loaded_metas,
        worker,
        rename,
        sort,
        write_access,
    }: HistoryTable<'_>,
) {
    let row_height = ui.text_style_height(&egui::TextStyle::Body) + 6.0;
    // The header row and the gap under it come out of the listing's budget
    // before the scrolling body gets what is left.
    let max_scroll_height =
        (max_listing_height - row_height - ui.spacing().item_spacing.y).max(0.0);

    // Identity is window width minus last frame's metadata width, see this
    // function's doc comment.
    let available_width = ui.available_width();
    let metadata_width_id = ui.id().with("history_metadata_width");
    let identity_width = ui
        .data(|d| d.get_temp::<f32>(metadata_width_id))
        .map_or(IDENTITY_DEFAULT_WIDTH, |metadata| {
            (available_width - metadata).max(IDENTITY_MIN_WIDTH)
        });

    // Right edges of the identity and last (action) columns, captured from the
    // header this frame to measure the metadata width for the next one. The
    // measurement is only valid outside the table's sizing pass: during it the
    // auto columns have not yet grown to their content, so the reserve reads too
    // small and identity briefly blows up (window sticks wide).
    let identity_right = Cell::new(0.0_f32);
    let last_column_right = Cell::new(0.0_f32);
    let measured_while_sizing = Cell::new(false);

    TableBuilder::new(ui)
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
        .column(Column::exact(identity_width).clip(true))
        // Metadata columns size to their fixed-format content - every sortable
        // column except identity, which was added above. They are not
        // resizable: there is nothing to gain from resizing a date or a byte
        // count, and it keeps the table's width fully determined by the window.
        .columns(Column::auto().resizable(false), SortColumn::COUNT - 1)
        .column(Column::auto().resizable(false))
        .header(row_height, |mut header| {
            // Driven off `SortColumn`, so a new sortable column cannot be added
            // without a header appearing.
            for column in SortColumn::iter() {
                header.col(|ui| {
                    // Identity is the measured, term-explained column. Every
                    // other one is a plain sortable header.
                    let term = (column == SortColumn::Identity).then(|| {
                        identity_right.set(ui.max_rect().right());
                        measured_while_sizing.set(ui.is_sizing_pass());
                        crate::terms::IDENTITY
                    });
                    sort_header(ui, column, sort, term);
                });
            }
            header.col(|ui| {
                last_column_right.set(ui.max_rect().right());
            });
        })
        .body(|body| {
            body.rows(row_height, visible.len(), |mut row| {
                // In-range by construction. `get` guards anyway.
                let Some(entry) = visible.get(row.index()) else {
                    return;
                };
                let already_loaded = loaded_metas.iter().any(|m| m.same_recording(&entry.meta));
                render_row(
                    &mut row,
                    entry,
                    already_loaded,
                    worker,
                    rename,
                    write_access,
                );
            });
        });

    // Record the metadata columns' total width (everything right of identity)
    // for next frame's fill calculation. The header always renders - unlike the
    // virtualized body rows - so these edges are always fresh.
    let metadata_width = last_column_right.get() - identity_right.get();
    if metadata_width > 0.0 && !measured_while_sizing.get() {
        ui.data_mut(|d| d.insert_temp(metadata_width_id, metadata_width));
    }
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
    /// The recordings already in the window, whose rows cannot be opened
    /// again.
    pub loaded_metas: &'a [gt_store::RecordingMeta],
    pub worker: &'a HistoryWorker,
    pub rename: &'a mut Option<RenameEdit>,
    pub sort: &'a mut HistorySort,
    pub write_access: WriteAccess,
}

/// Identity never collapses below a readable width, even in a narrow window.
const IDENTITY_MIN_WIDTH: f32 = 160.0;

/// Identity's width on the very first frame, before the metadata columns have
/// been measured (see [`history_table`]). From then on it fills the leftover
/// width. Kept above [`IDENTITY_MIN_WIDTH`] so this bootstrap value is already
/// a readable width.
const IDENTITY_DEFAULT_WIDTH: f32 = 280.0;

fn render_row(
    row: &mut TableRow<'_, '_>,
    entry: &RecordingEntry,
    already_loaded: bool,
    worker: &HistoryWorker,
    rename: &mut Option<RenameEdit>,
    write_access: WriteAccess,
) {
    // Identity column: the inline editor when this row is being renamed,
    // otherwise the normal cell.
    row.col(|ui| {
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
        ui.label(gt_store::format_count_suffix(entry.meta.nav_point_count));
        if entry.hidden_tracks > 0 {
            ui.weak(format!(
                "({}/{} hidden)",
                entry.hidden_tracks, entry.total_tracks
            ));
        }
    });

    breakdown_cell(row, entry, SortColumn::Size, |ui| {
        ui.label(gt_fmt::format_bytes(entry.meta.gtd_size_bytes));
    });

    row.col(|ui| {
        attached_logs_cell(ui, entry, worker);
    });

    row.col(|ui| {
        let open = ui.add_enabled(!already_loaded, Button::new("Open").small());
        if already_loaded {
            open.on_hover_text("Already loaded");
        } else if open.clicked() {
            worker.open(entry.db_ref.clone());
        }
        if ui
            .add_enabled(write_access.allows_writing(), Button::new("Delete").small())
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
    let count = entry.log_attachments.len();
    if count == 0 {
        return;
    }
    ui.menu_button(format!("{ICON_PAPERCLIP} {count}"), |ui| {
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

pub(in crate::app) const OPEN_LOG_LABEL: &str = "Open log";

const OPEN_LOG_HOVER: &str = "Load this log into the log viewer";

const ATTACHED_LOGS_HOVER: &str = "The logs stored with this recording";

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

    if entry.hidden_tracks > 0 {
        ui.label(
            RichText::new(
                "Hidden tracks came from 'remove filtered data'. \
                 Use 'Delete hidden data' to drop them permanently.",
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

/// How many channels the hover lists before summarizing the rest, so a
/// recording carrying dozens of them still produces a readable tooltip.
pub(super) const MAX_HOVER_CHANNELS: usize = 8;

/// A channel's name, with a vector channel's component labels appended:
/// `accel (x, y, z)`. A scalar channel is just its name.
pub(super) fn channel_title(channel: &ChannelSummary) -> String {
    if channel.components.is_empty() {
        return channel.name.clone();
    }
    format!("{} ({})", channel.name, channel.components.join(", "))
}

/// The recording's track count, noting how many of them are hidden.
pub(super) fn track_count_text(entry: &RecordingEntry) -> String {
    if entry.hidden_tracks > 0 {
        format!("{} ({} hidden)", entry.total_tracks, entry.hidden_tracks)
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
    let has_metadata = has_metadata_details(&meta);
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
            metadata_detail_rows(ui, &meta);
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
