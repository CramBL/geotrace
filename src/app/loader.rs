use std::{
    fs,
    path::PathBuf,
    sync::{Arc, mpsc},
    thread,
};

use gt_track_builder::{GeneratedMarkerConfig, SegmentationConfig, TrackLayoutConfig};

use chrono::Utc;
use egui::Context;
use gt_loaded_files::FileHistory;
use gt_log_view::LogAttachmentRef;
use gt_logfile::{LogText, ParsedLog};
use gt_pending_writes::{PendingWrites, WriteKind};
use gt_plot::{AnalysisConfig, PreparedSeries};
use gt_store::{AttachedLog, StoredLogFilter};
use gt_types::LoadedFile;

/// A finished load stays fully opaque in the status list this long before its
/// entry starts fading.
pub(super) const FINISHED_JOB_FADE_START_SECS: f32 = 2.0;
/// A finished load's entry reaches full transparency and is dropped from the
/// status list this long after it completed.
pub(super) const FINISHED_JOB_EXPIRE_SECS: f32 = 3.0;

pub(super) const STAGE_STARTING: &str = "Starting…";
pub(super) const STAGE_READING: &str = "Reading…";
pub(super) const STAGE_PARSING: &str = "Parsing…";
pub(super) const STAGE_PLOTTING: &str = "Building plot data…";

/// State for a single in-flight background load job, shown in the progress UI.
pub struct LoadingJob {
    pub id: u64,
    pub filename: String,
    pub progress: f32,
    pub stage: &'static str,
    /// Wall-clock time when the job was enqueued, used to display elapsed time.
    pub started_at: std::time::Instant,
}

/// A load job that has finished and is waiting to be dismissed from the UI.
///
/// Shown for a few seconds with a fade-out so the user can see how long the
/// load took even if it completed quickly.
pub struct FinishedJob {
    pub filename: String,
    /// Total wall-clock time the job took, frozen at the moment of completion.
    pub elapsed_secs: f32,
    /// egui frame time (`Context::input().time`, seconds) when the job
    /// completed, used to drive the fade-out animation. Frame time rather than
    /// [`std::time::Instant`] so the animation is a pure function of the
    /// deterministic clock the test harness advances - a wall-clock read here
    /// leaks into snapshots and makes them racy.
    pub completed_at: f64,
}

/// Final result produced by a background load thread.
pub enum LoadOutcome {
    /// A successfully parsed `.gtd` / HDF5 file with pre-built plot series.
    GtdFile {
        file: LoadedFile,
        /// Pre-built mipmap series, bound to a file index by
        /// [`gt_plot::PlotState::integrate_file`] once the UI thread appends
        /// the file.
        series: PreparedSeries,
        /// App-owned history attachment metadata for this file.
        history: FileHistory,
        /// True when a history load rebuilt generated markers with the current
        /// app settings rather than the recording's stored/default marker settings.
        applied_current_marker_settings: bool,
    },
    /// A successfully parsed log, not yet associated with a recording.
    Log {
        /// The name of the file the log was read from, `None` for text that
        /// arrived without one.
        filename: Option<String>,
        parsed: ParsedLog,
        /// Set for a log that came back with a recording opened from history.
        restored: Option<AttachedLogRestore>,
    },
}

/// A log read back out of history: the attachment it is stored as, and the
/// filter stack stored with it.
pub(super) struct AttachedLogRestore {
    pub attachment: LogAttachmentRef,
    pub filters: Vec<StoredLogFilter>,
}

/// Messages sent from background load threads to the UI thread via `mpsc`.
#[expect(
    clippy::large_enum_variant,
    reason = "Completed carries a full LoadOutcome by design; boxing would add an allocation on the infrequent completion path"
)]
pub enum LoadMessage {
    Progress {
        id: u64,
        fraction: f32,
        stage: &'static str,
    },
    Completed {
        id: u64,
        outcome: Result<LoadOutcome, String>,
    },
}

/// The result of a single completed background load, returned by `LoadJobs::drain`.
pub(super) struct CompletedLoad {
    pub filename: String,
    pub elapsed_secs: f32,
    pub outcome: Result<LoadOutcome, String>,
}

/// What to do, beyond a plain load, with a recording opened from history.
pub(super) enum HistoryOpen {
    /// Remove these track positions (0-based, segmentation order) from the loaded
    /// view - the recording's hidden tracks. The stored table is left unchanged.
    ApplyHidden {
        db_ref: gt_store::DatabaseRef,
        positions: Vec<usize>,
        applied_current_marker_settings: bool,
    },
    /// Overwrite the recording's stored track table and segmentation settings with
    /// a fresh segmentation under the load config (recalculation), discarding the
    /// previous hidden marks.
    Recalculate {
        db_ref: gt_store::DatabaseRef,
        applied_current_marker_settings: bool,
    },
}

impl HistoryOpen {
    fn applied_current_marker_settings(&self) -> bool {
        match self {
            Self::ApplyHidden {
                applied_current_marker_settings,
                ..
            }
            | Self::Recalculate {
                applied_current_marker_settings,
                ..
            } => *applied_current_marker_settings,
        }
    }

    fn db_ref(&self) -> &gt_store::DatabaseRef {
        match self {
            Self::ApplyHidden { db_ref, .. } | Self::Recalculate { db_ref, .. } => db_ref,
        }
    }
}

/// Manages the file-loading channel and all background load threads.
///
/// `loading_jobs` and `finishing_jobs` are public for the progress overlay UI.
pub(super) struct LoadJobs {
    ctx: Context,
    load_tx: mpsc::Sender<LoadMessage>,
    load_rx: mpsc::Receiver<LoadMessage>,
    pub loading_jobs: Vec<LoadingJob>,
    pub finishing_jobs: Vec<FinishedJob>,
    next_id: u64,
    file_dialog_rx: Option<mpsc::Receiver<Option<PathBuf>>>,
    /// Path to the history database file, forwarded to background load threads
    /// so they can insert recordings after parsing.  `None` when storage is
    /// unavailable (DB failed to open at startup).
    pub db_path: Option<PathBuf>,
    /// Analysis parameters (elevation mask) forwarded to background load threads
    /// so freshly built plot series match the rest of the plot.  Kept in sync
    /// with the plot state's analysis config by the app.
    pub analysis_config: AnalysisConfig,
    /// Registers every recording-database write a load thread makes, and
    /// refuses the ones that would start after shutdown began.
    pending_writes: PendingWrites,
}

impl LoadJobs {
    pub fn new(ctx: Context, pending_writes: PendingWrites) -> Self {
        let (load_tx, load_rx) = mpsc::channel::<LoadMessage>();
        Self {
            ctx,
            load_tx,
            load_rx,
            loading_jobs: Vec::new(),
            finishing_jobs: Vec::new(),
            next_id: 0,
            file_dialog_rx: None,
            db_path: None,
            analysis_config: AnalysisConfig::default(),
            pending_writes,
        }
    }

    fn alloc_id(&mut self) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        id
    }

    #[expect(
        clippy::expect_used,
        reason = "thread spawn can only fail under extreme system resource exhaustion"
    )]
    pub fn spawn_gtd_path(&mut self, path: PathBuf, config: SegmentationConfig) {
        let id = self.alloc_id();
        let filename = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("unknown")
            .to_owned();
        self.loading_jobs.push(LoadingJob {
            id,
            filename: filename.clone(),
            progress: 0.0,
            stage: STAGE_STARTING,
            started_at: std::time::Instant::now(),
        });
        let tx = self.load_tx.clone();
        let ctx = self.ctx.clone();
        let db_path = self.db_path.clone();
        let analysis = self.analysis_config;
        let pending_writes = self.pending_writes.clone();
        let log_name = filename.clone();
        log::info!("Loading file '{filename}'");
        thread::Builder::new()
            .name(format!("load-{filename}"))
            .spawn(move || {
                let r_tx = tx.clone();
                let r_ctx = ctx.clone();
                let r_name = log_name.clone();
                let report = move |fraction: f32, stage: &'static str| {
                    log::debug!("'{r_name}': {stage} ({:.0}%)", fraction * 100.0);
                    r_tx.send(LoadMessage::Progress {
                        id,
                        fraction,
                        stage,
                    })
                    .ok();
                    r_ctx.request_repaint();
                };
                let outcome = gt_loader::load_gtd_file_with_progress(&path, report, &config)
                    .map(|loaded| {
                        let file = loaded.file;
                        tx.send(LoadMessage::Progress {
                            id,
                            fraction: 0.95,
                            stage: STAGE_PLOTTING,
                        })
                        .ok();
                        ctx.request_repaint();
                        let series = gt_plot::prepare_file_series(&file, analysis);
                        // Read the bytes once for both the content fingerprint
                        // and the optional history insert.
                        let bytes = match std::fs::read(&path) {
                            Ok(bytes) => Some(bytes),
                            Err(e) => {
                                log::warn!(
                                    "Could not reread '{log_name}' for history storage from {}: {e}",
                                    path.display()
                                );
                                None
                            }
                        };
                        let meta = match bytes.as_deref().map(gt_store::extract_meta) {
                            Some(Ok(meta)) => Some(meta),
                            Some(Err(e)) => {
                                log::warn!(
                                    "Could not extract history metadata from '{log_name}': {e}"
                                );
                                None
                            }
                            None => None,
                        };
                        log::debug!("Parsed '{log_name}': {} track(s)", file.tracks.len());
                        let db_ref = HistoryInsert {
                            db_path: db_path.as_deref(),
                            file: &file,
                            identity: &loaded.identity,
                            meta: meta.as_ref(),
                            config: &config,
                            bytes: bytes.as_deref(),
                            filename: &log_name,
                            pending_writes: &pending_writes,
                        }
                        .store();
                        let history = meta.map_or(FileHistory::None, |meta| {
                            FileHistory::recording(loaded.identity, meta, db_ref)
                        });
                        LoadOutcome::GtdFile {
                            file,
                            series,
                            history,
                            applied_current_marker_settings: false,
                        }
                    })
                    .map_err(|e| e.to_string());
                tx.send(LoadMessage::Completed { id, outcome }).ok();
                ctx.request_repaint();
            })
            .expect("failed to spawn gtd-path loader thread");
    }

    pub fn spawn_gtd_bytes(
        &mut self,
        bytes: Arc<[u8]>,
        filename: String,
        config: SegmentationConfig,
    ) {
        self.spawn_bytes_job(bytes, filename, config, None);
    }

    /// Load a recording opened from the history database, applying the chosen
    /// open behaviour (re-apply the recording's hidden tracks, or recalculate and
    /// overwrite its stored track table) once it is parsed.
    pub fn spawn_gtd_from_history(
        &mut self,
        bytes: Arc<[u8]>,
        filename: String,
        config: SegmentationConfig,
        open: HistoryOpen,
    ) {
        self.spawn_bytes_job(bytes, filename, config, Some(open));
    }

    #[expect(
        clippy::expect_used,
        reason = "thread spawn can only fail under extreme system resource exhaustion"
    )]
    fn spawn_bytes_job(
        &mut self,
        bytes: Arc<[u8]>,
        filename: String,
        config: SegmentationConfig,
        open: Option<HistoryOpen>,
    ) {
        let id = self.alloc_id();
        self.loading_jobs.push(LoadingJob {
            id,
            filename: filename.clone(),
            progress: 0.0,
            stage: STAGE_STARTING,
            started_at: std::time::Instant::now(),
        });
        let tx = self.load_tx.clone();
        let ctx = self.ctx.clone();
        let db_path = self.db_path.clone();
        let analysis = self.analysis_config;
        let pending_writes = self.pending_writes.clone();
        let log_name = filename.clone();
        log::info!("Loading file '{filename}'");
        thread::Builder::new()
            .name(format!("load-{filename}"))
            .spawn(move || {
                let r_tx = tx.clone();
                let r_ctx = ctx.clone();
                let r_name = log_name.clone();
                let report = move |frac: f32, stage: &'static str| {
                    log::debug!("'{r_name}': {stage} ({:.0}%)", frac * 100.0);
                    r_tx.send(LoadMessage::Progress {
                        id,
                        fraction: frac,
                        stage,
                    })
                    .ok();
                    r_ctx.request_repaint();
                };
                let outcome =
                    gt_loader::load_gtd_bytes_with_progress(&bytes, filename, report, &config)
                        .map(|loaded| {
                            let mut file = loaded.file;
                            let applied_current_marker_settings = open
                                .as_ref()
                                .is_some_and(HistoryOpen::applied_current_marker_settings);
                            let meta = match gt_store::extract_meta(&bytes) {
                                Ok(meta) => Some(meta),
                                Err(e) => {
                                    log::warn!(
                                        "Could not extract history metadata from '{log_name}': {e}"
                                    );
                                    None
                                }
                            };
                            log::debug!("Parsed '{log_name}': {} track(s)", file.tracks.len());
                            // Store first (de-duplicates against the existing
                            // recording, keeping its stored track table) while the
                            // freshly segmented tracks are still in stored order.
                            let db_ref = HistoryInsert {
                                db_path: db_path.as_deref(),
                                file: &file,
                                identity: &loaded.identity,
                                meta: meta.as_ref(),
                                config: &config,
                                bytes: Some(&bytes),
                                filename: &log_name,
                                pending_writes: &pending_writes,
                            }
                            .store();
                            let open_db_ref = open.as_ref().map(HistoryOpen::db_ref).cloned();
                            let history_db_ref = db_ref.or(open_db_ref);
                            match &open {
                                Some(HistoryOpen::Recalculate { db_ref, .. }) => {
                                    if let Some(path) = db_path.as_deref() {
                                        recalculate_stored_tracks(
                                            path,
                                            db_ref,
                                            &file,
                                            &config,
                                            &log_name,
                                            &pending_writes,
                                        );
                                    }
                                }
                                Some(HistoryOpen::ApplyHidden { positions, .. }) => {
                                    drop_tracks(&mut file, positions);
                                }
                                None => {}
                            }
                            // Build the plot series after any hidden-track removal
                            // so the series matches the visible tracks.
                            tx.send(LoadMessage::Progress {
                                id,
                                fraction: 0.95,
                                stage: STAGE_PLOTTING,
                            })
                            .ok();
                            ctx.request_repaint();
                            let series = gt_plot::prepare_file_series(&file, analysis);
                            let history = meta.map_or(FileHistory::None, |meta| {
                                FileHistory::recording(loaded.identity, meta, history_db_ref)
                            });
                            LoadOutcome::GtdFile {
                                file,
                                series,
                                history,
                                applied_current_marker_settings,
                            }
                        })
                        .map_err(|e| e.to_string());
                tx.send(LoadMessage::Completed { id, outcome }).ok();
                ctx.request_repaint();
            })
            .expect("failed to spawn gtd-bytes loader thread");
    }

    #[expect(
        clippy::expect_used,
        reason = "thread spawn can only fail under extreme system resource exhaustion"
    )]
    pub fn spawn_log_path(&mut self, path: PathBuf) {
        let id = self.alloc_id();
        let filename = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("unknown")
            .to_owned();
        self.loading_jobs.push(LoadingJob {
            id,
            filename: filename.clone(),
            progress: 0.0,
            stage: STAGE_STARTING,
            started_at: std::time::Instant::now(),
        });
        let tx = self.load_tx.clone();
        let ctx = self.ctx.clone();
        thread::Builder::new()
            .name(format!("load-log-{filename}"))
            .spawn(move || {
                let report = progress_reporter(id, tx.clone(), ctx.clone());
                report(0.20, STAGE_READING);
                let bytes = match fs::read(&path) {
                    Ok(bytes) => bytes,
                    Err(e) => {
                        tx.send(LoadMessage::Completed {
                            id,
                            outcome: Err(format!("Failed to read {filename}: {e}")),
                        })
                        .ok();
                        ctx.request_repaint();
                        return;
                    }
                };
                let text = LogText::decode_lossy(&bytes);
                finish_log_load(id, Some(filename), text, None, &tx, &ctx, report);
            })
            .expect("failed to spawn log-path loader thread");
    }

    /// Loads the bytes of a dropped log, which need not be valid UTF-8.
    /// `filename` is `None` when the drop carried no name.
    pub fn spawn_log_bytes(&mut self, bytes: Arc<[u8]>, filename: Option<String>) {
        self.spawn_log_load(filename, move || LogText::decode_lossy(&bytes));
    }

    /// Loads pasted log text. Pasted text has no name of its own, so the log
    /// takes its name from its first entry.
    pub fn spawn_pasted_log_text(&mut self, text: String) {
        log::info!("Loading {} bytes of pasted log text", text.len());
        self.spawn_log_load(None, move || LogText::from(text));
    }

    /// Runs `decode` on a loader thread and parses what it yields, under a job
    /// named after `filename`.
    #[expect(
        clippy::expect_used,
        reason = "thread spawn can only fail under extreme system resource exhaustion"
    )]
    fn spawn_log_load(
        &mut self,
        filename: Option<String>,
        decode: impl FnOnce() -> LogText + Send + 'static,
    ) {
        let id = self.alloc_id();
        let job_name = filename.clone().unwrap_or_else(|| "log text".to_owned());
        self.loading_jobs.push(LoadingJob {
            id,
            filename: job_name.clone(),
            progress: 0.0,
            stage: STAGE_STARTING,
            started_at: std::time::Instant::now(),
        });
        let tx = self.load_tx.clone();
        let ctx = self.ctx.clone();
        thread::Builder::new()
            .name(format!("load-log-{job_name}"))
            .spawn(move || {
                let report = progress_reporter(id, tx.clone(), ctx.clone());
                report(0.20, STAGE_READING);
                let text = decode();
                finish_log_load(id, filename, text, None, &tx, &ctx, report);
            })
            .expect("failed to spawn log-text loader thread");
    }

    /// Parses a log a recording carries as an attachment, so it comes back
    /// with that recording.
    #[expect(
        clippy::expect_used,
        reason = "thread spawn can only fail under extreme system resource exhaustion"
    )]
    pub fn spawn_attached_log(&mut self, log: AttachedLog, attachment: LogAttachmentRef) {
        let id = self.alloc_id();
        let AttachedLog {
            name,
            text,
            filters,
        } = log;
        self.loading_jobs.push(LoadingJob {
            id,
            filename: name.clone(),
            progress: 0.0,
            stage: STAGE_STARTING,
            started_at: std::time::Instant::now(),
        });
        let tx = self.load_tx.clone();
        let ctx = self.ctx.clone();
        let text = LogText::from(text);
        let restored = AttachedLogRestore {
            attachment,
            filters,
        };
        log::info!("Loading the log {name:?} attached to a recording opened from history");
        thread::Builder::new()
            .name(format!("load-log-{name}"))
            .spawn(move || {
                let report = progress_reporter(id, tx.clone(), ctx.clone());
                finish_log_load(id, Some(name), text, Some(restored), &tx, &ctx, report);
            })
            .expect("failed to spawn attached-log loader thread");
    }

    /// Spawn a background thread that shows the OS file-picker dialog.
    ///
    /// `rfd::FileDialog::pick_file()` blocks its calling thread.  Running it
    /// on the render thread freezes the egui loop. On Wayland the compositor
    /// then stops delivering events, making the window appear unresponsive.
    /// The dialog runs on a dedicated thread instead. The chosen path arrives
    /// via `drain_file_dialog` each frame.
    #[expect(
        clippy::expect_used,
        reason = "thread spawn can only fail under extreme system resource exhaustion"
    )]
    pub fn open_file_dialog(&mut self) {
        let (tx, rx) = mpsc::channel();
        self.file_dialog_rx = Some(rx);
        let ctx = self.ctx.clone();
        thread::Builder::new()
            .name("file-dialog".to_owned())
            .spawn(move || {
                let path = rfd::FileDialog::new()
                    .add_filter("GeoTrace Data", &["gtd"])
                    .add_filter("Log Files", &["log", "txt"])
                    .pick_file();
                tx.send(path).ok();
                ctx.request_repaint();
            })
            .expect("failed to spawn file-dialog thread");
    }

    pub fn drain_file_dialog(&mut self) -> Option<PathBuf> {
        let rx = self.file_dialog_rx.as_ref()?;
        let Ok(path_opt) = rx.try_recv() else {
            return None;
        };
        self.file_dialog_rx = None;
        path_opt
    }

    /// Drain all pending channel messages. Updates `loading_jobs` for progress
    /// messages, returns one `CompletedLoad` per finished job.
    pub fn drain(&mut self) -> Vec<CompletedLoad> {
        let mut completed = Vec::new();
        while let Ok(msg) = self.load_rx.try_recv() {
            match msg {
                LoadMessage::Progress {
                    id,
                    fraction,
                    stage,
                } => {
                    if let Some(job) = self.loading_jobs.iter_mut().find(|j| j.id == id) {
                        job.progress = fraction;
                        job.stage = stage;
                    }
                }
                LoadMessage::Completed { id, outcome } => {
                    let elapsed_secs = self
                        .loading_jobs
                        .iter()
                        .find(|j| j.id == id)
                        .map_or(0.0, |j| j.started_at.elapsed().as_secs_f32());
                    let filename = self
                        .loading_jobs
                        .iter()
                        .find(|j| j.id == id)
                        .map(|j| j.filename.clone())
                        .unwrap_or_default();
                    self.loading_jobs.retain(|j| j.id != id);
                    completed.push(CompletedLoad {
                        filename,
                        elapsed_secs,
                        outcome,
                    });
                }
            }
        }
        completed
    }

    /// Remove the entries from `finishing_jobs` that have fully faded. `now` is
    /// the current egui frame time (`Context::input().time`).
    pub fn expire_finished(&mut self, now: f64) {
        self.finishing_jobs
            .retain(|j| now - j.completed_at < f64::from(FINISHED_JOB_EXPIRE_SECS));
    }
}

/// Reports a load job's progress to the UI thread and wakes it to draw it.
fn progress_reporter(
    id: u64,
    tx: mpsc::Sender<LoadMessage>,
    ctx: Context,
) -> impl Fn(f32, &'static str) {
    move |fraction: f32, stage: &'static str| {
        tx.send(LoadMessage::Progress {
            id,
            fraction,
            stage,
        })
        .ok();
        ctx.request_repaint();
    }
}

/// Shared tail of log loading: parse `text` and send the `Completed` message.
/// Called from both the path-based and the text-based log loader threads.
fn finish_log_load(
    id: u64,
    filename: Option<String>,
    text: LogText,
    restored: Option<AttachedLogRestore>,
    tx: &mpsc::Sender<LoadMessage>,
    ctx: &Context,
    report: impl Fn(f32, &'static str),
) {
    report(0.55, STAGE_PARSING);
    let outcome = match gt_logfile::parse_log(text, Utc::now()) {
        Ok(parsed) => {
            let unindexable_line_count = parsed.unindexable_line_count();
            if unindexable_line_count > 0 {
                let noun = gt_fmt::pluralize(unindexable_line_count, "line", "lines");
                let name = filename.as_deref().unwrap_or("log text");
                log::warn!(
                    "Dropped {unindexable_line_count} {noun} of {name:?} that the log index cannot address"
                );
            }
            Ok(LoadOutcome::Log {
                filename,
                parsed,
                restored,
            })
        }
        Err(err) => Err(err.to_string()),
    };
    tx.send(LoadMessage::Completed { id, outcome }).ok();
    ctx.request_repaint();
}

/// Convert the live segmentation settings into the form persisted alongside a
/// recording, so that re-opening can detect when the app's current settings
/// differ from those the stored tracks were built with.
pub(crate) fn stored_segmentation_from_config(
    config: &SegmentationConfig,
) -> gt_store::StoredSegmentation {
    gt_store::StoredSegmentation {
        track_split_gap_us: track_split_gap_us(config.track_layout),
        detect_clock_discontinuities: config.generated_markers.detect_clock_discontinuities,
        clock_discontinuity_sigmas: config.generated_markers.clock_discontinuity_sigmas,
    }
}

fn track_split_gap_us(config: TrackLayoutConfig) -> i64 {
    config
        .track_split_gap
        .num_microseconds()
        .unwrap_or(i64::MAX)
}

fn track_layout_from_stored(settings: &gt_store::StoredSegmentation) -> TrackLayoutConfig {
    TrackLayoutConfig {
        track_split_gap: chrono::Duration::microseconds(settings.track_split_gap_us),
    }
}

fn generated_markers_from_stored(settings: &gt_store::StoredSegmentation) -> GeneratedMarkerConfig {
    GeneratedMarkerConfig {
        detect_clock_discontinuities: settings.detect_clock_discontinuities,
        clock_discontinuity_sigmas: settings.clock_discontinuity_sigmas,
        ..GeneratedMarkerConfig::default()
    }
}

/// Returns `true` when the stored tracks were split with the same track-layout
/// setting as the current app config. Generated-marker settings are intentionally
/// ignored here because they do not affect the stored track ranges.
pub(crate) fn track_split_matches_config(
    settings: &gt_store::StoredSegmentation,
    config: &SegmentationConfig,
) -> bool {
    track_layout_from_stored(settings) == config.track_layout
}

/// Rebuild a [`SegmentationConfig`] for opening stored history tracks.
///
/// The stored track split gap is kept so hidden-track indices still line up with
/// the stored track table. Generated-marker settings come from `current`, so a
/// history load reflects the user's current marker toggles and slip thresholds.
pub(crate) fn config_from_stored_segmentation(
    settings: &gt_store::StoredSegmentation,
    current: SegmentationConfig,
) -> SegmentationConfig {
    SegmentationConfig {
        track_layout: track_layout_from_stored(settings),
        generated_markers: current.generated_markers,
    }
}

/// Returns `true` when the marker settings implied by a stored recording match
/// the current app config. History did not persist every generated-marker field,
/// so missing fields are treated as the historical defaults for mismatch
/// detection only. Loading still uses [`config_from_stored_segmentation`].
pub(crate) fn marker_settings_match_config(
    settings: &gt_store::StoredSegmentation,
    current: &SegmentationConfig,
) -> bool {
    generated_markers_from_stored(settings) == current.generated_markers
}

/// Derive contiguous per-track index ranges from a loaded file's tracks.
///
/// Segmentation produces contiguous ranges and the loader builds nav points 1:1
/// with the original file, so the cumulative point counts reconstruct the exact
/// `[start, end)` ranges into the recording's nav points.
fn track_ranges_from_file(file: &LoadedFile) -> Vec<gt_store::TrackRange> {
    let mut start = 0_u64;
    file.tracks
        .iter()
        .map(|t| {
            let end = start + t.points.len() as u64;
            let range = gt_store::TrackRange {
                start,
                end,
                hidden: false,
            };
            start = end;
            range
        })
        .collect()
}

/// Remove the tracks at the given 0-based positions (segmentation order) from a
/// loaded file's view - used to re-apply a recording's stored hidden tracks when
/// it is opened from history.
fn drop_tracks(file: &mut LoadedFile, positions: &[usize]) {
    if positions.is_empty() {
        return;
    }
    let drop: std::collections::HashSet<usize> = positions.iter().copied().collect();
    file.tracks = std::mem::take(&mut file.tracks)
        .into_iter()
        .enumerate()
        .filter(|(i, _)| !drop.contains(i))
        .map(|(_, t)| t)
        .collect();
    log::debug!("Applied {} hidden track(s) on open", drop.len());
}

/// Overwrite a recording's stored track table and segmentation settings with a
/// fresh segmentation of `file` under `config` (the recalculation path). Logs and
/// continues on failure - the recording is still loaded into the view.
fn recalculate_stored_tracks(
    db_path: &std::path::Path,
    db_ref: &gt_store::DatabaseRef,
    file: &LoadedFile,
    config: &SegmentationConfig,
    filename: &str,
    pending_writes: &PendingWrites,
) {
    use gt_store::HistoryDatabase;
    let _write = match pending_writes.try_begin(
        format!("Recalculating the stored tracks of {filename}"),
        WriteKind::RecordingDatabase,
    ) {
        Ok(write) => write,
        Err(refusal) => {
            log::debug!("Not recalculating the stored tracks of '{filename}': {refusal}");
            return;
        }
    };
    let tracks = track_ranges_from_file(file);
    let settings = stored_segmentation_from_config(config);
    match gt_store::Recordings::open_or_create(db_path) {
        Ok(mut db) => match db.set_tracks(db_ref, &tracks, settings) {
            Ok(()) => log::info!(
                "Recalculated stored tracks for {}/{} ({} track(s))",
                db_ref.identity,
                db_ref.group_name,
                tracks.len()
            ),
            Err(e) => log::warn!(
                "Failed to recalculate stored tracks for {}/{}: {e}",
                db_ref.identity,
                db_ref.group_name
            ),
        },
        Err(e) => log::warn!("Could not open history database to recalculate tracks: {e}"),
    }
}

/// One freshly loaded recording, and where it is to be stored.
struct HistoryInsert<'a> {
    /// `None` when storage is unavailable: nothing is stored then.
    db_path: Option<&'a std::path::Path>,
    file: &'a LoadedFile,
    identity: &'a str,
    meta: Option<&'a gt_store::RecordingMeta>,
    config: &'a SegmentationConfig,
    /// The `.gtd` bytes as they were read, stored alongside the recording.
    bytes: Option<&'a [u8]>,
    filename: &'a str,
    pending_writes: &'a PendingWrites,
}

impl HistoryInsert<'_> {
    /// Insert the recording into the history database, logging the outcome at
    /// each branch. Returns the stored reference, or `None` when storage is
    /// disabled, metadata is missing, the write registry turned the insert
    /// away, or the insert failed.
    fn store(self) -> Option<gt_store::DatabaseRef> {
        use gt_store::HistoryDatabase;

        let Self {
            db_path,
            file,
            identity,
            meta,
            config,
            bytes,
            filename,
            pending_writes,
        } = self;

        let Some(path) = db_path else {
            log::debug!("Storage disabled; not storing '{filename}' in history");
            return None;
        };
        let (Some(meta), Some(bytes)) = (meta, bytes) else {
            log::warn!("No recording metadata for '{filename}'; not storing in history");
            return None;
        };
        let _write = match pending_writes.try_begin(
            format!("Storing {filename} in recording history"),
            WriteKind::RecordingDatabase,
        ) {
            Ok(write) => write,
            Err(refusal) => {
                log::debug!("Not storing '{filename}' in history: {refusal}");
                return None;
            }
        };

        let tracks = track_ranges_from_file(file);
        // The cumulative end must cover exactly the recording's nav points (tracks
        // are a contiguous 1:1 partition), otherwise the derivation is unsound.
        debug_assert_eq!(
            tracks.last().map_or(0, |t| t.end),
            meta.nav_point_count,
            "track ranges must cover all nav points"
        );
        let settings = stored_segmentation_from_config(config);

        let mut db = match gt_store::Recordings::open_or_create(path) {
            Ok(db) => db,
            Err(e) => {
                log::warn!("Could not open history database at {}: {e}", path.display());
                return None;
            }
        };
        log::debug!(
            "Storing '{filename}' in history at {} with identity={identity:?}, start_us={}, nav_points={}, tracks={}",
            path.display(),
            meta.start_us,
            meta.nav_point_count,
            tracks.len()
        );
        match db.insert(identity, meta, &tracks, settings, bytes) {
            Ok(db_ref) => {
                match db.list_recordings() {
                    Ok(entries) => {
                        if entries.iter().any(|entry| entry.db_ref == db_ref) {
                            log::info!(
                                "Stored '{filename}' in history as identity={:?}, group={:?} ({} track(s))",
                                db_ref.identity,
                                db_ref.group_name,
                                tracks.len()
                            );
                        } else {
                            log::error!(
                                "Stored '{filename}' in history as identity={:?}, group={:?}, but it is not visible in the history listing",
                                db_ref.identity,
                                db_ref.group_name
                            );
                        }
                    }
                    Err(e) => {
                        log::error!(
                            "Stored '{filename}' in history as identity={:?}, group={:?}, but listing the history failed: {e}",
                            db_ref.identity,
                            db_ref.group_name
                        );
                    }
                }
                Some(db_ref)
            }
            Err(e) => {
                log::warn!(
                    "Failed to store '{filename}' in history at {} with identity={identity:?}: {e}",
                    path.display()
                );
                None
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use chrono::DateTime;
    use gt_store::{HistoryDatabase, Recordings};
    use gt_test_utils::{SyntheticGtdSpec, synthetic_gtd_bytes};

    use super::*;

    #[test]
    fn stored_segmentation_records_persisted_fields() {
        let generated_markers = GeneratedMarkerConfig {
            detect_clock_discontinuities: false,
            clock_discontinuity_sigmas: 3.5,
            ..GeneratedMarkerConfig::default()
        };
        let config = SegmentationConfig {
            track_layout: TrackLayoutConfig {
                track_split_gap: chrono::Duration::milliseconds(42_500),
            },
            generated_markers,
        };
        let stored = stored_segmentation_from_config(&config);
        assert_eq!(stored.track_split_gap_us, 42_500_000);
        assert!(!stored.detect_clock_discontinuities);
        assert_eq!(
            stored.clock_discontinuity_sigmas.to_bits(),
            config
                .generated_markers
                .clock_discontinuity_sigmas
                .to_bits()
        );
    }

    #[test]
    fn history_open_config_keeps_stored_track_split_and_current_marker_settings() {
        let stored_source = SegmentationConfig {
            track_layout: TrackLayoutConfig {
                track_split_gap: chrono::Duration::milliseconds(42_500),
            },
            generated_markers: GeneratedMarkerConfig {
                detect_clock_discontinuities: true,
                clock_discontinuity_sigmas: 7.0,
                ..GeneratedMarkerConfig::default()
            },
        };
        let current = SegmentationConfig {
            generated_markers: GeneratedMarkerConfig {
                detect_gnss_fix_lost: false,
                detect_gnss_fix_regained: false,
                detect_clock_discontinuities: false,
                clock_discontinuity_sigmas: 3.5,
                detect_clock_offset_excursions: false,
                clock_excursion_threshold_s: 30.0,
                detect_slips: false,
                slip_elevation_mask_deg: 30.0,
                slip_snr_drop_db: 20.0,
            },
            ..SegmentationConfig::default()
        };

        let stored = stored_segmentation_from_config(&stored_source);
        let back = config_from_stored_segmentation(&stored, current);

        assert_eq!(back.track_layout, stored_source.track_layout);
        assert_eq!(back.generated_markers, current.generated_markers);
    }

    #[test]
    fn marker_settings_match_detects_unstored_slip_toggle() {
        let stored = stored_segmentation_from_config(&SegmentationConfig::default());

        assert!(marker_settings_match_config(
            &stored,
            &SegmentationConfig::default()
        ));

        let current = SegmentationConfig {
            generated_markers: GeneratedMarkerConfig {
                detect_slips: false,
                ..GeneratedMarkerConfig::default()
            },
            ..SegmentationConfig::default()
        };

        assert!(!marker_settings_match_config(&stored, &current));
    }

    #[test]
    fn marker_settings_match_uses_stored_clock_marker_fields() {
        let stored_config = SegmentationConfig {
            generated_markers: GeneratedMarkerConfig {
                detect_clock_discontinuities: false,
                clock_discontinuity_sigmas: 3.5,
                ..GeneratedMarkerConfig::default()
            },
            ..SegmentationConfig::default()
        };
        let stored = stored_segmentation_from_config(&stored_config);
        let current = SegmentationConfig {
            generated_markers: GeneratedMarkerConfig {
                detect_clock_discontinuities: false,
                clock_discontinuity_sigmas: 3.5,
                ..GeneratedMarkerConfig::default()
            },
            ..SegmentationConfig::default()
        };

        assert!(marker_settings_match_config(&stored, &current));
    }

    fn write_sample_gtd(dir: &std::path::Path) -> PathBuf {
        let bytes = synthetic_gtd_bytes(SyntheticGtdSpec {
            start: DateTime::from_timestamp(1_748_000_000, 0).expect("valid timestamp"),
            point_count: 20,
            step_secs: 1,
            start_lat_deg: 51.5,
            start_lon_deg: -0.1,
            lat_step_deg: 0.0002,
            lon_step_deg: -0.00015,
            heading_deg: 270.0,
            speed_kmh: 22.0,
            eph_m: 2.4,
            sats_seen: 10,
            sats_in_fix: 7,
        });
        let path = dir.join("sample.gtd");
        std::fs::write(&path, &bytes).expect("write gtd");
        path
    }

    /// Block until a background load finishes, or time out.
    fn drain_until_complete(jobs: &mut LoadJobs) -> CompletedLoad {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            if let Some(done) = jobs.drain().into_iter().next() {
                return done;
            }
            assert!(Instant::now() < deadline, "load did not finish in time");
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    /// Pasted log text reaches the app as a parsed log, with no name of its
    /// own for the log to be named after its first entry.
    #[test]
    fn loading_pasted_log_text_completes_as_a_parsed_log() {
        let mut jobs = LoadJobs::new(egui::Context::default(), PendingWrites::default());
        jobs.spawn_pasted_log_text(
            "2026-01-01 14:02:11 navsyncd: uploaded 2 recordings\n".to_owned(),
        );

        let completed = drain_until_complete(&mut jobs);
        let LoadOutcome::Log {
            filename, parsed, ..
        } = completed.outcome.expect("load should succeed")
        else {
            panic!("expected a Log outcome");
        };
        assert_eq!(filename, None);
        assert_eq!(parsed.entries().len(), 1);
    }

    /// A log opened from a path is named after the file it was read from.
    #[test]
    fn loading_a_log_path_completes_as_a_parsed_log_named_after_the_file() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("navsyncd.log");
        std::fs::write(&path, "2026-01-01 14:02:11 navsyncd: queue empty\n").expect("write log");

        let mut jobs = LoadJobs::new(egui::Context::default(), PendingWrites::default());
        jobs.spawn_log_path(path);

        let completed = drain_until_complete(&mut jobs);
        let LoadOutcome::Log {
            filename, parsed, ..
        } = completed.outcome.expect("load should succeed")
        else {
            panic!("expected a Log outcome");
        };
        assert_eq!(filename.as_deref(), Some("navsyncd.log"));
        assert_eq!(parsed.entries().len(), 1);
    }

    /// Log text with no recognised timestamp format fails the load.
    #[test]
    fn loading_log_text_without_a_recognised_timestamp_fails() {
        let mut jobs = LoadJobs::new(egui::Context::default(), PendingWrites::default());
        jobs.spawn_pasted_log_text("kernel: no timestamp here\n".to_owned());

        let completed = drain_until_complete(&mut jobs);
        assert_eq!(
            completed.outcome.err(),
            Some(
                "Not a recognised log: no line has a timestamp in a known format \
                 (first line: \"kernel: no timestamp here\")"
                    .to_owned()
            )
        );
    }

    /// Regression: opening a `.gtd` file with storage enabled must insert it into
    /// the history database and report the resulting `db_ref`.
    #[test]
    fn loading_a_gtd_file_stores_it_in_history() {
        let dir = tempfile::tempdir().expect("temp dir");
        let gtd_path = write_sample_gtd(dir.path());
        let db_path = dir.path().join("history.h5");

        let mut jobs = LoadJobs::new(egui::Context::default(), PendingWrites::default());
        jobs.db_path = Some(db_path.clone());
        jobs.spawn_gtd_path(gtd_path, SegmentationConfig::default());

        let completed = drain_until_complete(&mut jobs);
        let outcome = completed.outcome.expect("load should succeed");
        let LoadOutcome::GtdFile { history, .. } = outcome else {
            panic!("expected a GtdFile outcome");
        };
        assert!(
            history.is_stored(),
            "loading with storage enabled must produce a history db_ref"
        );

        let db = Recordings::open_or_create(&db_path).expect("open history db");
        let entries = db.list_recordings().expect("list recordings");
        assert_eq!(
            entries.len(),
            1,
            "the loaded recording must be stored in history"
        );
    }

    /// With no database path (storage disabled) the file still loads, but nothing
    /// is written to history.
    #[test]
    fn loading_without_db_path_does_not_store() {
        let dir = tempfile::tempdir().expect("temp dir");
        let gtd_path = write_sample_gtd(dir.path());

        let mut jobs = LoadJobs::new(egui::Context::default(), PendingWrites::default());
        jobs.db_path = None;
        jobs.spawn_gtd_path(gtd_path, SegmentationConfig::default());

        let completed = drain_until_complete(&mut jobs);
        let outcome = completed.outcome.expect("load should succeed");
        let LoadOutcome::GtdFile { history, .. } = outcome else {
            panic!("expected a GtdFile outcome");
        };
        assert!(
            history.db_ref().is_none(),
            "no db_ref without a database path"
        );
    }

    /// A load finishing after shutdown began still reaches the view, and
    /// leaves the recording database as it was.
    #[test]
    fn loading_a_gtd_file_while_shutting_down_stores_nothing() {
        let dir = tempfile::tempdir().expect("temp dir");
        let gtd_path = write_sample_gtd(dir.path());
        let db_path = dir.path().join("history.h5");
        let pending_writes = PendingWrites::default();
        pending_writes.begin_shutdown();

        let mut jobs = LoadJobs::new(egui::Context::default(), pending_writes);
        jobs.db_path = Some(db_path.clone());
        jobs.spawn_gtd_path(gtd_path, SegmentationConfig::default());

        let completed = drain_until_complete(&mut jobs);
        let outcome = completed.outcome.expect("load should succeed");
        let LoadOutcome::GtdFile { file, history, .. } = outcome else {
            panic!("expected a GtdFile outcome");
        };
        assert!(!file.tracks.is_empty(), "the recording still loads");
        assert!(history.db_ref().is_none());
        assert!(!db_path.exists(), "the database was never opened");
    }

    /// A sample recording, loaded and stored in a fresh history database.
    fn load_and_store_sample(
        dir: &std::path::Path,
    ) -> (PathBuf, gt_store::DatabaseRef, LoadedFile) {
        let gtd_path = write_sample_gtd(dir);
        let db_path = dir.join("history.h5");
        let mut jobs = LoadJobs::new(egui::Context::default(), PendingWrites::default());
        jobs.db_path = Some(db_path.clone());
        jobs.spawn_gtd_path(gtd_path, SegmentationConfig::default());

        let completed = drain_until_complete(&mut jobs);
        let LoadOutcome::GtdFile { file, history, .. } =
            completed.outcome.expect("load should succeed")
        else {
            panic!("expected a GtdFile outcome");
        };
        let db_ref = history
            .db_ref()
            .cloned()
            .expect("loaded file must be stored in history");
        (db_path, db_ref, file)
    }

    /// The stored track split gap of `db_ref`, as the database holds it now.
    fn stored_track_split_gap_us(db_path: &std::path::Path, db_ref: &gt_store::DatabaseRef) -> i64 {
        let db = Recordings::open_or_create(db_path).expect("open db");
        db.load(db_ref)
            .expect("load stored recording")
            .segmentation
            .expect("a stored segmentation")
            .track_split_gap_us
    }

    /// Recalculating overwrites the stored segmentation with the one the
    /// recording was opened under.
    #[test]
    fn recalculating_stored_tracks_writes_the_segmentation_it_ran_under() {
        let dir = tempfile::tempdir().expect("temp dir");
        let (db_path, db_ref, file) = load_and_store_sample(dir.path());
        let config = SegmentationConfig {
            track_layout: TrackLayoutConfig {
                track_split_gap: chrono::Duration::milliseconds(42_500),
            },
            ..SegmentationConfig::default()
        };

        recalculate_stored_tracks(
            &db_path,
            &db_ref,
            &file,
            &config,
            "sample.gtd",
            &PendingWrites::default(),
        );

        assert_eq!(stored_track_split_gap_us(&db_path, &db_ref), 42_500_000);
    }

    /// Recalculating after shutdown began leaves the stored segmentation as it
    /// was.
    #[test]
    fn recalculating_stored_tracks_while_shutting_down_writes_nothing() {
        let dir = tempfile::tempdir().expect("temp dir");
        let (db_path, db_ref, file) = load_and_store_sample(dir.path());
        let stored_before = stored_track_split_gap_us(&db_path, &db_ref);
        let config = SegmentationConfig {
            track_layout: TrackLayoutConfig {
                track_split_gap: chrono::Duration::milliseconds(42_500),
            },
            ..SegmentationConfig::default()
        };
        let pending_writes = PendingWrites::default();
        pending_writes.begin_shutdown();

        recalculate_stored_tracks(
            &db_path,
            &db_ref,
            &file,
            &config,
            "sample.gtd",
            &pending_writes,
        );

        assert_eq!(stored_track_split_gap_us(&db_path, &db_ref), stored_before);
    }

    /// A loaded recording is stored with a per-track table, and those tracks can
    /// be hidden (what "Remove filtered data" does), surfacing in the listing so
    /// "Delete hidden data" enables.
    #[test]
    fn loaded_recording_stores_tracks_and_supports_hiding() {
        let dir = tempfile::tempdir().expect("temp dir");
        let gtd_path = write_sample_gtd(dir.path());
        let db_path = dir.path().join("history.h5");

        let mut jobs = LoadJobs::new(egui::Context::default(), PendingWrites::default());
        jobs.db_path = Some(db_path.clone());
        jobs.spawn_gtd_path(gtd_path, SegmentationConfig::default());
        let completed = drain_until_complete(&mut jobs);
        let LoadOutcome::GtdFile { history, .. } = completed.outcome.expect("load should succeed")
        else {
            panic!("expected a GtdFile outcome");
        };
        let db_ref = history
            .db_ref()
            .cloned()
            .expect("loaded file must be stored in history");

        let mut db = Recordings::open_or_create(&db_path).expect("open db");
        let stored = db.load(&db_ref).expect("load stored recording");
        assert!(
            !stored.tracks.is_empty(),
            "the loaded recording must store a track table"
        );

        // Hide every track (as removing all filtered data would).
        let all: Vec<usize> = (0..stored.tracks.len()).collect();
        db.set_tracks_hidden(&db_ref, &all, true).expect("hide");
        let entries = db.list_recordings().expect("list");
        assert_eq!(
            entries.first().map(|e| e.hidden_tracks),
            Some(stored.tracks.len()),
            "all tracks should now be hidden"
        );
    }
}
