//! Answering the interrupted deletes the day archives hold, after the user
//! took write access from the instance holding the data directory.
//!
//! Recovering an interrupted delete discards every archived day of the
//! archive it is in. That is the right answer for a delete a process that is
//! gone left behind, and the wrong one for a delete the instance the user
//! just overrode may still be running. A taken-over open therefore asks per
//! archive rather than recovering on its own.
//!
//! The open therefore runs in two steps, with no write guard held while the
//! user decides: [`inspect_archives_under`] reads the archives and ends, the
//! prompts run in the frames after it, and the open in
//! [`super::storage::open_in`] follows with the answers.

use std::collections::VecDeque;
use std::fs;
use std::mem;
use std::path::{Path, PathBuf};
use std::sync::mpsc;

use chrono::DateTime;
use egui::{RichText, Window};
use egui_phosphor::regular::WARNING as ICON_WARNING;
use gt_instance_lock::TakeOverRecord;
use gt_store::{
    DayArchiveError, EnvironmentArchive, InterruptedDelete, InterruptedDeleteRecovery,
    ReadOnlyDayArchive as _, ReadOnlyFlareStore, ReadOnlyIonexStore, ReadOnlyJamStore,
    ReadOnlySolarStore, Store,
};
use gt_ui_theme::warning_amber;
use strum::IntoEnumIterator as _;

use super::storage::StorageOpen;
use super::{App, modals, storage};

pub(in crate::app) const RECOVER_BUTTON_LABEL: &str = "Recover";

pub(in crate::app) const LEAVE_UNRECOVERED_BUTTON_LABEL: &str = "Leave unrecovered";

pub(in crate::app) const ARCHIVE_IN_USE_BUTTON_LABEL: &str = "Continue";

const PROMPT_MAX_WIDTH: f32 = 460.0;

/// Starts the line an interrupted-delete prompt states a take-over on.
pub(in crate::app) const WRITE_ACCESS_TAKEN_FROM: &str =
    "Write access to this data directory was taken from";

/// What reading one archive found, where it is something the user has to
/// answer for.
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
            .map(|written_at| format!(" on {}", written_at.format("%Y-%m-%d %H:%M UTC")))
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
    /// The archives to ask about, in the order the settings rows list them.
    findings: Vec<(EnvironmentArchive, InterruptedDeleteFinding)>,
}

impl InspectedArchives {
    /// A run with nothing to read and nothing to ask about.
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
    let findings = EnvironmentArchive::iter()
        .filter_map(|archive| {
            Some((
                archive,
                InterruptedDeleteFinding::read_from_the_archive(
                    archive,
                    &store,
                    previous_take_over,
                )?,
            ))
        })
        .collect();
    InspectedArchives {
        root: Some(root),
        findings,
    }
}

impl InterruptedDeleteFinding {
    /// What `archive` holds for the user to answer, or [`None`] where the
    /// open needs no answer.
    ///
    /// A failure that is neither an interrupted delete nor another process
    /// holding the file is left to the open to report: it fails there the
    /// same way it does on the normal path.
    fn read_from_the_archive(
        archive: EnvironmentArchive,
        store: &Store,
        previous_take_over: Option<TakeOverRecord>,
    ) -> Option<Self> {
        let path = archive.path_in(store);
        let take_over = previous_take_over.and_then(|record| {
            TakeOverAfterTheArchiveWasLastWritten::of_the_archive_at(&path, record)
        });
        match archive {
            EnvironmentArchive::AircraftInterference => Self::from_inspection(
                archive,
                ReadOnlyJamStore::interrupted_delete_at(&path),
                take_over,
            ),
            EnvironmentArchive::GeomagneticIndices => Self::from_inspection(
                archive,
                ReadOnlySolarStore::interrupted_delete_at(&path),
                take_over,
            ),
            EnvironmentArchive::IonosphericTec => Self::from_inspection(
                archive,
                ReadOnlyIonexStore::interrupted_delete_at(&path),
                take_over,
            ),
            EnvironmentArchive::SolarFlares => Self::from_inspection(
                archive,
                ReadOnlyFlareStore::interrupted_delete_at(&path),
                take_over,
            ),
        }
    }

    fn from_inspection<E: DayArchiveError>(
        archive: EnvironmentArchive,
        inspected: Result<Option<InterruptedDelete>, E>,
        take_over: Option<TakeOverAfterTheArchiveWasLastWritten>,
    ) -> Option<Self> {
        match inspected {
            Ok(None) => None,
            Ok(Some(interrupted)) => Some(Self::Interrupted {
                interrupted,
                take_over,
            }),
            Err(err) if err.is_held_by_another_process() => Some(Self::HeldByTheOtherInstance),
            Err(err) => {
                log::debug!(
                    "Reading the {} archive for an interrupted delete failed, which the open \
                     reports: {err}",
                    archive.label_in_sentence()
                );
                None
            }
        }
    }
}

/// Why an archive is closed for this session, as the user was told when they
/// answered for it.
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

/// Why the app has none of an archive this run, where it has none.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct UnavailableArchives {
    interference: Option<ArchiveUnavailable>,
    geomagnetic_indices: Option<ArchiveUnavailable>,
    tec_maps: Option<ArchiveUnavailable>,
    solar_flares: Option<ArchiveUnavailable>,
}

impl UnavailableArchives {
    /// Why one archive is closed this session, or [`None`] where it is open
    /// or failed for a reason the user was never asked about.
    pub const fn of(self, archive: EnvironmentArchive) -> Option<ArchiveUnavailable> {
        match archive {
            EnvironmentArchive::AircraftInterference => self.interference,
            EnvironmentArchive::GeomagneticIndices => self.geomagnetic_indices,
            EnvironmentArchive::IonosphericTec => self.tec_maps,
            EnvironmentArchive::SolarFlares => self.solar_flares,
        }
    }

    pub(in crate::app) const fn record(
        &mut self,
        archive: EnvironmentArchive,
        reason: ArchiveUnavailable,
    ) {
        match archive {
            EnvironmentArchive::AircraftInterference => self.interference = Some(reason),
            EnvironmentArchive::GeomagneticIndices => self.geomagnetic_indices = Some(reason),
            EnvironmentArchive::IonosphericTec => self.tec_maps = Some(reason),
            EnvironmentArchive::SolarFlares => self.solar_flares = Some(reason),
        }
    }
}

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

/// How an open answers the interrupted deletes it meets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::app) enum ArchiveRecovery {
    /// Recover whatever is found: the process that left it behind is gone,
    /// and this instance has the data directory to itself.
    Automatic,
    /// Answer each archive the way the user did after taking write access.
    AsTheUserChose(ArchiveRecoveryAnswers),
}

impl ArchiveRecovery {
    pub(in crate::app) const fn plan_for(self, archive: EnvironmentArchive) -> ArchiveOpenPlan {
        match self {
            Self::Automatic => ArchiveOpenPlan::Open(InterruptedDeleteRecovery::Recover),
            Self::AsTheUserChose(answers) => answers.plan_for(archive),
        }
    }
}

/// What the user answered for each archive a taken-over open asked about.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::app) struct ArchiveRecoveryAnswers {
    interference: ArchiveOpenPlan,
    geomagnetic_indices: ArchiveOpenPlan,
    tec_maps: ArchiveOpenPlan,
    solar_flares: ArchiveOpenPlan,
}

impl Default for ArchiveRecoveryAnswers {
    /// An archive nobody was asked about opens declining whatever interrupted
    /// delete it turns out to hold: one that appeared since the inspection
    /// read the archives is not recovered behind the user's back.
    fn default() -> Self {
        let unasked = ArchiveOpenPlan::Open(InterruptedDeleteRecovery::Decline);
        Self {
            interference: unasked,
            geomagnetic_indices: unasked,
            tec_maps: unasked,
            solar_flares: unasked,
        }
    }
}

impl ArchiveRecoveryAnswers {
    const fn plan_for(self, archive: EnvironmentArchive) -> ArchiveOpenPlan {
        match archive {
            EnvironmentArchive::AircraftInterference => self.interference,
            EnvironmentArchive::GeomagneticIndices => self.geomagnetic_indices,
            EnvironmentArchive::IonosphericTec => self.tec_maps,
            EnvironmentArchive::SolarFlares => self.solar_flares,
        }
    }

    const fn record(&mut self, archive: EnvironmentArchive, plan: ArchiveOpenPlan) {
        match archive {
            EnvironmentArchive::AircraftInterference => self.interference = plan,
            EnvironmentArchive::GeomagneticIndices => self.geomagnetic_indices = plan,
            EnvironmentArchive::IonosphericTec => self.tec_maps = plan,
            EnvironmentArchive::SolarFlares => self.solar_flares = plan,
        }
    }
}

/// The archives a taken-over open has yet to ask about, and the answers it
/// has so far.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::app) struct InterruptedDeletePrompts {
    root: Option<PathBuf>,
    unanswered: VecDeque<(EnvironmentArchive, InterruptedDeleteFinding)>,
    answers: ArchiveRecoveryAnswers,
}

impl InterruptedDeletePrompts {
    pub(in crate::app) fn asking_about(inspected: InspectedArchives) -> Self {
        let InspectedArchives { root, findings } = inspected;
        Self {
            root,
            unanswered: findings.into(),
            answers: ArchiveRecoveryAnswers::default(),
        }
    }

    /// The archive the open is asking about, or [`None`] once every one of
    /// them has an answer.
    fn being_asked_about(&self) -> Option<(EnvironmentArchive, InterruptedDeleteFinding)> {
        self.unanswered.front().copied()
    }

    fn record(&mut self, archive: EnvironmentArchive, answer: InterruptedDeleteAnswer) {
        self.answers.record(archive, answer.open_plan());
        self.unanswered.pop_front();
    }
}

/// What the user answered about one archive.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum InterruptedDeleteAnswer {
    /// Discard the archived days and open the archive.
    Recover,
    /// Keep the file as it is, which leaves the archive closed.
    LeaveUnrecovered,
    /// Read the notice about an archive the other GeoTrace has open, which is
    /// not opened here.
    LeaveToTheOtherInstance,
}

impl InterruptedDeleteAnswer {
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

/// Ask about the interrupted delete in `archive`, naming what each answer
/// costs.
///
/// Returns the answer in the frame the user gives it, and [`None`] while the
/// dialog is still open. Escape leaves the archive as it is: the answer that
/// discards nothing.
fn show_interrupted_delete_prompt(
    ui: &egui::Ui,
    archive: EnvironmentArchive,
    interrupted: InterruptedDelete,
    take_over: Option<TakeOverAfterTheArchiveWasLastWritten>,
) -> Option<InterruptedDeleteAnswer> {
    let mut answer = ui
        .ctx()
        .input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape))
        .then_some(InterruptedDeleteAnswer::LeaveUnrecovered);

    let title = format!("Recover the {} archive?", archive.label_in_sentence());
    Window::new(title)
        .collapsible(false)
        .resizable(false)
        .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
        .show(ui.ctx(), |ui| {
            ui.set_max_width(PROMPT_MAX_WIDTH);
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
            ui.add_space(6.0);
            modals::dialog_button_row(ui, |ui| {
                if ui
                    .button(
                        RichText::new(RECOVER_BUTTON_LABEL)
                            .color(warning_amber(ui.visuals().dark_mode)),
                    )
                    .on_hover_text("This cannot be undone")
                    .clicked()
                {
                    answer = Some(InterruptedDeleteAnswer::Recover);
                }
                if ui
                    .button(LEAVE_UNRECOVERED_BUTTON_LABEL)
                    .on_hover_text("The archive keeps its days and stays closed this session")
                    .clicked()
                {
                    answer = Some(InterruptedDeleteAnswer::LeaveUnrecovered);
                }
            });
        });

    answer
}

/// Tell the user about an archive the other GeoTrace has open, which there is
/// nothing to choose about: it cannot be read here.
///
/// Returns the answer in the frame the user dismisses it, and [`None`] while
/// the dialog is still open.
fn show_archive_held_by_the_other_instance(
    ui: &egui::Ui,
    archive: EnvironmentArchive,
) -> Option<InterruptedDeleteAnswer> {
    let mut answer = ui
        .ctx()
        .input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape))
        .then_some(InterruptedDeleteAnswer::LeaveToTheOtherInstance);

    let title = format!("The {} archive is in use", archive.label_in_sentence());
    Window::new(title)
        .collapsible(false)
        .resizable(false)
        .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
        .show(ui.ctx(), |ui| {
            ui.set_max_width(PROMPT_MAX_WIDTH);
            ui.label(
                "GeoTrace cannot read this file here: the other GeoTrace still has it open. \
                 The archive is unavailable for this session and nothing is written to it.",
            );
            ui.add_space(6.0);
            modals::dialog_button_row(ui, |ui| {
                if ui.button(ARCHIVE_IN_USE_BUTTON_LABEL).clicked() {
                    answer = Some(InterruptedDeleteAnswer::LeaveToTheOtherInstance);
                }
            });
        });

    answer
}

impl App {
    /// Take what the archive inspection found, and ask about each archive it
    /// found something in.
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
        self.open_the_archives_once_every_prompt_is_answered(&ctx);
    }

    /// Show the prompt for the archive the open is asking about, and start
    /// the open once the last one is answered.
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
        let answer = match finding {
            InterruptedDeleteFinding::Interrupted {
                interrupted,
                take_over,
            } => show_interrupted_delete_prompt(ui, archive, interrupted, take_over),
            InterruptedDeleteFinding::HeldByTheOtherInstance => {
                show_archive_held_by_the_other_instance(ui, archive)
            }
        };
        let Some(answer) = answer else {
            return;
        };
        log::warn!(
            "The {} archive is {}",
            archive.label_in_sentence(),
            answer.log_line()
        );
        prompts.record(archive, answer);
        self.open_the_archives_once_every_prompt_is_answered(ui.ctx());
    }

    /// Start the open the answers were gathered for, once no archive is left
    /// to ask about.
    fn open_the_archives_once_every_prompt_is_answered(&mut self, ctx: &egui::Context) {
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
        let answers = prompts.answers;
        self.storage_open = storage::open_in_background_under(
            root,
            ArchiveRecovery::AsTheUserChose(answers),
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
