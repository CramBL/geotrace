//! The confirmation for permanently deleting every hidden track, which the
//! History window's "Delete hidden data…" button raises.
//!
//! The count comes from the recording list, which the window reads again after
//! every change to the database: a recording finishing its load, an auto-prune,
//! or a track unhidden elsewhere. The confirmation stays up through all of
//! them, and reports that no track is hidden any more once the count reaches
//! zero.

use std::time::Instant;

use egui::{Button, Label, RichText};
use gt_ui_theme::warning_amber;

use crate::app::anchored_dialog::AnchoredDialogKind;
use crate::app::modals::{self, CountdownToTheClose, PointerOverTheDialog, TimeUntilTheClose};

use super::DESTRUCTIVE_DELETE_HOVER;

#[cfg(test)]
mod tests;

pub(super) const DELETE_HIDDEN_WINDOW_TITLE: &str = "Delete hidden data?";

const DELETE_HIDDEN_TRACKS_LABEL: &str = "Delete hidden tracks";

const NOTHING_LEFT_TO_DELETE_HOVER: &str = "No track is hidden any more";

const CLOSE_BUTTON_HOVER: &str = "Closes this confirmation now. It closes on its own when the \
                                  count reaches zero. The count holds while the pointer is over \
                                  this window.";

/// The user confirmed the delete.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct DeleteHiddenTracks;

#[derive(Clone, Copy, PartialEq, Eq)]
enum DeleteHiddenTracksChoice {
    Delete,
    Dismiss,
}

/// What the open confirmation shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DeleteHiddenTracksPromptContents {
    /// How many tracks the delete removes, as the recording list last counted
    /// them.
    HiddenTracks(usize),
    /// Nothing is left for the delete to remove.
    NoTrackIsHidden(TimeUntilTheClose),
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(super) enum DeleteHiddenTracksPrompt {
    #[default]
    Closed,
    /// Asking the user, at the count the recording list last reported.
    ConfirmingTheDelete { hidden_tracks: usize },
    /// Reporting that no track is hidden any more, counting down to its own
    /// close. The confirmation never counts tracks again from here.
    ReportingThatNoTrackIsHidden(CountdownToTheClose),
}

impl DeleteHiddenTracksPrompt {
    pub(super) fn open(&mut self, hidden_tracks: usize) {
        *self = Self::ConfirmingTheDelete { hidden_tracks };
    }

    pub(super) fn is_up(self) -> bool {
        !matches!(self, Self::Closed)
    }

    /// Draws the open confirmation, and reports the delete on the frame the
    /// user confirms it.
    ///
    /// `hidden_track_count` is what the recording list counts across the
    /// stored recordings, and [`None`] while a list request is in flight.
    pub(super) fn show(
        &mut self,
        ctx: &egui::Context,
        now: Instant,
        hidden_track_count: Option<usize>,
    ) -> Option<DeleteHiddenTracks> {
        let contents = self.contents_to_show(now, hidden_track_count)?;
        if let Self::ReportingThatNoTrackIsHidden(countdown) = *self {
            countdown.request_the_repaint_the_count_needs(ctx);
        }
        let response = show_delete_hidden_tracks_confirmation(ctx, contents);
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
    /// The count is read from the recording list every frame: a track that
    /// stopped being hidden since the confirmation opened is no longer one the
    /// delete removes. The count the confirmation last showed stands while a
    /// list request is in flight.
    fn contents_to_show(
        &mut self,
        now: Instant,
        hidden_track_count: Option<usize>,
    ) -> Option<DeleteHiddenTracksPromptContents> {
        match *self {
            Self::Closed => None,
            Self::ConfirmingTheDelete { hidden_tracks } => {
                let hidden_tracks = hidden_track_count.unwrap_or(hidden_tracks);
                if hidden_tracks == 0 {
                    let countdown = CountdownToTheClose::started_at(now);
                    *self = Self::ReportingThatNoTrackIsHidden(countdown);
                    return Some(DeleteHiddenTracksPromptContents::NoTrackIsHidden(
                        countdown.time_until_the_close(),
                    ));
                }
                *self = Self::ConfirmingTheDelete { hidden_tracks };
                Some(DeleteHiddenTracksPromptContents::HiddenTracks(
                    hidden_tracks,
                ))
            }
            Self::ReportingThatNoTrackIsHidden(countdown) => Some(
                DeleteHiddenTracksPromptContents::NoTrackIsHidden(countdown.time_until_the_close()),
            ),
        }
    }

    /// Closes the confirmation on the user's choice, reporting a confirmed
    /// delete.
    fn record_choice(&mut self, choice: DeleteHiddenTracksChoice) -> Option<DeleteHiddenTracks> {
        *self = Self::Closed;
        match choice {
            DeleteHiddenTracksChoice::Delete => Some(DeleteHiddenTracks),
            DeleteHiddenTracksChoice::Dismiss => None,
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
        let Self::ReportingThatNoTrackIsHidden(mut countdown) = *self else {
            return;
        };
        countdown.advance_to(now, pointer);
        *self = if countdown.has_run_out() {
            Self::Closed
        } else {
            Self::ReportingThatNoTrackIsHidden(countdown)
        };
    }
}

/// What the confirmation reports for the frame it drew.
struct DeleteHiddenTracksPromptResponse {
    /// The choice in the frame the user makes it, and [`None`] on every other
    /// frame the confirmation is up.
    choice: Option<DeleteHiddenTracksChoice>,
    pointer: PointerOverTheDialog,
}

/// Confirm permanently removing every hidden track from its recording, naming
/// how many there are, or report that no track is hidden any more.
fn show_delete_hidden_tracks_confirmation(
    ctx: &egui::Context,
    contents: DeleteHiddenTracksPromptContents,
) -> DeleteHiddenTracksPromptResponse {
    let mut pointer = PointerOverTheDialog::Away;
    let choice = modals::anchored_confirmation_dialog(
        ctx,
        AnchoredDialogKind::DeleteHiddenTracks,
        DELETE_HIDDEN_WINDOW_TITLE,
        DeleteHiddenTracksChoice::Dismiss,
        |ui, _regions| {
            pointer = PointerOverTheDialog::of(ui);
            match contents {
                DeleteHiddenTracksPromptContents::HiddenTracks(hidden_tracks) => {
                    let track_label = gt_fmt::pluralize(hidden_tracks, "track", "tracks");
                    let removal = format!(
                        "{hidden_tracks} hidden {track_label} will be permanently removed from \
                         their recordings."
                    );
                    ui.add(Label::new(removal).wrap());
                }
                DeleteHiddenTracksPromptContents::NoTrackIsHidden(_) => {
                    ui.add(
                        Label::new("No track is hidden any more: there is nothing left to delete")
                            .wrap(),
                    );
                }
            }
        },
        |ui| {
            let mut choice = None;
            let dismiss = match contents {
                DeleteHiddenTracksPromptContents::HiddenTracks(_) => {
                    if ui
                        .button(
                            RichText::new(DELETE_HIDDEN_TRACKS_LABEL)
                                .color(warning_amber(ui.visuals().dark_mode)),
                        )
                        .on_hover_text(DESTRUCTIVE_DELETE_HOVER)
                        .clicked()
                    {
                        choice = Some(DeleteHiddenTracksChoice::Delete);
                    }
                    ui.button("Cancel")
                }
                DeleteHiddenTracksPromptContents::NoTrackIsHidden(time_until_the_close) => {
                    ui.add_enabled(false, Button::new(DELETE_HIDDEN_TRACKS_LABEL))
                        .on_disabled_hover_text(NOTHING_LEFT_TO_DELETE_HOVER);
                    ui.button(time_until_the_close.close_button_label())
                        .on_hover_text(CLOSE_BUTTON_HOVER)
                }
            };
            if dismiss.clicked() {
                choice = Some(DeleteHiddenTracksChoice::Dismiss);
            }
            choice
        },
    );
    DeleteHiddenTracksPromptResponse { choice, pointer }
}
