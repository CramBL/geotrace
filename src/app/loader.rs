use std::{
    fs,
    path::PathBuf,
    sync::{Arc, mpsc},
    thread,
};

use gt_track_builder::SegmentationConfig;

use chrono::{DateTime, Utc};
use egui::Context;
use gt_plot::PreparedSeries;
use gt_types::{
    AssociationConfig, Coord, CustomMarker, FileMetadata, FileSource, LoadedFile, LoadedTrack,
    NavPoint, Rect, TimeRange, TrackMetadata, merc_bounds_for_rect,
};
use uom::si::f64::Length;
use uom::si::length::{kilometer, meter};

pub(super) const STAGE_STARTING: &str = "Starting…";
pub(super) const STAGE_READING: &str = "Reading…";
pub(super) const STAGE_PARSING: &str = "Parsing…";
pub(super) const STAGE_PROCESSING: &str = "Processing…";
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
    /// When the job completed - used to drive the fade-out animation.
    pub completed_at: std::time::Instant,
}

/// Final result produced by a background load thread.
pub enum LoadOutcome {
    /// A successfully parsed `.gtd` / HDF5 file with pre-built plot series.
    GtdFile {
        file: LoadedFile,
        /// Pre-built mipmap series; `fi` is a placeholder (0) because the real
        /// file index is only known on the UI thread when the file is appended
        /// to `loaded_files`.  `PlotState::integrate_file` re-stamps the index.
        series: PreparedSeries,
        /// Reference to the recording stored in the history database, if storage
        /// is enabled and the insert succeeded.
        db_ref: Option<gt_history::DatabaseRef>,
    },
    /// A successfully parsed log file; `loaded` is `None` when all entries were
    /// unassociated with any GPS track.
    LogFile {
        loaded: Option<LoadedFile>,
        /// Pre-built plot series for the loaded file, if any.
        series: Option<PreparedSeries>,
        unassociated: Vec<(DateTime<Utc>, String)>,
    },
}

/// Messages sent from background load threads to the UI thread via `mpsc`.
#[expect(
    clippy::large_enum_variant,
    reason = "Completed carries a full LoadOutcome by design; boxing would add an allocation on the infrequent completion path"
)]
pub enum LoadMessage {
    /// Intermediate progress update - does not indicate completion.
    Progress {
        id: u64,
        fraction: f32,
        stage: &'static str,
    },
    /// The job is finished - either a usable result or an error string.
    Completed {
        id: u64,
        outcome: Result<LoadOutcome, String>,
    },
}

/// The result of a single completed background load, returned by `LoaderManager::drain`.
pub(super) struct CompletedLoad {
    pub filename: String,
    pub elapsed_secs: f32,
    pub outcome: Result<LoadOutcome, String>,
}

/// Manages the file-loading channel and all background load threads.
///
/// `loading_jobs` and `finishing_jobs` are public for the progress overlay UI.
pub(super) struct LoaderManager {
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
}

impl LoaderManager {
    pub fn new(ctx: Context) -> Self {
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
                let outcome = gt_loader::load_file_with_progress(&path, report, &config)
                    .map(|mut file| {
                        tx.send(LoadMessage::Progress {
                            id,
                            fraction: 0.95,
                            stage: STAGE_PLOTTING,
                        })
                        .ok();
                        ctx.request_repaint();
                        let series = gt_plot::prepare_file_series(0, &file);
                        // Read the bytes once for both the content fingerprint
                        // and the optional history insert.
                        let bytes = std::fs::read(&path).ok();
                        file.recording_meta = bytes
                            .as_deref()
                            .and_then(|b| gt_history::extract_meta(b).ok());
                        log::debug!("Parsed '{log_name}': {} track(s)", file.tracks.len());
                        let db_ref = store_in_history(
                            db_path.as_deref(),
                            &file,
                            &config,
                            bytes.as_deref(),
                            &log_name,
                        );
                        LoadOutcome::GtdFile {
                            file,
                            series,
                            db_ref,
                        }
                    })
                    .map_err(|e| e.to_string());
                tx.send(LoadMessage::Completed { id, outcome }).ok();
                ctx.request_repaint();
            })
            .expect("failed to spawn gtd-path loader thread");
    }

    #[expect(
        clippy::expect_used,
        reason = "thread spawn can only fail under extreme system resource exhaustion"
    )]
    pub fn spawn_gtd_bytes(
        &mut self,
        bytes: Arc<[u8]>,
        filename: String,
        config: SegmentationConfig,
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
                    gt_loader::load_bytes_with_progress(&bytes, filename, report, &config)
                        .map(|mut file| {
                            tx.send(LoadMessage::Progress {
                                id,
                                fraction: 0.95,
                                stage: STAGE_PLOTTING,
                            })
                            .ok();
                            ctx.request_repaint();
                            let series = gt_plot::prepare_file_series(0, &file);
                            file.recording_meta = gt_history::extract_meta(&bytes).ok();
                            log::debug!("Parsed '{log_name}': {} track(s)", file.tracks.len());
                            let db_ref = store_in_history(
                                db_path.as_deref(),
                                &file,
                                &config,
                                Some(&bytes),
                                &log_name,
                            );
                            LoadOutcome::GtdFile {
                                file,
                                series,
                                db_ref,
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
    pub fn spawn_log_path(
        &mut self,
        path: PathBuf,
        nav_points: Vec<NavPoint>,
        assoc_config: AssociationConfig,
    ) {
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
                let r_tx = tx.clone();
                let r_ctx = ctx.clone();
                let report = move |frac: f32, stage: &'static str| {
                    r_tx.send(LoadMessage::Progress {
                        id,
                        fraction: frac,
                        stage,
                    })
                    .ok();
                    r_ctx.request_repaint();
                };
                let source = FileSource::LogPath(path.clone());
                report(0.20, STAGE_READING);
                let content = match fs::read_to_string(&path) {
                    Ok(s) => s,
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
                finish_log_load(
                    id,
                    &filename,
                    &content,
                    &nav_points,
                    &tx,
                    &ctx,
                    report,
                    assoc_config,
                    source,
                );
            })
            .expect("failed to spawn log-path loader thread");
    }

    #[expect(
        clippy::expect_used,
        reason = "thread spawn can only fail under extreme system resource exhaustion"
    )]
    pub fn spawn_log_text(
        &mut self,
        text: String,
        filename: String,
        nav_points: Vec<NavPoint>,
        assoc_config: AssociationConfig,
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
        let source = FileSource::LogText(Arc::from(text.as_str()));
        thread::Builder::new()
            .name(format!("load-log-{filename}"))
            .spawn(move || {
                let r_tx = tx.clone();
                let r_ctx = ctx.clone();
                let report = move |frac: f32, stage: &'static str| {
                    r_tx.send(LoadMessage::Progress {
                        id,
                        fraction: frac,
                        stage,
                    })
                    .ok();
                    r_ctx.request_repaint();
                };
                finish_log_load(
                    id,
                    &filename,
                    &text,
                    &nav_points,
                    &tx,
                    &ctx,
                    report,
                    assoc_config,
                    source,
                );
            })
            .expect("failed to spawn log-text loader thread");
    }

    /// Spawn a background thread that shows the OS file-picker dialog.
    ///
    /// `rfd::FileDialog::pick_file()` blocks its calling thread.  Running it
    /// on the render thread freezes the egui loop; on Wayland the compositor
    /// then stops delivering events, making the window appear unresponsive.
    /// The dialog runs on a dedicated thread instead; the chosen path arrives
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

    /// Consume a pending file-picker result if one has arrived.
    pub fn drain_file_dialog(&mut self) -> Option<PathBuf> {
        let rx = self.file_dialog_rx.as_ref()?;
        let Ok(path_opt) = rx.try_recv() else {
            return None;
        };
        self.file_dialog_rx = None;
        path_opt
    }

    /// Drain all pending channel messages. Updates `loading_jobs` for progress
    /// messages; returns one `CompletedLoad` per finished job.
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

    /// Remove entries from `finishing_jobs` that have fully faded (> 3 s since completion).
    pub fn expire_finished(&mut self) {
        self.finishing_jobs
            .retain(|j| j.completed_at.elapsed().as_secs_f32() < 3.0);
    }
}

/// Build a `LoadedFile` from a list of custom markers produced by log parsing.
///
/// Returns `None` when `markers` is empty (nothing to display on the map).
/// This is called from background load threads and uses no egui types.
pub(super) fn build_log_loaded_file(
    filename: &str,
    markers: Vec<CustomMarker>,
    source: FileSource,
) -> Option<LoadedFile> {
    let first = markers.first()?;

    let min_lat = markers
        .iter()
        .map(|m| m.lat.as_degrees())
        .fold(first.lat.as_degrees(), f64::min);
    let max_lat = markers
        .iter()
        .map(|m| m.lat.as_degrees())
        .fold(first.lat.as_degrees(), f64::max);
    let min_lon = markers
        .iter()
        .map(|m| m.lon.as_degrees())
        .fold(first.lon.as_degrees(), f64::min);
    let max_lon = markers
        .iter()
        .map(|m| m.lon.as_degrees())
        .fold(first.lon.as_degrees(), f64::max);
    let min_time = markers.iter().map(|m| m.time).min().unwrap_or(first.time);
    let max_time = markers.iter().map(|m| m.time).max().unwrap_or(first.time);

    let count = markers.len();
    let duration = max_time.signed_duration_since(min_time);
    let filename = if filename.is_empty() {
        "log".to_owned()
    } else {
        filename.to_owned()
    };

    let bounding_box = Rect::new(
        Coord {
            x: min_lon,
            y: min_lat,
        },
        Coord {
            x: max_lon,
            y: max_lat,
        },
    );
    let track = LoadedTrack {
        // No nav points, so no LOD to build.
        lod: gt_types::TrackLod::default(),
        metadata: TrackMetadata {
            index: 0,
            distance_km: Length::new::<kilometer>(0.0),
            duration,
            time_range: TimeRange::new(min_time, max_time),
            bounding_box,
            merc_bounds: merc_bounds_for_rect(bounding_box),
            point_set_diameter_m: Length::new::<meter>(0.0),
            segment_length_range: None,
            has_custom_markers: true,
            tpv_count: 0,
            satellite_report_count: 0,
            custom_marker_count: count,
            generated_marker_count: 0,
            event_marker_count: 0,
            fix_stats: None,
        },
        points: Vec::new(),
        custom_markers: markers,
        generated_markers: Vec::new(),
        event_markers: Vec::new(),
    };

    let identity = format!("auto:{filename}");
    Some(LoadedFile {
        metadata: FileMetadata {
            filename,
            total_distance_km: Length::new::<kilometer>(0.0),
            total_duration: duration,
            time_range: TimeRange::new(min_time, max_time),
            fix_stats: None,
        },
        identity,
        tracks: vec![track],
        event_marker_styles: std::collections::HashMap::new(),
        orphaned_event_markers: Vec::new(),
        source,
        load_warnings: Vec::new(),
        db_ref: None,
        recording_meta: None,
    })
}

/// Shared tail of log-file loading: parse `content`, build a `LoadedFile`, and
/// send the `Completed` message. Called from both the path-based and text-based
/// log loader threads after file content has been obtained.
#[expect(
    clippy::too_many_arguments,
    reason = "log loading inherently needs thread ID, file context, IPC channel, progress callback, association config, and source - grouping would obscure rather than clarify"
)]
fn finish_log_load(
    id: u64,
    filename: &str,
    content: &str,
    nav_points: &[NavPoint],
    tx: &mpsc::Sender<LoadMessage>,
    ctx: &Context,
    report: impl Fn(f32, &'static str),
    assoc_config: AssociationConfig,
    source: FileSource,
) {
    report(0.55, STAGE_PARSING);
    let result = gt_logfile::load_log(content, nav_points, chrono::Utc::now(), &assoc_config);

    if result.markers.is_empty() && result.unassociated.is_empty() {
        tx.send(LoadMessage::Completed {
            id,
            outcome: Err("Unrecognised file format".to_owned()),
        })
        .ok();
        ctx.request_repaint();
        return;
    }

    report(0.90, STAGE_PROCESSING);
    let loaded = build_log_loaded_file(filename, result.markers, source);
    report(0.95, STAGE_PLOTTING);
    let series = loaded.as_ref().map(|f| gt_plot::prepare_file_series(0, f));

    tx.send(LoadMessage::Completed {
        id,
        outcome: Ok(LoadOutcome::LogFile {
            loaded,
            series,
            unassociated: result.unassociated,
        }),
    })
    .ok();
    ctx.request_repaint();
}

/// Convert the live segmentation settings into the form persisted alongside a
/// recording, so that re-opening can detect when the app's current settings
/// differ from those the stored tracks were built with.
pub(crate) fn stored_segmentation_from_config(
    config: &SegmentationConfig,
) -> gt_history::StoredSegmentation {
    gt_history::StoredSegmentation {
        track_split_gap_us: config
            .track_split_gap
            .num_microseconds()
            .unwrap_or(i64::MAX),
        detect_clock_discontinuities: config.detect_clock_discontinuities,
        clock_discontinuity_sigmas: config.clock_discontinuity_sigmas,
    }
}

/// Insert a freshly-loaded recording into the history database, logging the
/// outcome at each branch. Returns the stored reference, or `None` when storage
/// is disabled, metadata is missing, or the insert failed.
fn store_in_history(
    db_path: Option<&std::path::Path>,
    file: &LoadedFile,
    config: &SegmentationConfig,
    bytes: Option<&[u8]>,
    filename: &str,
) -> Option<gt_history::DatabaseRef> {
    use gt_history::HistoryDatabase;

    let Some(path) = db_path else {
        log::debug!("Storage disabled; not storing '{filename}' in history");
        return None;
    };
    let (Some(meta), Some(bytes)) = (file.recording_meta.as_ref(), bytes) else {
        log::warn!("No recording metadata for '{filename}'; not storing in history");
        return None;
    };

    // Track ranges from cumulative point counts: segmentation produces contiguous
    // ranges and the loader builds nav points 1:1 with the original file.
    let mut start = 0_u64;
    let tracks: Vec<gt_history::TrackRange> = file
        .tracks
        .iter()
        .map(|t| {
            let end = start + t.points.len() as u64;
            let range = gt_history::TrackRange {
                start,
                end,
                hidden: false,
            };
            start = end;
            range
        })
        .collect();
    // The cumulative end must cover exactly the recording's nav points (tracks
    // are a contiguous 1:1 partition); otherwise the derivation is unsound.
    debug_assert_eq!(
        start, meta.nav_point_count,
        "track ranges must cover all nav points"
    );
    let settings = stored_segmentation_from_config(config);

    let mut db = match gt_history::Database::open_or_create(path) {
        Ok(db) => db,
        Err(e) => {
            log::warn!("Could not open history database at {}: {e}", path.display());
            return None;
        }
    };
    match db.insert(&file.identity, meta, &tracks, settings, bytes) {
        Ok(db_ref) => {
            log::info!(
                "Stored '{filename}' in history as {}/{} ({} track(s))",
                db_ref.identity,
                db_ref.group_name,
                tracks.len()
            );
            Some(db_ref)
        }
        Err(e) => {
            log::warn!("Failed to store '{filename}' in history: {e}");
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use chrono::DateTime;
    use gt_history::{Database, HistoryDatabase};
    use gt_test_utils::{SyntheticGtdSpec, synthetic_gtd_bytes};

    use super::*;

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
    fn drain_until_complete(manager: &mut LoaderManager) -> CompletedLoad {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            if let Some(done) = manager.drain().into_iter().next() {
                return done;
            }
            assert!(Instant::now() < deadline, "load did not finish in time");
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    /// Regression: opening a `.gtd` file with storage enabled must insert it into
    /// the history database and report the resulting `db_ref`.
    #[test]
    fn loading_a_gtd_file_stores_it_in_history() {
        let dir = tempfile::tempdir().expect("temp dir");
        let gtd_path = write_sample_gtd(dir.path());
        let db_path = dir.path().join("history.h5");

        let mut manager = LoaderManager::new(egui::Context::default());
        manager.db_path = Some(db_path.clone());
        manager.spawn_gtd_path(gtd_path, SegmentationConfig::default());

        let completed = drain_until_complete(&mut manager);
        let outcome = completed.outcome.expect("load should succeed");
        let LoadOutcome::GtdFile { db_ref, .. } = outcome else {
            panic!("expected a GtdFile outcome");
        };
        assert!(
            db_ref.is_some(),
            "loading with storage enabled must produce a history db_ref"
        );

        let db = Database::open_or_create(&db_path).expect("open history db");
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

        let mut manager = LoaderManager::new(egui::Context::default());
        manager.db_path = None;
        manager.spawn_gtd_path(gtd_path, SegmentationConfig::default());

        let completed = drain_until_complete(&mut manager);
        let outcome = completed.outcome.expect("load should succeed");
        let LoadOutcome::GtdFile { db_ref, .. } = outcome else {
            panic!("expected a GtdFile outcome");
        };
        assert!(db_ref.is_none(), "no db_ref without a database path");
    }

    /// A loaded recording is stored with a per-track table, and those tracks can
    /// be hidden (what "Remove filtered data" does), surfacing in the listing so
    /// "Delete hidden data" enables.
    #[test]
    fn loaded_recording_stores_tracks_and_supports_hiding() {
        let dir = tempfile::tempdir().expect("temp dir");
        let gtd_path = write_sample_gtd(dir.path());
        let db_path = dir.path().join("history.h5");

        let mut manager = LoaderManager::new(egui::Context::default());
        manager.db_path = Some(db_path.clone());
        manager.spawn_gtd_path(gtd_path, SegmentationConfig::default());
        let completed = drain_until_complete(&mut manager);
        let LoadOutcome::GtdFile { db_ref, .. } = completed.outcome.expect("load should succeed")
        else {
            panic!("expected a GtdFile outcome");
        };
        let db_ref = db_ref.expect("loaded file must be stored in history");

        let mut db = Database::open_or_create(&db_path).expect("open db");
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
