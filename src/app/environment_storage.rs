//! The day-keyed archives GeoTrace downloads for the days a recording spans:
//! what they hold, and deleting days from them.
//!
//! A delete runs off the UI thread: it rewrites every column of an archive,
//! which takes seconds on a filled TEC archive.

use std::sync::Arc;
use std::sync::mpsc;
use std::thread;

use chrono::{Months, NaiveDate, Utc};
use egui::Context;
use gt_pending_writes::{PendingWriteGuard, PendingWrites, WriteKind};
use gt_store::{
    ArchiveUsage, FlareStore, IonexStore, JamStore, PruneProgress, PruneProgressSink, SolarStore,
};
use strum::{EnumIter, IntoEnumIterator as _};

use super::App;
use super::day_fetch_queue::DayFetchQueue;
use super::modals::{
    EnvironmentPruneChoice, EnvironmentPrunePrompt, show_environment_prune_confirmation,
};

/// One of the archives, as the settings rows and the delete controls name it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, EnumIter)]
pub enum EnvironmentArchive {
    AircraftInterference,
    GeomagneticIndices,
    IonosphericTec,
    SolarFlares,
}

impl EnvironmentArchive {
    pub const fn label(self) -> &'static str {
        match self {
            Self::AircraftInterference => gt_jam::text::LAYER_LABEL,
            Self::GeomagneticIndices => "Geomagnetic indices",
            Self::IonosphericTec => "Ionospheric TEC",
            Self::SolarFlares => gt_flare::text::LAYER_LABEL,
        }
    }

    /// The label as it reads inside a sentence, where only an acronym keeps
    /// its capitals.
    pub const fn label_in_sentence(self) -> &'static str {
        match self {
            Self::AircraftInterference => "aircraft interference",
            Self::GeomagneticIndices => "geomagnetic indices",
            Self::IonosphericTec => "ionospheric TEC",
            Self::SolarFlares => "solar flares",
        }
    }

    /// Register the insert of one downloaded day, or [`None`] once shutdown
    /// has begun and the day is to be discarded.
    pub fn try_begin_day_insert(
        self,
        pending_writes: &PendingWrites,
        day: NaiveDate,
    ) -> Option<PendingWriteGuard> {
        pending_writes.try_begin(
            format!("Archiving {} for {day}", self.label_in_sentence()),
            WriteKind::ArchiveDayInsert {
                archive: self.label_in_sentence(),
            },
        )
    }

    /// Register the rewrite that deletes this archive's days, or [`None`]
    /// once shutdown has begun and the days stay archived.
    pub fn try_begin_day_delete(self, pending_writes: &PendingWrites) -> Option<PendingWriteGuard> {
        pending_writes.try_begin(
            format!("Deleting {} days", self.label_in_sentence()),
            WriteKind::ArchiveCompaction {
                archive: self.label_in_sentence(),
            },
        )
    }
}

/// The days a delete removed from an archive.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrunedDays {
    /// Every archived day before this one.
    Before(NaiveDate),
    /// Every day the archive held.
    All,
}

impl PrunedDays {
    pub fn covers(self, day: NaiveDate) -> bool {
        match self {
            Self::Before(cutoff) => day < cutoff,
            Self::All => true,
        }
    }

    /// How many of `days` a delete would remove.
    pub fn count_covered(self, days: impl IntoIterator<Item = NaiveDate>) -> usize {
        days.into_iter().filter(|day| self.covers(*day)).count()
    }
}

/// Which archives a delete acts on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PruneScope {
    Every,
    One(EnvironmentArchive),
}

impl PruneScope {
    pub fn covers(self, archive: EnvironmentArchive) -> bool {
        match self {
            Self::Every => true,
            Self::One(one) => one == archive,
        }
    }
}

/// One delete, as the settings page requests it and the dialog confirms it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PruneRequest {
    pub scope: PruneScope,
    pub days: PrunedDays,
}

/// The day an age-based auto-prune deletes everything before: the configured
/// age, or `oldest_needed_day` where that is earlier.
///
/// Deleting a day the schedulers still need would put it straight back on
/// their fetch queues.
pub fn auto_prune_cutoff(
    today: NaiveDate,
    max_age_months: u32,
    oldest_needed_day: Option<NaiveDate>,
) -> Option<NaiveDate> {
    let configured = today.checked_sub_months(Months::new(max_age_months))?;
    Some(oldest_needed_day.map_or(configured, |needed| configured.min(needed)))
}

/// How one archive's delete ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ArchivePruneOutcome {
    Removed(usize),
    /// What the archive said when its rewrite failed.
    Failed(String),
    /// The archive kept its days: shutdown began before the rewrite started.
    SkippedDuringShutdown,
}

impl ArchivePruneOutcome {
    /// How many days went, or [`None`] where the archive kept them.
    pub const fn days_removed(&self) -> Option<usize> {
        match self {
            Self::Removed(days) => Some(*days),
            Self::Failed(_) | Self::SkippedDuringShutdown => None,
        }
    }
}

/// What one archive's delete reported.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArchivePruneReport {
    pub archive: EnvironmentArchive,
    pub outcome: ArchivePruneOutcome,
}

/// What a whole delete reported, one entry per archive it acted on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PruneReport {
    pub request: PruneRequest,
    pub archives: Vec<ArchivePruneReport>,
}

impl PruneReport {
    /// What the delete removed, named per archive, or [`None`] where it
    /// removed nothing.
    pub fn removed_days_line(&self) -> Option<String> {
        let total = self.days_removed();
        if total == 0 {
            return None;
        }
        let per_archive: Vec<String> = self
            .archives
            .iter()
            .filter_map(|report| {
                let removed = report.outcome.days_removed()?;
                (removed > 0).then(|| format!("{removed} {}", report.archive.label_in_sentence()))
            })
            .collect();
        Some(format!(
            "Deleted {total} archived environment {}: {}",
            gt_fmt::pluralize(total, "day", "days"),
            per_archive.join(", ")
        ))
    }

    /// The archives shutdown stopped the delete from reaching, or [`None`]
    /// where it reached every one of them.
    pub fn skipped_during_shutdown_line(&self) -> Option<String> {
        let skipped: Vec<&str> = self
            .archives
            .iter()
            .filter(|report| report.outcome == ArchivePruneOutcome::SkippedDuringShutdown)
            .map(|report| report.archive.label_in_sentence())
            .collect();
        (!skipped.is_empty()).then(|| {
            format!(
                "Not deleting archived days from {}: shutting down",
                skipped.join(", ")
            )
        })
    }

    /// How many days went across the archives that succeeded.
    pub fn days_removed(&self) -> usize {
        self.archives
            .iter()
            .filter_map(|report| report.outcome.days_removed())
            .sum()
    }

    /// The archives whose delete failed, with what each reported.
    pub fn failures(&self) -> impl Iterator<Item = (EnvironmentArchive, &str)> {
        self.archives
            .iter()
            .filter_map(|report| match &report.outcome {
                ArchivePruneOutcome::Failed(detail) => Some((report.archive, detail.as_str())),
                ArchivePruneOutcome::Removed(_) | ArchivePruneOutcome::SkippedDuringShutdown => {
                    None
                }
            })
    }
}

/// The archives a delete acts on, taken from the schedulers that own them.
///
/// An archive that could not be opened is [`None`], and a delete skips it.
#[derive(Default, Clone)]
pub struct OpenEnvironmentArchives {
    pub interference: Option<Arc<JamStore>>,
    pub geomagnetic_indices: Option<Arc<SolarStore>>,
    pub tec_maps: Option<Arc<IonexStore>>,
    pub solar_flares: Option<Arc<FlareStore>>,
}

impl OpenEnvironmentArchives {
    /// The archives `scope` covers that are open, in the order the settings
    /// rows list them.
    fn covered_and_open(
        &self,
        scope: PruneScope,
    ) -> Vec<(EnvironmentArchive, &dyn ArchivedDayDeletion)> {
        EnvironmentArchive::iter()
            .filter(|archive| scope.covers(*archive))
            .filter_map(|archive| Some((archive, self.opened(archive)?)))
            .collect()
    }

    fn opened(&self, archive: EnvironmentArchive) -> Option<&dyn ArchivedDayDeletion> {
        match archive {
            EnvironmentArchive::AircraftInterference => Some(self.interference.as_deref()?),
            EnvironmentArchive::GeomagneticIndices => Some(self.geomagnetic_indices.as_deref()?),
            EnvironmentArchive::IonosphericTec => Some(self.tec_maps.as_deref()?),
            EnvironmentArchive::SolarFlares => Some(self.solar_flares.as_deref()?),
        }
    }
}

/// Deleting archived days, as each archive offers it. The errors differ per
/// archive, and a delete only reports them.
trait ArchivedDayDeletion {
    fn delete_pruned_days(
        &self,
        pruned: PrunedDays,
        report: PruneProgressSink<'_>,
    ) -> Result<usize, String>;

    /// Delete `pruned`, moving the shutdown window's bar for `write` along as
    /// the archive rewrites its columns.
    fn delete_pruned_days_reporting_progress(
        &self,
        pruned: PrunedDays,
        write: &PendingWriteGuard,
    ) -> ArchivePruneOutcome {
        let report = |progress: PruneProgress| write.set_progress(progress.fraction());
        match self.delete_pruned_days(pruned, Some(&report)) {
            Ok(removed) => ArchivePruneOutcome::Removed(removed),
            Err(detail) => ArchivePruneOutcome::Failed(detail),
        }
    }
}

impl ArchivedDayDeletion for JamStore {
    fn delete_pruned_days(
        &self,
        pruned: PrunedDays,
        report: PruneProgressSink<'_>,
    ) -> Result<usize, String> {
        match pruned {
            PrunedDays::Before(cutoff) => self.delete_days_before(cutoff, report),
            PrunedDays::All => self.delete_all_days(report),
        }
        .map_err(|err| err.to_string())
    }
}

impl ArchivedDayDeletion for SolarStore {
    fn delete_pruned_days(
        &self,
        pruned: PrunedDays,
        report: PruneProgressSink<'_>,
    ) -> Result<usize, String> {
        match pruned {
            PrunedDays::Before(cutoff) => self.delete_days_before(cutoff, report),
            PrunedDays::All => self.delete_all_days(report),
        }
        .map_err(|err| err.to_string())
    }
}

impl ArchivedDayDeletion for IonexStore {
    fn delete_pruned_days(
        &self,
        pruned: PrunedDays,
        report: PruneProgressSink<'_>,
    ) -> Result<usize, String> {
        match pruned {
            PrunedDays::Before(cutoff) => self.delete_days_before(cutoff, report),
            PrunedDays::All => self.delete_all_days(report),
        }
        .map_err(|err| err.to_string())
    }
}

impl ArchivedDayDeletion for FlareStore {
    fn delete_pruned_days(
        &self,
        pruned: PrunedDays,
        report: PruneProgressSink<'_>,
    ) -> Result<usize, String> {
        match pruned {
            PrunedDays::Before(cutoff) => self.delete_days_before(cutoff, report),
            PrunedDays::All => self.delete_all_days(report),
        }
        .map_err(|err| err.to_string())
    }
}

/// A delete running off the UI thread.
#[derive(Default)]
pub struct EnvironmentPruneRun {
    running: Option<mpsc::Receiver<PruneReport>>,
}

impl EnvironmentPruneRun {
    pub const fn is_running(&self) -> bool {
        self.running.is_some()
    }

    /// Start `request` against `archives`, replacing nothing: a caller starts
    /// one only while none is running.
    #[expect(
        clippy::expect_used,
        reason = "thread spawn can only fail under extreme system resource exhaustion"
    )]
    pub fn start(
        &mut self,
        ctx: Context,
        archives: OpenEnvironmentArchives,
        pending_writes: PendingWrites,
        request: PruneRequest,
    ) {
        let (tx, rx) = mpsc::channel();
        self.running = Some(rx);
        thread::Builder::new()
            .name("environment-prune".to_owned())
            .spawn(move || {
                tx.send(prune(&archives, &pending_writes, request)).ok();
                ctx.request_repaint();
            })
            .expect("failed to spawn the environment prune thread");
    }

    /// What the running delete reported, once it has finished.
    ///
    /// A worker that died without reporting frees the run: leaving it in place
    /// would gray the controls for the rest of the session.
    pub fn take_finished(&mut self) -> Option<PruneReport> {
        match self.running.as_ref()?.try_recv() {
            Ok(report) => {
                self.running = None;
                Some(report)
            }
            Err(mpsc::TryRecvError::Empty) => None,
            Err(mpsc::TryRecvError::Disconnected) => {
                log::error!("The environment data delete ended without reporting what it did");
                self.running = None;
                None
            }
        }
    }
}

/// Delete from every archive the request covers, in turn.
///
/// A rewrite that has begun runs to its end even once shutdown starts: the
/// archive is unreadable until it finishes.
fn prune(
    archives: &OpenEnvironmentArchives,
    pending_writes: &PendingWrites,
    request: PruneRequest,
) -> PruneReport {
    let covered = archives.covered_and_open(request.scope);
    let mut reports = Vec::with_capacity(covered.len());
    for (index, (archive, store)) in covered.iter().enumerate() {
        let Some(write) = archive.try_begin_day_delete(pending_writes) else {
            reports.extend(
                covered
                    .iter()
                    .skip(index)
                    .map(|(archive, _)| ArchivePruneReport {
                        archive: *archive,
                        outcome: ArchivePruneOutcome::SkippedDuringShutdown,
                    }),
            );
            break;
        };
        reports.push(ArchivePruneReport {
            archive: *archive,
            outcome: store.delete_pruned_days_reporting_progress(request.days, &write),
        });
    }
    PruneReport {
        request,
        archives: reports,
    }
}

/// How many days each archive holds inside a delete's range.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CoveredDayCounts {
    pub interference: usize,
    pub geomagnetic_indices: usize,
    pub tec_maps: usize,
    pub solar_flares: usize,
}

impl CoveredDayCounts {
    pub const fn of(&self, archive: EnvironmentArchive) -> usize {
        match archive {
            EnvironmentArchive::AircraftInterference => self.interference,
            EnvironmentArchive::GeomagneticIndices => self.geomagnetic_indices,
            EnvironmentArchive::IonosphericTec => self.tec_maps,
            EnvironmentArchive::SolarFlares => self.solar_flares,
        }
    }

    pub fn total(&self) -> usize {
        EnvironmentArchive::iter()
            .map(|archive| self.of(archive))
            .sum()
    }
}

/// What every archive holds, as the settings rows show it.
#[derive(Debug, Clone, Copy, Default)]
pub struct EnvironmentUsage {
    pub interference: Option<ArchiveUsage>,
    pub geomagnetic_indices: Option<ArchiveUsage>,
    pub tec_maps: Option<ArchiveUsage>,
    pub solar_flares: Option<ArchiveUsage>,
}

impl EnvironmentUsage {
    /// What one archive holds, or [`None`] where it could not be opened.
    pub const fn of(&self, archive: EnvironmentArchive) -> Option<ArchiveUsage> {
        match archive {
            EnvironmentArchive::AircraftInterference => self.interference,
            EnvironmentArchive::GeomagneticIndices => self.geomagnetic_indices,
            EnvironmentArchive::IonosphericTec => self.tec_maps,
            EnvironmentArchive::SolarFlares => self.solar_flares,
        }
    }

    /// Every archive added up, or [`None`] while none of them could be
    /// opened.
    pub fn total(&self) -> Option<ArchiveUsage> {
        let opened: Vec<ArchiveUsage> = EnvironmentArchive::iter()
            .filter_map(|archive| self.of(archive))
            .collect();
        (!opened.is_empty()).then(|| ArchiveUsage::total(opened))
    }
}

impl App {
    /// The archives the environment storage rows report on and the delete acts
    /// on, taken from the schedulers that own them.
    fn open_environment_archives(&self) -> OpenEnvironmentArchives {
        OpenEnvironmentArchives {
            interference: self.jamming.archive(),
            geomagnetic_indices: self.geomagnetic_indices.archive(),
            tec_maps: self.tec_maps.archive(),
            solar_flares: self.solar_flares.archive(),
        }
    }

    /// What every environment archive holds on disk.
    pub(super) fn environment_usage(&self) -> EnvironmentUsage {
        EnvironmentUsage {
            interference: self.jamming.archive_usage(),
            geomagnetic_indices: self.geomagnetic_indices.archive_usage(),
            tec_maps: self.tec_maps.archive_usage(),
            solar_flares: self.solar_flares.archive_usage(),
        }
    }

    /// How many days each archive would lose to a delete of `pruned`.
    pub(super) fn environment_days_covered(&self, pruned: PrunedDays) -> CoveredDayCounts {
        CoveredDayCounts {
            interference: self.jamming.archived_days_covered(pruned),
            geomagnetic_indices: self.geomagnetic_indices.archived_days_covered(pruned),
            tec_maps: self.tec_maps.archived_days_covered(pruned),
            solar_flares: self.solar_flares.archived_days_covered(pruned),
        }
    }

    /// The loaded recordings spanning a day the delete removes, named as the
    /// rest of the app names them.
    pub(super) fn recordings_spanning_pruned_days(&self, pruned: PrunedDays) -> Vec<String> {
        let shared = self.shared.borrow();
        let names = gt_loaded_files::RecordingNames::resolve(
            shared.loaded_files.view(),
            &shared.recording_name_template,
        );
        shared
            .loaded_files
            .iter()
            .enumerate()
            .filter(|(_, file)| {
                file.tracks
                    .iter()
                    .any(|track| pruned.covers(track.metadata.time_range.start.date_naive()))
            })
            .filter_map(|(index, file)| {
                names
                    .get(gt_types::FileIdx::new(index))
                    .map(str::to_owned)
                    .or_else(|| Some(file.metadata.filename.clone()))
            })
            .collect()
    }

    /// The earliest day the fetch schedulers still count for the recordings
    /// loaded this session.
    fn oldest_needed_environment_day(&self) -> Option<NaiveDate> {
        [
            self.jamming.fetch_queue(),
            self.geomagnetic_indices.fetch_queue(),
            self.tec_maps.fetch_queue(),
            self.solar_flares.fetch_queue(),
        ]
        .into_iter()
        .filter_map(DayFetchQueue::oldest_needed_day)
        .min()
    }

    /// The delete the age-based auto-prune would start now, or [`None`] while
    /// it is off, a delete is running, or no archive holds a day that old.
    pub(super) fn environment_auto_prune_request(&self) -> Option<PruneRequest> {
        let settings = self.environment_storage_settings;
        if !settings.auto_prune_enabled || self.environment_prune.is_running() {
            return None;
        }
        let days = PrunedDays::Before(auto_prune_cutoff(
            Utc::now().date_naive(),
            settings.auto_prune_max_age_months,
            self.oldest_needed_environment_day(),
        )?);
        (self.environment_days_covered(days).total() > 0).then_some(PruneRequest {
            scope: PruneScope::Every,
            days,
        })
    }

    /// Delete the archived days past the configured age, without a
    /// confirmation: the age is the user's own standing choice.
    pub(super) fn auto_prune_environment_days(&mut self) {
        let Some(request) = self.environment_auto_prune_request() else {
            return;
        };
        let ctx = self.ctx.clone();
        self.start_environment_prune(&ctx, request);
    }

    /// Whether a delete is running, which grays the controls that start one.
    pub(super) const fn environment_prune_running(&self) -> bool {
        self.environment_prune.is_running()
    }

    /// Start the delete the user confirmed.
    pub(super) fn start_environment_prune(&mut self, ctx: &egui::Context, request: PruneRequest) {
        if self.environment_prune.is_running() {
            return;
        }
        if self.pending_writes.is_shutting_down() {
            log::debug!("Not deleting archived environment days: shutting down");
            return;
        }
        let archives = self.open_environment_archives();
        self.environment_prune
            .start(ctx.clone(), archives, self.pending_writes.clone(), request);
    }

    /// Apply a finished delete: report it, and put the schedulers back on the
    /// days it removed.
    pub(super) fn poll_environment_prune(&mut self) {
        let Some(report) = self.environment_prune.take_finished() else {
            return;
        };
        for (archive, detail) in report.failures() {
            log::error!(
                "Deleting archived days of {} failed: {detail}",
                archive.label()
            );
            self.toasts.error(format!("{}: {detail}", archive.label()));
        }
        if let Some(line) = report.skipped_during_shutdown_line() {
            log::info!("{line}");
        }
        self.forget_pruned_environment_days(&report);
        self.request_environment_days_of_loaded_recordings();

        if let Some(line) = report.removed_days_line() {
            log::info!("{line}");
            self.toasts
                .info(line)
                .duration(Some(std::time::Duration::from_secs(4)));
        }
    }

    /// Drop what the schedulers hold for the days the delete removed.
    fn forget_pruned_environment_days(&mut self, report: &PruneReport) {
        let pruned = report.request.days;
        for archive in report
            .archives
            .iter()
            .filter(|one| one.outcome.days_removed().is_some())
        {
            match archive.archive {
                EnvironmentArchive::AircraftInterference => {
                    self.jamming.forget_pruned_days(pruned);
                }
                EnvironmentArchive::GeomagneticIndices => {
                    self.geomagnetic_indices.forget_pruned_days(pruned);
                }
                EnvironmentArchive::IonosphericTec => {
                    self.tec_maps.forget_pruned_days(pruned);
                }
                EnvironmentArchive::SolarFlares => {
                    self.solar_flares.forget_pruned_days(pruned);
                }
            }
        }
    }

    /// Queue the days `range` spans in every archive. Called when a recording
    /// loads and when a delete finishes.
    pub(super) fn request_environment_days_for(&mut self, range: gt_types::TimeRange) {
        self.jamming.request_days_for(range);
        self.geomagnetic_indices.request_days_for(range);
        self.tec_maps.request_days_for(range);
        self.solar_flares.request_days_for(range);
    }

    /// Queue the days every loaded recording spans again. The days a delete
    /// removed go back on the fetch queues here.
    fn request_environment_days_of_loaded_recordings(&mut self) {
        let ranges: Vec<gt_types::TimeRange> = self
            .shared
            .borrow()
            .loaded_files
            .iter()
            .flat_map(|file| file.tracks.iter())
            .map(|track| track.metadata.time_range)
            .collect();
        for range in ranges {
            self.request_environment_days_for(range);
        }
    }

    /// Confirm the environment-data delete the settings page requested, and
    /// start it once the user says so.
    pub(super) fn show_environment_prune_prompt(&mut self, ui: &egui::Ui) {
        let Some(request) = self.pending_environment_prune else {
            return;
        };
        let prompt = EnvironmentPrunePrompt {
            request,
            covered: self.environment_days_covered(request.days),
            loaded_recordings: &self.recordings_spanning_pruned_days(request.days),
        };
        match show_environment_prune_confirmation(ui, &prompt) {
            Some(EnvironmentPruneChoice::Delete) => {
                self.pending_environment_prune = None;
                self.start_environment_prune(ui.ctx(), request);
            }
            Some(EnvironmentPruneChoice::Cancel) => self.pending_environment_prune = None,
            None => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use chrono::{TimeDelta, Utc};
    use rstest::rstest;

    use super::*;

    fn day(offset: i64) -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 7, 20).unwrap_or_default() + TimeDelta::days(offset)
    }

    /// An interference archive holding three days, and nothing else opened.
    fn archives_with_interference() -> (tempfile::TempDir, OpenEnvironmentArchives) {
        let dir = tempfile::tempdir().expect("temp dir");
        let store = gt_store::Store::open_in(dir.path())
            .open_interference()
            .expect("open the archive");
        for offset in 0..3 {
            store
                .insert_day(day(offset), "host", Utc::now(), &[])
                .expect("insert");
        }
        let archives = OpenEnvironmentArchives {
            interference: Some(store),
            ..OpenEnvironmentArchives::default()
        };
        (dir, archives)
    }

    /// The archives of [`archives_with_interference`], with a solar flare
    /// archive holding three days beside them.
    fn archives_with_interference_and_solar_flares() -> (tempfile::TempDir, OpenEnvironmentArchives)
    {
        let (dir, mut archives) = archives_with_interference();
        let store = gt_store::Store::open_in(dir.path())
            .open_solar_flares()
            .expect("open the archive");
        for offset in 0..3 {
            store
                .insert_or_replace_day(day(offset), "host", Utc::now(), &[])
                .expect("insert");
        }
        archives.solar_flares = Some(store);
        (dir, archives)
    }

    fn archived_interference_days(archives: &OpenEnvironmentArchives) -> usize {
        archives
            .interference
            .as_ref()
            .expect("the archive is open")
            .days()
            .expect("days")
            .len()
    }

    fn ymd(year: i32, month: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(year, month, day).unwrap_or_default()
    }

    /// The configured age decides the cutoff, except where the schedulers
    /// still need older days.
    #[rstest]
    #[case::nothing_loaded(None, ymd(2025, 8, 21))]
    #[case::a_recording_older_than_the_age(Some(ymd(2024, 3, 2)), ymd(2024, 3, 2))]
    #[case::a_recording_newer_than_the_age(Some(ymd(2026, 6, 4)), ymd(2025, 8, 21))]
    #[case::a_quiet_time_window_reaching_past_the_age(Some(ymd(2025, 7, 26)), ymd(2025, 7, 26))]
    fn the_auto_prune_cutoff_keeps_the_days_the_schedulers_need(
        #[case] oldest_needed_day: Option<NaiveDate>,
        #[case] expected: NaiveDate,
    ) {
        assert_eq!(
            auto_prune_cutoff(ymd(2026, 8, 21), 12, oldest_needed_day),
            Some(expected)
        );
    }

    #[rstest]
    #[case::before_the_cutoff(PrunedDays::Before(day(1)), day(0), true)]
    #[case::on_the_cutoff(PrunedDays::Before(day(1)), day(1), false)]
    #[case::after_the_cutoff(PrunedDays::Before(day(1)), day(2), false)]
    #[case::every_day(PrunedDays::All, day(9), true)]
    fn a_delete_covers_the_days_it_removes(
        #[case] pruned: PrunedDays,
        #[case] day: NaiveDate,
        #[case] covered: bool,
    ) {
        assert_eq!(pruned.covers(day), covered);
    }

    /// A delete over every archive reports only the ones that are open: an
    /// archive that could not be opened has no days to lose.
    #[test]
    fn a_delete_reports_the_archives_it_reached() {
        let (_dir, archives) = archives_with_interference();

        let report = prune(
            &archives,
            &PendingWrites::default(),
            PruneRequest {
                scope: PruneScope::Every,
                days: PrunedDays::Before(day(2)),
            },
        );

        assert_eq!(report.archives.len(), 1);
        assert_eq!(report.days_removed(), 2);
        assert_eq!(report.failures().count(), 0);
        assert_eq!(
            report.archives.first().map(|archive| archive.archive),
            Some(EnvironmentArchive::AircraftInterference)
        );
    }

    /// A delete of one archive leaves the others alone, even where they are
    /// open.
    #[test]
    fn a_delete_of_one_archive_reaches_no_other() {
        let (_dir, archives) = archives_with_interference();

        let report = prune(
            &archives,
            &PendingWrites::default(),
            PruneRequest {
                scope: PruneScope::One(EnvironmentArchive::SolarFlares),
                days: PrunedDays::All,
            },
        );

        assert!(report.archives.is_empty());
        assert_eq!(report.days_removed(), 0);
        assert_eq!(
            archived_interference_days(&archives),
            3,
            "another archive's delete took days from the interference archive"
        );
    }

    /// An archive that reports its rewrite moves the bar the shutdown window
    /// draws for that delete.
    #[test]
    fn a_reported_rewrite_moves_the_write_guard_along() {
        struct ReportingArchive;

        impl ArchivedDayDeletion for ReportingArchive {
            fn delete_pruned_days(
                &self,
                _pruned: PrunedDays,
                report: PruneProgressSink<'_>,
            ) -> Result<usize, String> {
                for columns_rewritten in 0..=4 {
                    if let Some(report) = report {
                        report(PruneProgress {
                            columns_rewritten,
                            columns_total: 4,
                        });
                    }
                }
                Ok(1)
            }
        }

        let pending_writes = PendingWrites::default();
        let write = EnvironmentArchive::IonosphericTec
            .try_begin_day_delete(&pending_writes)
            .expect("the registry is running");

        let outcome =
            ReportingArchive.delete_pruned_days_reporting_progress(PrunedDays::All, &write);

        assert_eq!(outcome, ArchivePruneOutcome::Removed(1));
        assert_eq!(
            pending_writes
                .snapshot()
                .running
                .first()
                .and_then(|status| status.progress),
            Some(1.0),
            "the guard did not follow the rewrite"
        );
    }

    /// Shutdown stops the delete before it rewrites anything, and every
    /// archive it would have reached is reported as skipped.
    #[test]
    fn a_delete_started_during_shutdown_leaves_every_archive_alone() {
        let (_dir, archives) = archives_with_interference_and_solar_flares();
        let pending_writes = PendingWrites::default();
        pending_writes.begin_shutdown();

        let report = prune(
            &archives,
            &pending_writes,
            PruneRequest {
                scope: PruneScope::Every,
                days: PrunedDays::All,
            },
        );

        assert_eq!(
            report.archives,
            vec![
                skipped(EnvironmentArchive::AircraftInterference),
                skipped(EnvironmentArchive::SolarFlares),
            ]
        );
        assert_eq!(report.days_removed(), 0);
        assert_eq!(report.failures().count(), 0);
        assert_eq!(archived_interference_days(&archives), 3);
    }

    /// A delete keeps the process from closing underneath it, and lets go of
    /// it once the archive is rewritten.
    #[test]
    fn a_finished_delete_is_registered_while_it_runs() {
        let (_dir, archives) = archives_with_interference();
        let pending_writes = PendingWrites::default();

        prune(
            &archives,
            &pending_writes,
            PruneRequest {
                scope: PruneScope::Every,
                days: PrunedDays::All,
            },
        );

        assert!(pending_writes.is_idle());
        assert_eq!(
            pending_writes.snapshot().recently_finished,
            ["Deleting aircraft interference days"]
        );
    }

    fn removed(archive: EnvironmentArchive, days: usize) -> ArchivePruneReport {
        ArchivePruneReport {
            archive,
            outcome: ArchivePruneOutcome::Removed(days),
        }
    }

    fn failed(archive: EnvironmentArchive) -> ArchivePruneReport {
        ArchivePruneReport {
            archive,
            outcome: ArchivePruneOutcome::Failed("archive is inconsistent".to_owned()),
        }
    }

    fn skipped(archive: EnvironmentArchive) -> ArchivePruneReport {
        ArchivePruneReport {
            archive,
            outcome: ArchivePruneOutcome::SkippedDuringShutdown,
        }
    }

    /// The line states the total and then every archive that lost days.
    #[rstest]
    #[case::one_archive(
        vec![removed(EnvironmentArchive::AircraftInterference, 12)],
        Some("Deleted 12 archived environment days: 12 aircraft interference")
    )]
    #[case::several_archives(
        vec![
            removed(EnvironmentArchive::AircraftInterference, 12),
            removed(EnvironmentArchive::IonosphericTec, 14),
            removed(EnvironmentArchive::SolarFlares, 14),
        ],
        Some(
            "Deleted 40 archived environment days: 12 aircraft interference, \
             14 ionospheric TEC, 14 solar flares"
        )
    )]
    #[case::a_single_day(
        vec![removed(EnvironmentArchive::AircraftInterference, 1)],
        Some("Deleted 1 archived environment day: 1 aircraft interference")
    )]
    #[case::an_archive_that_lost_nothing(
        vec![
            removed(EnvironmentArchive::AircraftInterference, 3),
            removed(EnvironmentArchive::GeomagneticIndices, 0),
        ],
        Some("Deleted 3 archived environment days: 3 aircraft interference")
    )]
    #[case::nothing_removed(
        vec![
            removed(EnvironmentArchive::AircraftInterference, 0),
            failed(EnvironmentArchive::IonosphericTec),
        ],
        None
    )]
    fn the_removal_line_names_the_archives_that_lost_days(
        #[case] archives: Vec<ArchivePruneReport>,
        #[case] expected: Option<&str>,
    ) {
        let report = PruneReport {
            request: PruneRequest {
                scope: PruneScope::Every,
                days: PrunedDays::All,
            },
            archives,
        };

        assert_eq!(report.removed_days_line().as_deref(), expected);
    }

    /// The line names every archive the delete did not reach, and nothing at
    /// all where it reached all of them.
    #[rstest]
    #[case::every_archive_reached(vec![removed(EnvironmentArchive::AircraftInterference, 3)], None)]
    #[case::two_archives_skipped(
        vec![
            removed(EnvironmentArchive::AircraftInterference, 3),
            skipped(EnvironmentArchive::GeomagneticIndices),
            skipped(EnvironmentArchive::IonosphericTec),
        ],
        Some(
            "Not deleting archived days from geomagnetic indices, ionospheric TEC: \
             shutting down"
        )
    )]
    fn the_skipped_line_names_the_archives_that_kept_their_days(
        #[case] archives: Vec<ArchivePruneReport>,
        #[case] expected: Option<&str>,
    ) {
        let report = PruneReport {
            request: PruneRequest {
                scope: PruneScope::Every,
                days: PrunedDays::All,
            },
            archives,
        };

        assert_eq!(report.skipped_during_shutdown_line().as_deref(), expected);
    }

    /// The sentence form names the same archive as the settings row.
    #[test]
    fn every_archive_reads_the_same_inside_a_sentence() {
        for archive in EnvironmentArchive::iter() {
            assert_eq!(
                archive.label_in_sentence().to_lowercase(),
                archive.label().to_lowercase()
            );
        }
    }

    #[test]
    fn a_failed_delete_is_reported_with_what_it_said_and_a_skipped_one_is_not() {
        let report = PruneReport {
            request: PruneRequest {
                scope: PruneScope::Every,
                days: PrunedDays::All,
            },
            archives: vec![
                removed(EnvironmentArchive::AircraftInterference, 4),
                failed(EnvironmentArchive::IonosphericTec),
                skipped(EnvironmentArchive::SolarFlares),
            ],
        };

        assert_eq!(report.days_removed(), 4);
        assert_eq!(
            report
                .failures()
                .collect::<Vec<(EnvironmentArchive, &str)>>(),
            [(
                EnvironmentArchive::IonosphericTec,
                "archive is inconsistent"
            )]
        );
    }
}
