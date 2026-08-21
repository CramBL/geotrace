//! The day-keyed archives GeoTrace downloads for the days a recording spans:
//! what they hold, and deleting days from them.
//!
//! A delete runs off the UI thread: it rewrites every column of an archive,
//! which takes seconds on a filled TEC archive.

use std::sync::Arc;
use std::sync::mpsc;
use std::thread;

use chrono::NaiveDate;
use egui::Context;
use gt_store::{ArchiveUsage, FlareStore, IonexStore, JamStore, SolarStore};
use strum::{EnumIter, IntoEnumIterator as _};

use super::App;
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

/// What one archive's delete reported.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArchivePruneReport {
    pub archive: EnvironmentArchive,
    /// How many days went, or why none did.
    pub outcome: Result<usize, String>,
}

/// What a whole delete reported, one entry per archive it acted on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PruneReport {
    pub request: PruneRequest,
    pub archives: Vec<ArchivePruneReport>,
}

impl PruneReport {
    /// How many days went across the archives that succeeded.
    pub fn days_removed(&self) -> usize {
        self.archives
            .iter()
            .filter_map(|report| report.outcome.as_ref().ok())
            .sum()
    }

    /// The archives whose delete failed, with what each reported.
    pub fn failures(&self) -> impl Iterator<Item = (EnvironmentArchive, &str)> {
        self.archives.iter().filter_map(|report| {
            report
                .outcome
                .as_ref()
                .err()
                .map(|detail| (report.archive, detail.as_str()))
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

/// Deleting archived days, as each archive offers it. The errors differ per
/// archive, and a delete only reports them.
trait ArchivedDayDeletion {
    fn delete_pruned_days(&self, pruned: PrunedDays) -> Result<usize, String>;
}

impl ArchivedDayDeletion for JamStore {
    fn delete_pruned_days(&self, pruned: PrunedDays) -> Result<usize, String> {
        match pruned {
            PrunedDays::Before(cutoff) => self.delete_days_before(cutoff),
            PrunedDays::All => self.delete_all_days(),
        }
        .map_err(|err| err.to_string())
    }
}

impl ArchivedDayDeletion for SolarStore {
    fn delete_pruned_days(&self, pruned: PrunedDays) -> Result<usize, String> {
        match pruned {
            PrunedDays::Before(cutoff) => self.delete_days_before(cutoff),
            PrunedDays::All => self.delete_all_days(),
        }
        .map_err(|err| err.to_string())
    }
}

impl ArchivedDayDeletion for IonexStore {
    fn delete_pruned_days(&self, pruned: PrunedDays) -> Result<usize, String> {
        match pruned {
            PrunedDays::Before(cutoff) => self.delete_days_before(cutoff),
            PrunedDays::All => self.delete_all_days(),
        }
        .map_err(|err| err.to_string())
    }
}

impl ArchivedDayDeletion for FlareStore {
    fn delete_pruned_days(&self, pruned: PrunedDays) -> Result<usize, String> {
        match pruned {
            PrunedDays::Before(cutoff) => self.delete_days_before(cutoff),
            PrunedDays::All => self.delete_all_days(),
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
        request: PruneRequest,
    ) {
        let (tx, rx) = mpsc::channel();
        self.running = Some(rx);
        thread::Builder::new()
            .name("environment-prune".to_owned())
            .spawn(move || {
                tx.send(prune(&archives, request)).ok();
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
fn prune(archives: &OpenEnvironmentArchives, request: PruneRequest) -> PruneReport {
    let mut reports = Vec::new();
    for archive in EnvironmentArchive::iter().filter(|one| request.scope.covers(*one)) {
        let outcome = match archive {
            EnvironmentArchive::AircraftInterference => archives
                .interference
                .as_deref()
                .map(|store| store.delete_pruned_days(request.days)),
            EnvironmentArchive::GeomagneticIndices => archives
                .geomagnetic_indices
                .as_deref()
                .map(|store| store.delete_pruned_days(request.days)),
            EnvironmentArchive::IonosphericTec => archives
                .tec_maps
                .as_deref()
                .map(|store| store.delete_pruned_days(request.days)),
            EnvironmentArchive::SolarFlares => archives
                .solar_flares
                .as_deref()
                .map(|store| store.delete_pruned_days(request.days)),
        };
        if let Some(outcome) = outcome {
            reports.push(ArchivePruneReport { archive, outcome });
        }
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

    /// Whether a delete is running, which grays the controls that start one.
    pub(super) const fn environment_prune_running(&self) -> bool {
        self.environment_prune.is_running()
    }

    /// Start the delete the user confirmed.
    pub(super) fn start_environment_prune(&mut self, ctx: &egui::Context, request: PruneRequest) {
        if self.environment_prune.is_running() {
            return;
        }
        let archives = self.open_environment_archives();
        self.environment_prune.start(ctx.clone(), archives, request);
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
        self.forget_pruned_environment_days(&report);
        self.request_environment_days_of_loaded_recordings();

        let removed = report.days_removed();
        if removed > 0 {
            let days = gt_fmt::pluralize(removed, "day", "days");
            self.toasts
                .info(format!("Deleted {removed} archived {days}"))
                .duration(Some(std::time::Duration::from_secs(4)));
        }
    }

    /// Drop what the schedulers hold for the days the delete removed.
    fn forget_pruned_environment_days(&mut self, report: &PruneReport) {
        let pruned = report.request.days;
        for archive in report.archives.iter().filter(|one| one.outcome.is_ok()) {
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
            PruneRequest {
                scope: PruneScope::One(EnvironmentArchive::SolarFlares),
                days: PrunedDays::All,
            },
        );

        assert!(report.archives.is_empty());
        assert_eq!(report.days_removed(), 0);
        assert_eq!(
            archives
                .interference
                .as_ref()
                .expect("the archive is open")
                .days()
                .expect("days")
                .len(),
            3,
            "another archive's delete took days from the interference archive"
        );
    }

    #[test]
    fn a_failed_delete_is_reported_with_what_it_said() {
        let report = PruneReport {
            request: PruneRequest {
                scope: PruneScope::Every,
                days: PrunedDays::All,
            },
            archives: vec![
                ArchivePruneReport {
                    archive: EnvironmentArchive::AircraftInterference,
                    outcome: Ok(4),
                },
                ArchivePruneReport {
                    archive: EnvironmentArchive::IonosphericTec,
                    outcome: Err("archive is inconsistent".to_owned()),
                },
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
