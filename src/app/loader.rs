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
        thread::Builder::new()
            .name(format!("load-{filename}"))
            .spawn(move || {
                let r_tx = tx.clone();
                let r_ctx = ctx.clone();
                let report = move |fraction: f32, stage: &'static str| {
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
                        let db_ref = db_path.as_deref().and_then(|p| {
                            let meta = file.recording_meta.as_ref()?;
                            let bytes = bytes.as_deref()?;
                            use gt_history::HistoryDatabase;
                            let mut db = gt_history::Database::open_or_create(p).ok()?;
                            match db.insert(&file.identity, meta, bytes) {
                                Ok(r) => Some(r),
                                Err(e) => {
                                    log::warn!("Failed to store recording in history: {e}");
                                    None
                                }
                            }
                        });
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
        thread::Builder::new()
            .name(format!("load-{filename}"))
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
                            let db_ref = db_path.as_deref().and_then(|p| {
                                let meta = file.recording_meta.as_ref()?;
                                use gt_history::HistoryDatabase;
                                let mut db = gt_history::Database::open_or_create(p).ok()?;
                                match db.insert(&file.identity, meta, &bytes) {
                                    Ok(r) => Some(r),
                                    Err(e) => {
                                        log::warn!("Failed to store recording in history: {e}");
                                        None
                                    }
                                }
                            });
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
