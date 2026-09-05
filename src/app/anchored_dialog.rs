//! The window a dialog anchored over the app draws itself in.
//!
//! Every control keeps the place the user saw it in: [`AnchoredDialog`] fixes
//! the window's position on the frame it opens, and each region of the body
//! keeps the height it had then. egui places an anchored
//! [`egui::Window`] from the size its content took on the previous frame, and
//! hit-tests a press against the widget rects of that frame. A dialog that
//! grows while the pointer rests on one of its controls moves that control out
//! from under the pointer, and the press lands on whatever took its place.

use std::collections::BTreeMap;

use egui::emath::GuiRounding as _;
use egui::{ScrollArea, TextStyle, Window};
use strum::EnumIter;

use crate::app::modals::{self, DialogActionRow, DialogBody, DialogBodyHeight};

#[cfg(test)]
mod tests;

/// The share of the viewport a dialog may take before the user resizes it.
const MAX_VIEWPORT_FRACTION: f32 = 0.9;

/// Where a dialog's frozen region heights sit under its window id.
const FROZEN_REGIONS: &str = "frozen_regions";

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

/// What one dialog holds from the pass it opened on.
#[derive(Clone, Default)]
struct HeldLayout {
    /// The pass the dialog last drew on: a gap in the passes is a fresh open.
    last_drawn_pass: u64,

    /// The outer height the dialog's content took, and the left-top corner
    /// centring it on the viewport. `None` until the opening pass ends.
    size: Option<HeldSize>,
}

/// The window size and position the dialog measured on the pass it opened on.
#[derive(Clone, Copy)]
struct HeldSize {
    height: f32,
    position: egui::Pos2,

    /// Whether egui's own height for the window has been held down to
    /// [`HeldSize::height`] yet. The pass after the opening one holds that
    /// height down: egui grows it towards the content and never back. On
    /// every pass after that the user can drag the window larger.
    capped: bool,
}

/// The height each region of one dialog's body holds, keyed by the region's
/// salt.
#[derive(Clone, Default)]
struct FrozenRegionHeights(BTreeMap<&'static str, f32>);

/// The height one region of a dialog's body holds, in lines of body text.
///
/// The region takes the height its content needed on the frame the dialog
/// opened. These two counts are the floor and the ceiling on that height.
#[derive(Clone, Copy)]
pub(super) struct HeldBodyLines {
    at_least: u8,
    at_most: Option<u8>,
}

impl HeldBodyLines {
    /// For a region whose content is all there when the dialog opens.
    pub(super) fn what_the_content_took() -> Self {
        Self {
            at_least: 0,
            at_most: None,
        }
    }

    /// At least `lines`. That is the room for content that arrives after the
    /// dialog opens.
    pub(super) fn at_least(lines: u8) -> Self {
        Self {
            at_least: lines,
            at_most: None,
        }
    }

    /// At most `lines`, for content with no length of its own to hold it: the
    /// rest scrolls inside the region.
    pub(super) fn and_at_most(self, lines: u8) -> Self {
        debug_assert!(
            lines >= self.at_least,
            "a region cannot hold at most {lines} lines: it already holds at least {}",
            self.at_least
        );
        Self {
            at_most: Some(lines),
            ..self
        }
    }
}

/// The regions of one dialog's body. Each keeps the height it had on the frame
/// the dialog opened, and content arriving after that scrolls inside it.
#[derive(Clone, Copy)]
pub(super) struct DialogRegions {
    id: egui::Id,
}

impl DialogRegions {
    /// Draws `content` at the height this region holds. `lines` bounds that
    /// height on the frame the dialog opens.
    pub(super) fn frozen_at_open<R>(
        self,
        ui: &mut egui::Ui,
        salt: &'static str,
        lines: HeldBodyLines,
        content: impl FnOnce(&mut egui::Ui) -> R,
    ) -> R {
        let frozen = ui.data(|data| {
            data.get_temp::<FrozenRegionHeights>(self.id)
                .and_then(|heights| heights.0.get(salt).copied())
        });
        if let Some(height) = frozen {
            return ScrollArea::vertical()
                .id_salt(salt)
                .auto_shrink(false)
                // A region shorter than 64 points would grow the dialog by
                // the difference: egui keeps 64 points for a scroll area to
                // scroll in.
                .min_scrolled_height(0.0)
                .max_height(height)
                .show(ui, content)
                .inner;
        }
        // The pass the dialog opened on, where the window has no height to
        // fill yet and every region takes what its content needs, up to what
        // `lines` holds it to.
        let line_height = ui.text_style_height(&TextStyle::Body);
        let laid_out = ui.scope(|ui| match lines.at_most {
            Some(most) => {
                let ceiling = f32::from(most) * line_height;
                ScrollArea::vertical()
                    .id_salt(salt)
                    .auto_shrink([false, true])
                    // egui sizes a scroll area from the height its parent
                    // has left, which is nothing on this pass. The ceiling is
                    // set as both the minimum and the maximum: the area takes
                    // the ceiling and then shrinks to its content.
                    .min_scrolled_height(ceiling)
                    .max_height(ceiling)
                    .show(ui, content)
                    .inner
            }
            None => content(ui),
        });
        let drawn = laid_out.response.rect.height();
        let held = drawn.max(f32::from(lines.at_least) * line_height);
        ui.add_space(held - drawn);
        // A window with no size of its own is measured on a sizing pass first,
        // where every widget takes its minimum height. The region holds the
        // height its content takes on the pass after that one, where a row
        // takes the height it interacts at.
        if !ui.is_sizing_pass() {
            ui.data_mut(|data| {
                data.get_temp_mut_or_default::<FrozenRegionHeights>(self.id)
                    .0
                    .insert(salt, held);
            });
        }
        laid_out.inner
    }
}

/// A dialog centred on the viewport, at the height its content took when it
/// opened.
pub(super) struct AnchoredDialog<'a> {
    kind: AnchoredDialogKind,
    title: String,
    area_id: egui::Id,
    open: Option<&'a mut bool>,
}

impl<'a> AnchoredDialog<'a> {
    /// Two dialogs on screen at once need titles that differ: egui derives the
    /// window's id from `title`, which is what `AuditedWindow::titled` and
    /// `HarnessInteraction::window_rect` look a window up by. The size, the
    /// position and the frozen regions the dialog holds sit under
    /// [`AnchoredDialogKind::window_id`].
    pub(super) fn new(kind: AnchoredDialogKind, title: impl Into<String>) -> Self {
        let title = title.into();
        Self {
            area_id: egui::Id::new(Some(title.as_str())),
            kind,
            title,
            open: None,
        }
    }

    /// Puts a close button in the title bar, which sets `open` to `false`.
    pub(super) fn with_close_button(mut self, open: &'a mut bool) -> Self {
        self.open = Some(open);
        self
    }

    /// The regions of this dialog's body, taken before
    /// [`show`](Self::show) so the body can draw into them.
    pub(super) fn regions(&self) -> DialogRegions {
        DialogRegions {
            id: self.kind.window_id().with(FROZEN_REGIONS),
        }
    }

    /// Draws the dialog: `body` scrolls inside the held height, and `actions`
    /// sit at its bottom edge. Returns `None` once the close button has set
    /// the flag given to [`with_close_button`](Self::with_close_button) to
    /// `false`.
    pub(super) fn show<R>(
        self,
        ctx: &egui::Context,
        body: DialogBody<impl FnOnce(&mut egui::Ui)>,
        actions: DialogActionRow<impl FnOnce(&mut egui::Ui), impl FnOnce(&mut egui::Ui) -> R>,
    ) -> Option<R> {
        let Self {
            kind,
            title,
            area_id,
            open,
        } = self;
        let held_id = kind.window_id().with("held_layout");
        let pass = ctx.cumulative_pass_nr();
        let mut held = ctx
            .data(|data| data.get_temp::<HeldLayout>(held_id))
            .unwrap_or_default();
        if held.last_drawn_pass + 1 < pass {
            held = HeldLayout::default();
            ctx.data_mut(|data| {
                data.remove::<FrozenRegionHeights>(kind.window_id().with(FROZEN_REGIONS));
            });
        }
        held.last_drawn_pass = pass;

        let viewport = ctx.content_rect();
        let width = kind.width().min(viewport.width() * MAX_VIEWPORT_FRACTION);
        let mut window = Window::new(title)
            .id(area_id)
            .collapsible(false)
            .resizable(true)
            .constrain_to(viewport)
            .min_width(width)
            // The height starts at nothing, and egui grows what it keeps for
            // the window towards the content and never back: the pass the
            // dialog opens on measures its content with no height to fill.
            .default_size(egui::vec2(width, 0.0));
        let cap = viewport.size() * MAX_VIEWPORT_FRACTION;
        let body_height = match held.size {
            Some(size) => {
                let height = if size.capped { cap.y } else { size.height };
                window = window
                    .fixed_pos(size.position)
                    .max_size(egui::vec2(cap.x, height));
                DialogBodyHeight::TheHeldHeight
            }
            #[expect(
                clippy::disallowed_methods,
                reason = "Every anchored dialog anchors its window here, and nowhere else"
            )]
            None => {
                window = window
                    .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
                    .max_size(cap);
                DialogBodyHeight::WhatItsContentNeeds
            }
        };
        if let Some(open) = open {
            window = window.open(open);
        }

        let laid_out = window.show(ctx, |ui| {
            modals::dialog_body_above_the_action_row_taking(ui, body_height, body, actions)
        });

        match held.size {
            // The opening pass has laid the content out: the window takes that
            // height and the position centring it from here on.
            None => {
                if let Some(rect) = laid_out.as_ref().map(|window| window.response.rect) {
                    let measured = rect.size().min(cap);
                    held.size = Some(HeldSize {
                        height: measured.y,
                        position: egui::Rect::from_center_size(viewport.center(), measured)
                            .left_top()
                            .round_ui(),
                        capped: false,
                    });
                }
            }
            Some(size) => {
                held.size = Some(HeldSize {
                    capped: true,
                    ..size
                })
            }
        }
        ctx.data_mut(|data| data.insert_temp(held_id, held));
        laid_out.and_then(|window| window.inner)
    }
}
