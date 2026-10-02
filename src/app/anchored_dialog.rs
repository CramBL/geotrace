use gt_ui_components::{AnchoredWindow, AnchoredWindowPhase, AnchoredWindowSizing, FrozenRegions};
use strum::EnumIter;

use crate::app::modals::{self, DialogActionRow, DialogBody, DialogBodyHeight};

/// Every dialog [`AnchoredDialog`] draws. A new dialog names itself here and
/// the suite in `tests` then holds it to the layout guarantees.
#[derive(Clone, Copy, Debug, Hash, PartialEq, Eq, EnumIter)]
pub(super) enum AnchoredDialogKind {
    AboutGeoTrace,
    ArchiveHeldByTheOtherInstance,
    AssociateLog,
    AutoPrune,
    DeleteArchivedDays,
    DeleteShelvedTracks,
    ForceQuit,
    HistoryDatabaseCorrupted,
    HistoryDatabaseInUse,
    HistoryDatabaseLocked,
    MapboxToken,
    RecordingsAlreadyInHistory,
    RecoverArchive,
    ShelveItems,
    SnapToRoadAgain,
    SnapToRoadAutomatically,
    SnapToRoadConsent,
    SnapToRoadScope,
    TakeOverWriteAccess,
    TrackSettingsDiffer,
    #[cfg(feature = "self-update")]
    UpdateAvailable,
    WaitingForTheDataDirectory,
}

impl AnchoredDialogKind {
    /// The id every anchored dialog holds its size, its position and its
    /// frozen regions under.
    pub(super) fn window_id(self) -> egui::Id {
        egui::Id::new(self)
    }

    fn width(self) -> f32 {
        match self {
            // Room for the attribution lines that pair a sentence with a link.
            Self::AboutGeoTrace => 400.0,
            // Room for the two sentences about the archive the other GeoTrace
            // has open, on two lines.
            Self::ArchiveHeldByTheOtherInstance => 460.0,
            // Room for a recording name beside how much of the log it ran
            // alongside.
            Self::AssociateLog => 460.0,
            // Room for a recording identity and its group name on one line.
            Self::AutoPrune => 480.0,
            // Room for an archive's name beside the days it loses, and for a
            // recording name on one line.
            Self::DeleteArchivedDays => 480.0,
            // Room for the sentence counting the shelved tracks on one line.
            Self::DeleteShelvedTracks => 420.0,
            // Fits inside the window that shutdown sizes itself down to.
            Self::ForceQuit => 360.0,
            // Room for each of the two sentences about the unreadable file on
            // one line.
            Self::HistoryDatabaseCorrupted => 400.0,
            // Room for each of the two sentences about the other process on
            // two lines.
            Self::HistoryDatabaseInUse => 460.0,
            // Room for the sentence about an unclean shutdown, and for the
            // warning under it, on two lines each.
            Self::HistoryDatabaseLocked => 460.0,
            // Room for the token field between its label and the Apply button.
            Self::MapboxToken => 420.0,
            // Room for a recording's file name on one line, and for each
            // sentence about the two versions of it on two lines.
            Self::RecordingsAlreadyInHistory => 460.0,
            // Room for the sentence about the interrupted delete, and for
            // the one stating when write access was taken, on two lines each.
            Self::RecoverArchive => 460.0,
            // Room for a track's name beside its number, distance and
            // duration, and for the line stating what the confirmation does in
            // history.
            Self::ShelveItems => 420.0,
            // Room for the statement about replacing the stored result on two
            // lines.
            Self::SnapToRoadAgain => 380.0,
            // Room for the default server's URL on one line.
            Self::SnapToRoadAutomatically => 420.0,
            // Room for the default server's URL on one line, and for the three
            // buttons on one row.
            Self::SnapToRoadConsent => 420.0,
            // Room for the two scope rows, and for the statement about
            // replacing data on two lines under them.
            Self::SnapToRoadScope => 380.0,
            // Room for each statement about what the other GeoTrace is doing
            // on two lines, and for the warning about writing to the
            // recordings on four.
            Self::TakeOverWriteAccess => 460.0,
            // Room for a recording name to wrap at a readable length, and for
            // the stored and current settings side by side.
            Self::TrackSettingsDiffer => 480.0,
            // Room for the primary action and the two dismissals beside it,
            // and for each statement the install reports on one line.
            #[cfg(feature = "self-update")]
            Self::UpdateAvailable => 460.0,
            // Room for each statement about the instance holding the data
            // directory on three lines.
            Self::WaitingForTheDataDirectory => 360.0,
        }
    }
}

pub(super) struct AnchoredDialog<'a> {
    window: AnchoredWindow,
    open: Option<&'a mut bool>,
}

impl<'a> AnchoredDialog<'a> {
    pub(super) fn new(kind: AnchoredDialogKind, title: impl Into<String>) -> Self {
        let title = title.into();
        Self {
            window: AnchoredWindow {
                layout_id: kind.window_id(),
                window_id: egui::Id::new(Some(title.as_str())),
                title,
                sizing: AnchoredWindowSizing {
                    preferred_width: kind.width(),
                    maximum_viewport_fraction: MAX_VIEWPORT_FRACTION,
                },
            },
            open: None,
        }
    }

    pub(super) fn with_close_button(mut self, open: &'a mut bool) -> Self {
        self.open = Some(open);
        self
    }

    pub(super) fn regions(&self) -> FrozenRegions {
        self.window.regions()
    }

    pub(super) fn show<R>(
        self,
        ctx: &egui::Context,
        body: DialogBody<impl FnOnce(&mut egui::Ui)>,
        actions: DialogActionRow<impl FnOnce(&mut egui::Ui), impl FnOnce(&mut egui::Ui) -> R>,
    ) -> Option<R> {
        self.window
            .show_ui(ctx, self.open, |ui, phase| {
                let height = match phase {
                    AnchoredWindowPhase::HoldHeight => DialogBodyHeight::TheHeldHeight,
                    AnchoredWindowPhase::MeasureContent => DialogBodyHeight::WhatItsContentNeeds,
                };
                modals::dialog_body_above_the_action_row_taking(ui, height, body, actions)
            })
            .and_then(|window| window.inner)
    }
}

const MAX_VIEWPORT_FRACTION: f32 = 0.9;

#[cfg(test)]
mod tests;
