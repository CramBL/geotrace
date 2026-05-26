use nav_types::{Coord, CustomMarker, FileMetadata, LoadedFile, LoadedTrip, Rect, TripMetadata};
use uom::si::angle::degree;

/// State for a single in-flight background load job, shown in the progress UI.
pub struct LoadingJob {
    pub id: u64,
    pub filename: String,
    pub progress: f32,
    pub stage: &'static str,
}

/// Final result produced by a background load thread.
pub enum LoadOutcome {
    /// A successfully parsed `.nvd` / HDF5 file.
    NvdFile(LoadedFile),
    /// A successfully parsed log file; `loaded` is `None` when all entries were
    /// unassociated with any GPS track.
    LogFile {
        loaded: Option<LoadedFile>,
        unassociated: Vec<String>,
    },
}

/// Messages sent from background load threads to the UI thread via `mpsc`.
pub enum LoadMessage {
    /// Intermediate progress update — does not indicate completion.
    Progress {
        id: u64,
        fraction: f32,
        stage: &'static str,
    },
    /// The job is finished — either a usable result or an error string.
    Completed {
        id: u64,
        outcome: Result<LoadOutcome, String>,
    },
}

/// Build a `LoadedFile` from a list of custom markers produced by log parsing.
///
/// Returns `None` when `markers` is empty (nothing to display on the map).
/// This is called from background load threads and uses no egui types.
pub(super) fn build_log_loaded_file(
    filename: &str,
    markers: Vec<CustomMarker>,
) -> Option<LoadedFile> {
    let first = markers.first()?;

    let mut min_lat = first.lat.get::<degree>();
    let mut max_lat = min_lat;
    let mut min_lon = first.lon.get::<degree>();
    let mut max_lon = min_lon;
    let mut min_time = first.time;
    let mut max_time = first.time;

    for m in &markers {
        let lat = m.lat.get::<degree>();
        let lon = m.lon.get::<degree>();
        if lat < min_lat {
            min_lat = lat;
        }
        if lat > max_lat {
            max_lat = lat;
        }
        if lon < min_lon {
            min_lon = lon;
        }
        if lon > max_lon {
            max_lon = lon;
        }
        if m.time < min_time {
            min_time = m.time;
        }
        if m.time > max_time {
            max_time = m.time;
        }
    }

    let count = markers.len();
    let duration = max_time.signed_duration_since(min_time);
    let filename = if filename.is_empty() {
        "log".to_owned()
    } else {
        filename.to_owned()
    };

    let trip = LoadedTrip {
        metadata: TripMetadata {
            index: 0,
            distance_km: 0.0,
            duration,
            time_range: (min_time, max_time),
            bounding_box: Rect::new(
                Coord {
                    x: min_lon,
                    y: min_lat,
                },
                Coord {
                    x: max_lon,
                    y: max_lat,
                },
            ),
            point_set_diameter_m: 0.0,
            has_custom_markers: true,
            tpv_count: 0,
            satellite_report_count: 0,
            custom_marker_count: count,
            generated_marker_count: 0,
        },
        points: Vec::new(),
        custom_markers: markers,
        generated_markers: Vec::new(),
    };

    Some(LoadedFile {
        metadata: FileMetadata {
            filename,
            total_distance_km: 0.0,
            total_duration: duration,
            time_range: (min_time, max_time),
        },
        trips: vec![trip],
    })
}
