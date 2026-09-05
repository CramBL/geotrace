//! The confirmation for permanently deleting shelved tracks, which the History
//! window's "Delete shelved data…" button raises over every recording and the
//! shelf's closing line over the one recording it is open on.
//!
//! The figures come from the recording list, which the window reads again after
//! every change to the database: a recording finishing its load, an auto-prune,
//! or a track unshelved elsewhere. The confirmation stays up through all of
//! them, and reports that every track is live again once the count reaches
//! zero.

use std::time::Instant;

use egui::{Button, Label, RichText};
use gt_store::RecordingEntry;
use gt_ui_theme::warning_amber;

use crate::app::anchored_dialog::AnchoredDialogKind;
use crate::app::history_db::DeleteShelvedTracksScope;
use crate::app::modals::{self, CountdownToTheClose, PointerOverTheDialog, TimeUntilTheClose};

use super::DESTRUCTIVE_DELETE_HOVER;

#[cfg(test)]
mod tests;

pub(super) const DELETE_SHELVED_WINDOW_TITLE: &str = "Delete shelved data?";

pub(super) const DELETE_SHELVED_TRACKS_LABEL: &str = "Delete shelved tracks";

const NOTHING_LEFT_TO_DELETE_HOVER: &str = "Every track is live again";

const CLOSE_BUTTON_HOVER: &str = "Closes this confirmation now. It closes on its own when the \
                                  count reaches zero. The count holds while the pointer is over \
                                  this window.";

/// Holds the list to a readable height whatever the database holds: how many
/// recordings the confirmation writes out before it counts the rest.
const RECORDINGS_WRITTEN_OUT: usize = 5;

#[derive(Clone, Copy, PartialEq, Eq)]
enum DeleteShelvedTracksChoice {
    Delete,
    Dismiss,
}

/// What a delete of shelved tracks removes, as the recording list last counted
/// it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(super) struct ShelvedTracksToDelete {
    tracks: usize,
    /// The recordings the delete removes from history entirely, in the
    /// listing's order, written as `identity/group_name`. A recording is in
    /// this list when it holds only shelved tracks.
    recordings_deleted_whole: Vec<String>,
}

impl ShelvedTracksToDelete {
    /// What a delete over `scope` removes, read from the recording list.
    ///
    /// `total_tracks` and `shelved_tracks` both count a recording's live and
    /// shelved tracks and skip the tombstones of the tracks already deleted
    /// permanently. The two are therefore equal exactly for a recording that
    /// the purge would leave without a track, and the purge takes such a
    /// recording out of history whole.
    fn of(scope: &DeleteShelvedTracksScope, listing: &[RecordingEntry]) -> Self {
        let mut shelved = Self::default();
        for entry in listing {
            if entry.shelved_tracks == 0 || !scope.covers(&entry.db_ref) {
                continue;
            }
            shelved.tracks += entry.shelved_tracks;
            if entry.shelved_tracks == entry.total_tracks {
                shelved
                    .recordings_deleted_whole
                    .push(entry.db_ref.to_string());
            }
        }
        shelved
    }
}

/// What the open confirmation shows.
#[derive(Debug, Clone, PartialEq, Eq)]
enum DeleteShelvedTracksPromptContents {
    /// What the delete removes, as the recording list last counted it.
    ShelvedTracks {
        scope: DeleteShelvedTracksScope,
        shelved: ShelvedTracksToDelete,
    },
    /// Nothing is left for the delete to remove.
    EveryTrackIsLive(TimeUntilTheClose),
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(super) enum DeleteShelvedTracksPrompt {
    #[default]
    Closed,
    /// Asking the user, at the figures the recording list last reported.
    ConfirmingTheDelete {
        scope: DeleteShelvedTracksScope,
        shelved: ShelvedTracksToDelete,
    },
    /// Reporting that every track is live again, counting down to its own
    /// close. The confirmation never counts tracks again from here.
    ReportingThatEveryTrackIsLive(CountdownToTheClose),
}

impl DeleteShelvedTracksPrompt {
    /// Raise the confirmation over the recordings `scope` covers, at what
    /// `listing` reports the delete would take.
    pub(super) fn open(&mut self, scope: DeleteShelvedTracksScope, listing: &[RecordingEntry]) {
        let shelved = ShelvedTracksToDelete::of(&scope, listing);
        *self = Self::ConfirmingTheDelete { scope, shelved };
    }

    pub(super) fn is_up(&self) -> bool {
        !matches!(self, Self::Closed)
    }

    /// Draws the open confirmation, and returns the scope to delete over on the
    /// frame the user confirms it.
    ///
    /// `listing` is the recording list the History window holds, and [`None`]
    /// while a list request is in flight.
    pub(super) fn show(
        &mut self,
        ctx: &egui::Context,
        now: Instant,
        listing: Option<&[RecordingEntry]>,
    ) -> Option<DeleteShelvedTracksScope> {
        let contents = self.contents_to_show(now, listing)?;
        if let Self::ReportingThatEveryTrackIsLive(countdown) = *self {
            countdown.request_the_repaint_the_count_needs(ctx);
        }
        let response = show_delete_shelved_tracks_confirmation(ctx, &contents);
        match response.choice {
            Some(choice) => self.record_choice(choice),
            None => {
                self.advance_the_countdown_and_close_when_it_runs_out(now, response.pointer);
                None
            }
        }
    }

    /// What the open confirmation shows, or [`None`] while it is closed.
    ///
    /// The figures are read from the recording list every frame: a track
    /// unshelved since the confirmation opened is one the delete leaves alone.
    /// The figures the confirmation last showed stand while a list request is
    /// in flight.
    fn contents_to_show(
        &mut self,
        now: Instant,
        listing: Option<&[RecordingEntry]>,
    ) -> Option<DeleteShelvedTracksPromptContents> {
        match self {
            Self::Closed => None,
            Self::ConfirmingTheDelete { scope, shelved } => {
                if let Some(listing) = listing {
                    *shelved = ShelvedTracksToDelete::of(scope, listing);
                }
                if shelved.tracks == 0 {
                    let countdown = CountdownToTheClose::started_at(now);
                    *self = Self::ReportingThatEveryTrackIsLive(countdown);
                    return Some(DeleteShelvedTracksPromptContents::EveryTrackIsLive(
                        countdown.time_until_the_close(),
                    ));
                }
                Some(DeleteShelvedTracksPromptContents::ShelvedTracks {
                    scope: scope.clone(),
                    shelved: shelved.clone(),
                })
            }
            Self::ReportingThatEveryTrackIsLive(countdown) => {
                Some(DeleteShelvedTracksPromptContents::EveryTrackIsLive(
                    countdown.time_until_the_close(),
                ))
            }
        }
    }

    /// Closes the confirmation on the user's choice, returning the scope of a
    /// confirmed delete.
    fn record_choice(
        &mut self,
        choice: DeleteShelvedTracksChoice,
    ) -> Option<DeleteShelvedTracksScope> {
        match (std::mem::take(self), choice) {
            (Self::ConfirmingTheDelete { scope, .. }, DeleteShelvedTracksChoice::Delete) => {
                Some(scope)
            }
            _ => None,
        }
    }

    /// The pointer resting over the confirmation holds the count: closing the
    /// confirmation under the pointer would send the press that follows to the
    /// History window behind it.
    fn advance_the_countdown_and_close_when_it_runs_out(
        &mut self,
        now: Instant,
        pointer: PointerOverTheDialog,
    ) {
        let Self::ReportingThatEveryTrackIsLive(mut countdown) = *self else {
            return;
        };
        countdown.advance_to(now, pointer);
        *self = if countdown.has_run_out() {
            Self::Closed
        } else {
            Self::ReportingThatEveryTrackIsLive(countdown)
        };
    }
}

/// What the confirmation reports for the frame it drew.
struct DeleteShelvedTracksPromptResponse {
    /// The choice in the frame the user makes it, and [`None`] on every other
    /// frame the confirmation is up.
    choice: Option<DeleteShelvedTracksChoice>,
    pointer: PointerOverTheDialog,
}

/// Confirm permanently removing shelved tracks from the recordings in scope,
/// stating how many there are and which recordings the delete removes
/// entirely, or report that every track is live again.
fn show_delete_shelved_tracks_confirmation(
    ctx: &egui::Context,
    contents: &DeleteShelvedTracksPromptContents,
) -> DeleteShelvedTracksPromptResponse {
    let mut pointer = PointerOverTheDialog::Away;
    let choice = modals::anchored_confirmation_dialog(
        ctx,
        AnchoredDialogKind::DeleteShelvedTracks,
        DELETE_SHELVED_WINDOW_TITLE,
        DeleteShelvedTracksChoice::Dismiss,
        |ui, _regions| {
            pointer = PointerOverTheDialog::of(ui);
            match contents {
                DeleteShelvedTracksPromptContents::ShelvedTracks { scope, shelved } => {
                    let tracks = shelved.tracks;
                    let track_label = gt_fmt::pluralize(tracks, "track", "tracks");
                    let removal = match scope {
                        DeleteShelvedTracksScope::EveryRecording => format!(
                            "{tracks} shelved {track_label} will be permanently removed from \
                             their recordings."
                        ),
                        DeleteShelvedTracksScope::OneRecording(db_ref) => format!(
                            "{tracks} shelved {track_label} will be permanently removed from \
                             {db_ref}."
                        ),
                    };
                    ui.add(Label::new(removal).wrap());
                    recordings_deleted_whole_ui(ui, &shelved.recordings_deleted_whole);
                }
                DeleteShelvedTracksPromptContents::EveryTrackIsLive(_) => {
                    ui.add(
                        Label::new("Every track is live again: there is nothing left to delete")
                            .wrap(),
                    );
                }
            }
        },
        |ui| {
            let mut choice = None;
            let dismiss = match contents {
                DeleteShelvedTracksPromptContents::ShelvedTracks { .. } => {
                    if ui
                        .button(
                            RichText::new(DELETE_SHELVED_TRACKS_LABEL)
                                .color(warning_amber(ui.visuals().dark_mode)),
                        )
                        .on_hover_text(DESTRUCTIVE_DELETE_HOVER)
                        .clicked()
                    {
                        choice = Some(DeleteShelvedTracksChoice::Delete);
                    }
                    ui.button("Cancel")
                }
                DeleteShelvedTracksPromptContents::EveryTrackIsLive(time_until_the_close) => {
                    ui.add_enabled(false, Button::new(DELETE_SHELVED_TRACKS_LABEL))
                        .on_disabled_hover_text(NOTHING_LEFT_TO_DELETE_HOVER);
                    ui.button(time_until_the_close.close_button_label())
                        .on_hover_text(CLOSE_BUTTON_HOVER)
                }
            };
            if dismiss.clicked() {
                choice = Some(DeleteShelvedTracksChoice::Dismiss);
            }
            choice
        },
    );
    DeleteShelvedTracksPromptResponse { choice, pointer }
}

/// The recordings the delete removes from history entirely, which the line
/// above this one counts only as tracks.
///
/// One such recording is written out in the line itself. Several are counted in
/// the line and written out under it, up to [`RECORDINGS_WRITTEN_OUT`] of them.
fn recordings_deleted_whole_ui(ui: &mut egui::Ui, names: &[String]) {
    if names.is_empty() {
        return;
    }
    ui.add_space(4.0);
    match names {
        [name] => {
            let line =
                format!("{name} holds only shelved tracks, so this delete removes it entirely.");
            ui.add(
                Label::new(RichText::new(line).color(warning_amber(ui.visuals().dark_mode))).wrap(),
            );
        }
        _ => {
            let line = format!(
                "{} recordings hold only shelved tracks, so this delete removes them entirely:",
                names.len()
            );
            ui.add(
                Label::new(RichText::new(line).color(warning_amber(ui.visuals().dark_mode))).wrap(),
            );
            for name in names.iter().take(RECORDINGS_WRITTEN_OUT) {
                ui.add(Label::new(name.as_str()).truncate());
            }
            let rest = names.len().saturating_sub(RECORDINGS_WRITTEN_OUT);
            if rest > 0 {
                ui.weak(format!("and {rest} more"));
            }
        }
    }
}
