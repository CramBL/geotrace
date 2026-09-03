//! Asking the user about the interrupted deletes the day archives hold,
//! after they took write access from the instance holding the data directory.
//!
//! Recovering an interrupted delete discards every archived day of the
//! archive it is in. That is right for a delete a process that is gone left
//! behind, and wrong for a delete the instance the user just overrode may
//! still be running. A taken-over open therefore asks the user per archive
//! and recovers nothing on its own.
//!
//! The open therefore runs in two steps, with no write guard held while the
//! user decides: [`inspect_archives_under`] reads the archives and ends, the
//! prompts run in the frames after it, and the open in
//! [`super::storage::open_in`] follows with the choices.

use std::collections::VecDeque;
use std::fs;
use std::mem;
use std::path::{Path, PathBuf};
use std::sync::mpsc;

use chrono::DateTime;
use egui::RichText;
use egui_phosphor::regular::WARNING as ICON_WARNING;
use gt_fmt::UTC_MINUTE_FORMAT;
use gt_instance_lock::TakeOverRecord;
use gt_store::{
    DayArchiveError as _, EnvironmentArchive, FlareStore, InterruptedDelete,
    InterruptedDeleteRecovery, IonexStore, JamStore, PerArchive, ReadOnlyDayArchive as _,
    SolarStore, Store, StoredDayArchive,
};
use gt_ui_theme::warning_amber;

use super::anchored_dialog::AnchoredDialogKind;
use super::storage::StorageOpen;
use super::{App, modals, storage};

pub(in crate::app) const RECOVER_BUTTON_LABEL: &str = "Recover";

pub(in crate::app) const LEAVE_UNRECOVERED_BUTTON_LABEL: &str = "Leave unrecovered";

pub(in crate::app) const ARCHIVE_IN_USE_BUTTON_LABEL: &str = "Continue";

/// Starts the line an interrupted-delete prompt states a take-over on.
pub(in crate::app) const WRITE_ACCESS_TAKEN_FROM: &str =
    "Write access to this data directory was taken from";

/// What reading one archive found, where the user has to choose what to do
/// about it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::app) enum InterruptedDeleteFinding {
    /// A delete was interrupted part-way through the archive, leaving the
    /// days recovering it would discard.
    Interrupted {
        interrupted: InterruptedDelete,
        take_over: Option<TakeOverAfterTheArchiveWasLastWritten>,
    },
    /// Nothing here can read the file: the other GeoTrace has it open.
    HeldByTheOtherInstance,
}

/// A take-over recorded in the data directory, where the archive was last
/// written no later than the take-over. The interrupted-delete prompt for
/// that archive states it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::app) struct TakeOverAfterTheArchiveWasLastWritten(TakeOverRecord);

impl TakeOverAfterTheArchiveWasLastWritten {
    /// `record` where the archive at `path` was last written at or before
    /// the take-over, and [`None`] where a write followed it.
    ///
    /// A record without `written_at`, and an archive whose modification time
    /// `fs::metadata` does not give, are both stated: withholding the
    /// take-over costs more than stating one a later write may explain.
    fn of_the_archive_at(path: &Path, record: TakeOverRecord) -> Option<Self> {
        let Some(written_at) = record.written_at else {
            return Some(Self(record));
        };
        let last_written_at = fs::metadata(path)
            .and_then(|metadata| metadata.modified())
            .ok()
            .and_then(gt_instance_lock::seconds_since_the_epoch);
        match last_written_at {
            Some(last_written_at) if written_at < last_written_at => None,
            Some(_) | None => Some(Self(record)),
        }
    }

    /// The line the prompt states: which process write access was taken
    /// from, and when. Either is left out where the record has none.
    fn prompt_sentence(self) -> String {
        let taken_from = match self.0.taken_from_process_id {
            Some(process_id) => format!("another GeoTrace (process {process_id})"),
            None => "another GeoTrace".to_owned(),
        };
        let when = self
            .0
            .written_at
            .and_then(|written_at| i64::try_from(written_at).ok())
            .and_then(|seconds| DateTime::from_timestamp(seconds, 0))
            .map(|written_at| format!(" on {} UTC", written_at.format(UTC_MINUTE_FORMAT)))
            .unwrap_or_default();
        format!("{WRITE_ACCESS_TAKEN_FROM} {taken_from}{when}.")
    }
}

/// What the first step of a taken-over open read off the archives.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::app) struct InspectedArchives {
    /// Root the archives were read under, whose very files the open that
    /// follows opens. [`None`] for a run that opens nothing.
    root: Option<PathBuf>,
    /// The archives to ask the user about, in the order the settings rows
    /// list them.
    findings: Vec<(EnvironmentArchive, InterruptedDeleteFinding)>,
}

impl InspectedArchives {
    /// A run with nothing to read and nothing to ask the user about.
    pub(in crate::app) const fn of_nothing() -> Self {
        Self {
            root: None,
            findings: Vec::new(),
        }
    }

    /// The archives under `root`, with nothing found in them. What an
    /// inspection that never reported falls back to: the open goes ahead and
    /// declines whatever it meets.
    pub(in crate::app) const fn unread_under(root: Option<PathBuf>) -> Self {
        Self {
            root,
            findings: Vec::new(),
        }
    }

    /// The findings a test puts to the prompts, for an archive it cannot make
    /// this process report on its own.
    #[cfg(test)]
    pub(in crate::app) const fn of_findings_under(
        root: PathBuf,
        findings: Vec<(EnvironmentArchive, InterruptedDeleteFinding)>,
    ) -> Self {
        Self {
            root: Some(root),
            findings,
        }
    }
}

/// Read every day archive under `root` for an interrupted delete, which
/// writes nothing and leaves every file closed.
///
/// `previous_take_over` is the take-over recorded in the data directory
/// before the one this open follows, which the prompt for an archive written
/// no later than it states.
pub(in crate::app) fn inspect_archives_under(
    root: PathBuf,
    previous_take_over: Option<TakeOverRecord>,
) -> InspectedArchives {
    let store = Store::open_in(&root);
    let findings = [
        InterruptedDeleteFinding::read_from_the_archive::<JamStore>(&store, previous_take_over),
        InterruptedDeleteFinding::read_from_the_archive::<SolarStore>(&store, previous_take_over),
        InterruptedDeleteFinding::read_from_the_archive::<IonexStore>(&store, previous_take_over),
        InterruptedDeleteFinding::read_from_the_archive::<FlareStore>(&store, previous_take_over),
    ]
    .into_iter()
    .flatten()
    .collect();
    InspectedArchives {
        root: Some(root),
        findings,
    }
}

impl InterruptedDeleteFinding {
    /// What archive `A` holds for the user to choose about, or [`None`] where
    /// the open needs no choice.
    ///
    /// A failure that is neither an interrupted delete nor another process
    /// holding the file is left to the open to report: it fails there the
    /// same way it does on the normal path.
    fn read_from_the_archive<A: StoredDayArchive>(
        store: &Store,
        previous_take_over: Option<TakeOverRecord>,
    ) -> Option<(EnvironmentArchive, Self)> {
        let path = store.archive_path::<A>();
        let take_over = previous_take_over.and_then(|record| {
            TakeOverAfterTheArchiveWasLastWritten::of_the_archive_at(&path, record)
        });
        let finding = match A::ReadOnly::interrupted_delete_at(&path) {
            Ok(None) => return None,
            Ok(Some(interrupted)) => Self::Interrupted {
                interrupted,
                take_over,
            },
            Err(err) if err.is_held_by_another_process() => Self::HeldByTheOtherInstance,
            Err(err) => {
                log::debug!(
                    "Reading the {} archive for an interrupted delete failed, which the open \
                     reports: {err}",
                    A::ARCHIVE.label_in_sentence()
                );
                return None;
            }
        };
        Some((A::ARCHIVE, finding))
    }
}

/// Why an archive is closed for this session, as its prompt stated when the
/// user chose.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArchiveUnavailable {
    /// The archive keeps the days it holds: the user left the interrupted
    /// delete in it unrecovered.
    InterruptedDeleteLeftUnrecovered,
    /// The GeoTrace the user took write access from has the file open.
    HeldByTheOtherInstance,
    /// The data directory holds no such archive, and a read-only session
    /// creates none.
    MissingInAReadOnlySession,
}

impl ArchiveUnavailable {
    /// What a disabled control says after stating that the archive is
    /// unavailable this session.
    pub const fn explanation(self) -> &'static str {
        match self {
            Self::InterruptedDeleteLeftUnrecovered => {
                "an interrupted delete in it was left unrecovered, and nothing is written to it"
            }
            Self::HeldByTheOtherInstance => "the other GeoTrace has the file open",
            Self::MissingInAReadOnlySession => {
                "there is no such archive yet, and a read-only session creates none"
            }
        }
    }
}

/// Why the app has none of an archive this run. [`None`] where the archive is
/// open, or where it failed for a reason the user was never asked about.
pub type UnavailableArchives = PerArchive<Option<ArchiveUnavailable>>;

/// What an open does with one archive.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::app) enum ArchiveOpenPlan {
    /// Open it, creating it where it is not there, and recover an interrupted
    /// delete as the choice says.
    Open(InterruptedDeleteRecovery),
    /// Open the archive that is already there without writing to it.
    OpenReadOnly,
    /// Left closed for this session, for the reason the user was given.
    LeaveClosed(ArchiveUnavailable),
}

impl ArchiveOpenPlan {
    /// What a read-only session does with `archive`: it reads one that is
    /// already there, and creates none that is not.
    pub(in crate::app) fn in_a_read_only_session(
        archive: EnvironmentArchive,
        store: &Store,
    ) -> Self {
        if archive.path_in(store).exists() {
            Self::OpenReadOnly
        } else {
            Self::LeaveClosed(ArchiveUnavailable::MissingInAReadOnlySession)
        }
    }
}

/// What an open does with the interrupted deletes it meets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::app) enum ArchiveRecovery {
    /// Recover whatever is found: the process that left it behind is gone,
    /// and this instance has the data directory to itself.
    Automatic,
    /// Follow the user's choice for each archive after taking write access.
    AsTheUserChose(ArchiveRecoveryChoices),
}

impl ArchiveRecovery {
    pub(in crate::app) fn plan_for(self, archive: EnvironmentArchive) -> ArchiveOpenPlan {
        match self {
            Self::Automatic => ArchiveOpenPlan::Open(InterruptedDeleteRecovery::Recover),
            Self::AsTheUserChose(choices) => choices[archive],
        }
    }
}

/// The open plan the user chose for each archive.
pub(in crate::app) type ArchiveRecoveryChoices = PerArchive<ArchiveOpenPlan>;

/// The archives a taken-over open has yet to ask the user about, and the
/// choices it has so far.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::app) struct InterruptedDeletePrompts {
    root: Option<PathBuf>,
    without_a_choice: VecDeque<(EnvironmentArchive, InterruptedDeleteFinding)>,
    choices: ArchiveRecoveryChoices,
}

impl InterruptedDeletePrompts {
    /// Every archive starts out declining whatever interrupted delete it turns
    /// out to hold: one that appeared since the inspection read the archives
    /// is not recovered without a choice from the user.
    pub(in crate::app) fn asking_about(inspected: InspectedArchives) -> Self {
        let InspectedArchives { root, findings } = inspected;
        Self {
            root,
            without_a_choice: findings.into(),
            choices: ArchiveRecoveryChoices::filled_with(ArchiveOpenPlan::Open(
                InterruptedDeleteRecovery::Decline,
            )),
        }
    }

    /// The archive the open is asking the user about, or [`None`] once every
    /// one of them has a choice.
    fn being_asked_about(&self) -> Option<(EnvironmentArchive, InterruptedDeleteFinding)> {
        self.without_a_choice.front().copied()
    }

    fn record(&mut self, archive: EnvironmentArchive, choice: InterruptedDeleteChoice) {
        self.choices[archive] = choice.open_plan();
        self.without_a_choice.pop_front();
    }
}

/// What the user chose for one archive.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum InterruptedDeleteChoice {
    /// Discard the archived days and open the archive.
    Recover,
    /// Keep the file as it is, which leaves the archive closed.
    LeaveUnrecovered,
    /// Read the notice about an archive the other GeoTrace has open, which is
    /// not opened here.
    LeaveToTheOtherInstance,
}

impl InterruptedDeleteChoice {
    const fn open_plan(self) -> ArchiveOpenPlan {
        match self {
            Self::Recover => ArchiveOpenPlan::Open(InterruptedDeleteRecovery::Recover),
            Self::LeaveUnrecovered => {
                ArchiveOpenPlan::LeaveClosed(ArchiveUnavailable::InterruptedDeleteLeftUnrecovered)
            }
            Self::LeaveToTheOtherInstance => {
                ArchiveOpenPlan::LeaveClosed(ArchiveUnavailable::HeldByTheOtherInstance)
            }
        }
    }

    const fn log_line(self) -> &'static str {
        match self {
            Self::Recover => "recovering the interrupted delete, which discards its archived days",
            Self::LeaveUnrecovered => {
                "left unrecovered on the user's choice, and unavailable this session"
            }
            Self::LeaveToTheOtherInstance => {
                "held open by the other instance, and unavailable this session"
            }
        }
    }
}

/// Ask the user about the interrupted delete in `archive`, naming what each
/// choice costs.
///
/// Returns the choice in the frame the user makes it, and [`None`] while the
/// dialog is still open. Escape leaves the archive as it is: the choice that
/// discards nothing.
fn show_interrupted_delete_prompt(
    ui: &egui::Ui,
    archive: EnvironmentArchive,
    interrupted: InterruptedDelete,
    take_over: Option<TakeOverAfterTheArchiveWasLastWritten>,
) -> Option<InterruptedDeleteChoice> {
    modals::anchored_confirmation_dialog(
        ui.ctx(),
        AnchoredDialogKind::RecoverArchive,
        format!("Recover the {} archive?", archive.label_in_sentence()),
        InterruptedDeleteChoice::LeaveUnrecovered,
        |ui, _regions| {
            ui.label(
                "A delete was interrupted part-way through this archive, and GeoTrace cannot \
                 open it as it stands.",
            );
            if let Some(take_over) = take_over {
                ui.add_space(4.0);
                ui.label(take_over.prompt_sentence());
            }
            ui.add_space(6.0);
            ui.horizontal_wrapped(|ui| {
                ui.label(RichText::new(ICON_WARNING).color(warning_amber(ui.visuals().dark_mode)));
                ui.label(format!(
                    "Recovering discards the {} {} it holds. They are downloaded again as they \
                     are needed.",
                    interrupted.archived_days,
                    gt_fmt::pluralize(interrupted.archived_days, "archived day", "archived days")
                ));
            });
            ui.add_space(4.0);
            ui.label(
                "Leaving it unrecovered keeps the file exactly as it is. The archive is \
                 unavailable for this session and nothing is written to it.",
            );
        },
        |ui| {
            let mut choice = None;
            if modals::destructive_button(ui, RECOVER_BUTTON_LABEL).clicked() {
                choice = Some(InterruptedDeleteChoice::Recover);
            }
            if ui
                .button(LEAVE_UNRECOVERED_BUTTON_LABEL)
                .on_hover_text("The archive keeps its days and stays closed this session")
                .clicked()
            {
                choice = Some(InterruptedDeleteChoice::LeaveUnrecovered);
            }
            choice
        },
    )
}

/// Tell the user about an archive the other GeoTrace has open, which there is
/// nothing to choose about: it cannot be read here.
///
/// Returns the choice in the frame the user dismisses it, and [`None`] while
/// the dialog is still open.
fn show_archive_held_by_the_other_instance(
    ui: &egui::Ui,
    archive: EnvironmentArchive,
) -> Option<InterruptedDeleteChoice> {
    modals::anchored_confirmation_dialog(
        ui.ctx(),
        AnchoredDialogKind::ArchiveHeldByTheOtherInstance,
        format!("The {} archive is in use", archive.label_in_sentence()),
        InterruptedDeleteChoice::LeaveToTheOtherInstance,
        |ui, _regions| {
            ui.label(
                "GeoTrace cannot read this file here: the other GeoTrace still has it open. \
                 The archive is unavailable for this session and nothing is written to it.",
            );
        },
        |ui| {
            ui.button(ARCHIVE_IN_USE_BUTTON_LABEL)
                .clicked()
                .then_some(InterruptedDeleteChoice::LeaveToTheOtherInstance)
        },
    )
}

impl App {
    /// Take what the archive inspection found, and ask the user about each
    /// archive it found something in.
    ///
    /// An inspection that lands once the app is closing is left where it is:
    /// an app on its way out opens no database.
    pub(in crate::app) fn adopt_finished_archive_inspection(&mut self) {
        if self.shutdown.has_begun() {
            return;
        }
        let StorageOpen::InspectingArchives {
            inspected,
            queued_loads,
        } = &mut self.storage_open
        else {
            return;
        };
        let inspected = match inspected.try_recv() {
            Ok(inspected) => inspected,
            Err(mpsc::TryRecvError::Empty) => return,
            Err(mpsc::TryRecvError::Disconnected) => {
                log::error!(
                    "The archive inspection ended without reporting: the archives are opened \
                     without recovering anything"
                );
                InspectedArchives::unread_under(self.storage.root_to_open())
            }
        };
        self.storage_open = StorageOpen::AskingAboutInterruptedDeletes {
            prompts: InterruptedDeletePrompts::asking_about(inspected),
            queued_loads: mem::take(queued_loads),
        };
        let ctx = self.ctx.clone();
        self.open_the_archives_once_every_prompt_has_a_choice(&ctx);
    }

    /// Show the prompt for the archive the open is asking the user about, and
    /// start the open once the last one has a choice.
    pub(in crate::app) fn show_interrupted_delete_prompts(&mut self, ui: &egui::Ui) {
        if self.shutdown.has_begun() {
            return;
        }
        let StorageOpen::AskingAboutInterruptedDeletes { prompts, .. } = &mut self.storage_open
        else {
            return;
        };
        let Some((archive, finding)) = prompts.being_asked_about() else {
            return;
        };
        let choice = match finding {
            InterruptedDeleteFinding::Interrupted {
                interrupted,
                take_over,
            } => show_interrupted_delete_prompt(ui, archive, interrupted, take_over),
            InterruptedDeleteFinding::HeldByTheOtherInstance => {
                show_archive_held_by_the_other_instance(ui, archive)
            }
        };
        let Some(choice) = choice else {
            return;
        };
        log::warn!(
            "The {} archive is {}",
            archive.label_in_sentence(),
            choice.log_line()
        );
        prompts.record(archive, choice);
        self.open_the_archives_once_every_prompt_has_a_choice(ui.ctx());
    }

    /// Start the open the choices were gathered for, once no archive is left
    /// to ask the user about.
    fn open_the_archives_once_every_prompt_has_a_choice(&mut self, ctx: &egui::Context) {
        let StorageOpen::AskingAboutInterruptedDeletes {
            prompts,
            queued_loads,
        } = &mut self.storage_open
        else {
            return;
        };
        if prompts.being_asked_about().is_some() {
            return;
        }
        let queued_loads = mem::take(queued_loads);
        let root = prompts.root.take();
        let choices = prompts.choices;
        self.storage_open = storage::open_in_background_under(
            root,
            ArchiveRecovery::AsTheUserChose(choices),
            ctx,
            self.pending_writes.clone(),
            queued_loads,
        );
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use rstest::rstest;

    use super::*;

    const TAKEN_FROM_PROCESS_ID: u32 = 4321;

    /// A take-over of a data directory, stamped as the case says.
    const fn take_over_recorded_at(written_at: Option<u64>) -> TakeOverRecord {
        TakeOverRecord {
            taken_by_process_id: 1234,
            taken_from_process_id: Some(TAKEN_FROM_PROCESS_ID),
            written_at,
        }
    }

    /// When `path` was last written, in the unit a [`TakeOverRecord`] is
    /// stamped in.
    fn last_written_at(path: &Path) -> u64 {
        let modified = fs::metadata(path)
            .and_then(|metadata| metadata.modified())
            .expect("the file's modification time");
        gt_instance_lock::seconds_since_the_epoch(modified)
            .expect("a modification time after the Unix epoch")
    }

    /// A take-over the archive was written after explains nothing about the
    /// state that write left. A record without a timestamp cannot be
    /// compared with the modification time at all.
    #[rstest]
    #[case::recorded_after_the_last_write(Some(60), true)]
    #[case::recorded_before_the_last_write(Some(-60), false)]
    #[case::recorded_without_a_timestamp(None, true)]
    fn a_take_over_is_stated_unless_the_archive_was_written_after_it(
        #[case] seconds_after_the_last_write: Option<i64>,
        #[case] stated: bool,
    ) {
        let directory = tempfile::tempdir().expect("temp dir");
        let path = directory.path().join("interference.h5");
        fs::write(&path, b"an archive").expect("write the archive");
        let written_at = seconds_after_the_last_write
            .map(|offset| last_written_at(&path).saturating_add_signed(offset));

        let take_over = TakeOverAfterTheArchiveWasLastWritten::of_the_archive_at(
            &path,
            take_over_recorded_at(written_at),
        );

        assert_eq!(take_over.is_some(), stated);
    }

    /// `fs::metadata` reports no modification time for a file that is not
    /// there, which [`TakeOverAfterTheArchiveWasLastWritten::of_the_archive_at`]
    /// states the take-over for.
    #[test]
    fn a_take_over_is_stated_where_the_archive_has_no_modification_time() {
        let directory = tempfile::tempdir().expect("temp dir");

        let take_over = TakeOverAfterTheArchiveWasLastWritten::of_the_archive_at(
            &directory.path().join("interference.h5"),
            take_over_recorded_at(Some(1_700_000_000)),
        );

        assert!(take_over.is_some());
    }

    #[rstest]
    #[case::with_a_process_id_and_a_time(
        take_over_recorded_at(Some(1_700_000_000)),
        "Write access to this data directory was taken from another GeoTrace (process 4321) on \
         2023-11-14 22:13 UTC."
    )]
    #[case::without_a_time(
        take_over_recorded_at(None),
        "Write access to this data directory was taken from another GeoTrace (process 4321)."
    )]
    #[case::without_a_process_id(
        TakeOverRecord { taken_from_process_id: None, ..take_over_recorded_at(Some(1_700_000_000)) },
        "Write access to this data directory was taken from another GeoTrace on 2023-11-14 22:13 \
         UTC."
    )]
    fn the_prompt_states_the_take_over_with_what_the_record_holds(
        #[case] record: TakeOverRecord,
        #[case] expected: &str,
    ) {
        assert_eq!(
            TakeOverAfterTheArchiveWasLastWritten(record).prompt_sentence(),
            expected
        );
    }
}
