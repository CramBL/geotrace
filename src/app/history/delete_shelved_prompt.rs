//! The confirmation for permanently deleting every shelved track, which the
//! History window's "Delete shelved data…" button raises.
//!
//! The count comes from the recording list, which the window reads again after
//! every change to the database: a recording finishing its load, an auto-prune,
//! or a track unshelved elsewhere. The confirmation stays up through all of
//! them, and reports that every track is live again once the count reaches
//! zero.

use std::time::Instant;

use egui::{Button, Label, RichText};
use gt_ui_theme::warning_amber;

use crate::app::anchored_dialog::AnchoredDialogKind;
use crate::app::modals::{self, CountdownToTheClose, PointerOverTheDialog, TimeUntilTheClose};

use super::DESTRUCTIVE_DELETE_HOVER;

#[cfg(test)]
mod tests;

pub(super) const DELETE_SHELVED_WINDOW_TITLE: &str = "Delete shelved data?";

const DELETE_SHELVED_TRACKS_LABEL: &str = "Delete shelved tracks";

const NOTHING_LEFT_TO_DELETE_HOVER: &str = "Every track is live again";

const CLOSE_BUTTON_HOVER: &str = "Closes this confirmation now. It closes on its own when the \
                                  count reaches zero. The count holds while the pointer is over \
                                  this window.";

/// The user confirmed the delete.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct DeleteShelvedTracks;

#[derive(Clone, Copy, PartialEq, Eq)]
enum DeleteShelvedTracksChoice {
    Delete,
    Dismiss,
}

/// What the open confirmation shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DeleteShelvedTracksPromptContents {
    /// How many tracks the delete removes, as the recording list last counted
    /// them.
    ShelvedTracks(usize),
    /// Nothing is left for the delete to remove.
    EveryTrackIsLive(TimeUntilTheClose),
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(super) enum DeleteShelvedTracksPrompt {
    #[default]
    Closed,
    /// Asking the user, at the count the recording list last reported.
    ConfirmingTheDelete { shelved_tracks: usize },
    /// Reporting that every track is live again, counting down to its own
    /// close. The confirmation never counts tracks again from here.
    ReportingThatEveryTrackIsLive(CountdownToTheClose),
}

impl DeleteShelvedTracksPrompt {
    pub(super) fn open(&mut self, shelved_tracks: usize) {
        *self = Self::ConfirmingTheDelete { shelved_tracks };
    }

    pub(super) fn is_up(self) -> bool {
        !matches!(self, Self::Closed)
    }

    /// Draws the open confirmation, and reports the delete on the frame the
    /// user confirms it.
    ///
    /// `shelved_track_count` is what the recording list counts across the
    /// stored recordings, and [`None`] while a list request is in flight.
    pub(super) fn show(
        &mut self,
        ctx: &egui::Context,
        now: Instant,
        shelved_track_count: Option<usize>,
    ) -> Option<DeleteShelvedTracks> {
        let contents = self.contents_to_show(now, shelved_track_count)?;
        if let Self::ReportingThatEveryTrackIsLive(countdown) = *self {
            countdown.request_the_repaint_the_count_needs(ctx);
        }
        let response = show_delete_shelved_tracks_confirmation(ctx, contents);
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
    /// The count is read from the recording list every frame: a track
    /// unshelved since the confirmation opened is one the delete leaves alone.
    /// The count the confirmation last showed stands while a list request is in
    /// flight.
    fn contents_to_show(
        &mut self,
        now: Instant,
        shelved_track_count: Option<usize>,
    ) -> Option<DeleteShelvedTracksPromptContents> {
        match *self {
            Self::Closed => None,
            Self::ConfirmingTheDelete { shelved_tracks } => {
                let shelved_tracks = shelved_track_count.unwrap_or(shelved_tracks);
                if shelved_tracks == 0 {
                    let countdown = CountdownToTheClose::started_at(now);
                    *self = Self::ReportingThatEveryTrackIsLive(countdown);
                    return Some(DeleteShelvedTracksPromptContents::EveryTrackIsLive(
                        countdown.time_until_the_close(),
                    ));
                }
                *self = Self::ConfirmingTheDelete { shelved_tracks };
                Some(DeleteShelvedTracksPromptContents::ShelvedTracks(
                    shelved_tracks,
                ))
            }
            Self::ReportingThatEveryTrackIsLive(countdown) => {
                Some(DeleteShelvedTracksPromptContents::EveryTrackIsLive(
                    countdown.time_until_the_close(),
                ))
            }
        }
    }

    /// Closes the confirmation on the user's choice, reporting a confirmed
    /// delete.
    fn record_choice(&mut self, choice: DeleteShelvedTracksChoice) -> Option<DeleteShelvedTracks> {
        *self = Self::Closed;
        match choice {
            DeleteShelvedTracksChoice::Delete => Some(DeleteShelvedTracks),
            DeleteShelvedTracksChoice::Dismiss => None,
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

/// Confirm permanently removing every shelved track from its recording,
/// stating how many there are, or report that every track is live again.
fn show_delete_shelved_tracks_confirmation(
    ctx: &egui::Context,
    contents: DeleteShelvedTracksPromptContents,
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
                DeleteShelvedTracksPromptContents::ShelvedTracks(shelved_tracks) => {
                    let track_label = gt_fmt::pluralize(shelved_tracks, "track", "tracks");
                    let removal = format!(
                        "{shelved_tracks} shelved {track_label} will be permanently removed \
                         from their recordings."
                    );
                    ui.add(Label::new(removal).wrap());
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
                DeleteShelvedTracksPromptContents::ShelvedTracks(_) => {
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
