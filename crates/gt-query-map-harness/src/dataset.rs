use std::path::PathBuf;
use std::sync::Arc;

use chrono::{DateTime, Duration, Utc};
use gt_loaded_files::{FileHistory, LoadedFiles};
use gt_query_run::{JammingValues, SnapErrorValues};
use gt_track_builder::{FileMeta, SegmentationConfig};
use gt_types::coordinates::{Latitude, Longitude};
use gt_types::time_types::GpsTime;
use gt_types::tpv::TimePositionVelocity;
use gt_types::{FileIdx, FileSource, LoadedFile, NavPoint, TrackIdx, TrackRef};
use uom::si::angle::degree;
use uom::si::f64::{Angle, Velocity};
use uom::si::velocity::kilometer_per_hour;

/// Unix seconds the first point of every dataset sits at, so a scenario can
/// express the time filter in whole seconds from the start.
pub const EPOCH_SECS: i64 = 1_700_000_000;

/// Seconds between one track of a file and the next.
///
/// A file's tracks are the segments of one recording, so the only way to get two
/// of them is a gap the track builder splits on - which also means the tracks of
/// one file never overlap in time. Two recordings that do overlap are two files.
const TRACK_GAP_SECS: i64 = 600;

/// Degrees of latitude and longitude between consecutive seconds, so points a
/// second apart are a metre-scale step rather than the same spot.
const DEG_PER_SEC: f64 = 0.001;

/// Base position of every dataset, in the Copenhagen area like the other
/// fixtures.
const BASE_LAT_DEG: f64 = 55.0;
const BASE_LON_DEG: f64 = 12.0;

/// The typed address of a track, for the terse call sites a scenario wants.
pub fn track(file_index: usize, track_index: usize) -> TrackRef {
    TrackRef::new(FileIdx::new(file_index), TrackIdx::new(track_index))
}

/// The first instant of every dataset.
pub fn epoch() -> DateTime<Utc> {
    DateTime::from_timestamp(EPOCH_SECS, 0).unwrap_or(DateTime::<Utc>::UNIX_EPOCH)
}

/// One synthetic point: when it was recorded, where, and the metric values a
/// query can read.
///
/// `secs` is an offset from [`EPOCH_SECS`]; position defaults to a north-east
/// walk keyed off it. The metrics are opt-in, so a scenario states only what it
/// filters on and everything else reads as missing - which is what a receiver
/// that did not report them looks like.
#[derive(Debug, Clone, Copy)]
pub struct PointSpec {
    pub secs: i64,
    pub lat_deg: f64,
    pub lon_deg: f64,
    pub speed_kmh: Option<f64>,
    pub heading_deg: Option<f64>,
    pub eph_m: Option<f32>,
}

impl PointSpec {
    /// A point `secs` after the epoch, with a position derived from it and no
    /// velocity, heading, or accuracy.
    pub fn at_secs(secs: i64) -> Self {
        Self {
            secs,
            lat_deg: BASE_LAT_DEG + secs as f64 * DEG_PER_SEC,
            lon_deg: BASE_LON_DEG + secs as f64 * DEG_PER_SEC,
            speed_kmh: None,
            heading_deg: None,
            eph_m: None,
        }
    }

    pub fn speed_kmh(mut self, speed_kmh: f64) -> Self {
        self.speed_kmh = Some(speed_kmh);
        self
    }

    pub fn heading_deg(mut self, heading_deg: f64) -> Self {
        self.heading_deg = Some(heading_deg);
        self
    }

    pub fn eph_m(mut self, eph_m: f32) -> Self {
        self.eph_m = Some(eph_m);
        self
    }

    pub fn lat_lon(mut self, lat_deg: f64, lon_deg: f64) -> Self {
        self.lat_deg = lat_deg;
        self.lon_deg = lon_deg;
        self
    }

    fn build(self, shift_secs: i64) -> NavPoint {
        let time = GpsTime::from_utc(epoch() + Duration::seconds(self.secs + shift_secs));
        let tpv = TimePositionVelocity::builder()
            .time(time)
            .lat(Latitude::new(self.lat_deg))
            .lon(Longitude::new(self.lon_deg))
            .maybe_velocity(self.speed_kmh.map(Velocity::new::<kilometer_per_hour>))
            .maybe_heading(self.heading_deg.map(Angle::new::<degree>))
            .maybe_eph_m(self.eph_m)
            .build();
        NavPoint::new(tpv, None)
    }
}

/// One synthetic track: its points, plus the per-track series a query can read
/// that do not live in the recording (snap error, interference).
#[derive(Debug, Clone, Default)]
pub struct TrackSpec {
    points: Vec<PointSpec>,
    snap_error: Option<Vec<Option<f64>>>,
    jamming: Option<Vec<Option<f64>>>,
}

impl TrackSpec {
    pub fn from_points(points: Vec<PointSpec>) -> Self {
        Self {
            points,
            ..Self::default()
        }
    }

    /// One point per speed, a second apart - the shape most scenarios want,
    /// since `velocity` is the metric they filter on.
    pub fn from_speeds_kmh(speeds: &[f64]) -> Self {
        Self::from_points(
            speeds
                .iter()
                .enumerate()
                .map(|(i, &speed)| PointSpec::at_secs(i as i64).speed_kmh(speed))
                .collect(),
        )
    }

    /// `count` points a second apart, all at `speed_kmh`.
    pub fn steady(count: usize, speed_kmh: f64) -> Self {
        Self::from_speeds_kmh(&vec![speed_kmh; count])
    }

    /// Dense per-point snap error values, as the app hands them to a run.
    pub fn snap_error(mut self, values: Vec<Option<f64>>) -> Self {
        self.snap_error = Some(values);
        self
    }

    /// Dense per-point interference percentages, as the app hands them to a run.
    pub fn jamming(mut self, values: Vec<Option<f64>>) -> Self {
        self.jamming = Some(values);
        self
    }
}

/// One synthetic file and the tracks in it.
#[derive(Debug, Clone, Default)]
pub struct FileSpec {
    filename: String,
    tracks: Vec<TrackSpec>,
}

impl FileSpec {
    pub fn new(filename: &str) -> Self {
        Self {
            filename: filename.to_owned(),
            ..Self::default()
        }
    }

    pub fn with_tracks(filename: &str, tracks: Vec<TrackSpec>) -> Self {
        Self {
            filename: filename.to_owned(),
            tracks,
        }
    }

    pub fn track(mut self, track: TrackSpec) -> Self {
        self.tracks.push(track);
        self
    }

    /// Build the file through the real track builder, so its tracks, metadata,
    /// LOD, and markers are what loading a recording produces - and so a new
    /// field on [`LoadedFile`] never reaches this crate.
    ///
    /// The tracks are laid out [`TRACK_GAP_SECS`] apart, which is how a
    /// recording comes to hold more than one.
    fn build(&self) -> LoadedFile {
        let points: Vec<NavPoint> = self
            .tracks
            .iter()
            .enumerate()
            .flat_map(|(index, spec)| {
                let shift = index as i64 * TRACK_GAP_SECS;
                spec.points.iter().map(move |point| point.build(shift))
            })
            .collect();
        gt_track_builder::build_loaded_file(
            self.filename.clone(),
            &points,
            &[],
            Vec::new(),
            Vec::new(),
            &[],
            &SegmentationConfig::default(),
            FileSource::GtdPath(PathBuf::from(self.filename.clone())),
            FileMeta::default(),
            Vec::new(),
        )
    }
}

/// The loaded state a scenario runs against: the files, plus the per-track
/// series the app supplies alongside them.
pub struct Dataset {
    files: LoadedFiles,
    snap_errors: SnapErrorValues,
    jamming: JammingValues,
}

impl Dataset {
    /// A dataset of several files, in load order.
    pub fn of_files(specs: &[FileSpec]) -> Self {
        let mut files = LoadedFiles::new();
        let mut snap_errors = SnapErrorValues::new();
        let mut jamming = JammingValues::new();
        for (fi, spec) in specs.iter().enumerate() {
            files.push(spec.build(), FileHistory::None);
            for (ti, track_spec) in spec.tracks.iter().enumerate() {
                let track_ref = track(fi, ti);
                if let Some(values) = &track_spec.snap_error {
                    snap_errors.insert(track_ref, Arc::new(values.clone()));
                }
                if let Some(values) = &track_spec.jamming {
                    jamming.insert(track_ref, Arc::new(values.clone()));
                }
            }
        }
        Self {
            files,
            snap_errors,
            jamming,
        }
    }

    /// One file holding `tracks`, each starting a recording's worth of time
    /// after the last.
    pub fn one_file(tracks: Vec<TrackSpec>) -> Self {
        Self::of_files(&[FileSpec::with_tracks("track.gtd", tracks)])
    }

    /// One file holding one track - the shape of most scenarios.
    pub fn single_track(spec: TrackSpec) -> Self {
        Self::one_file(vec![spec])
    }

    pub fn files(&self) -> &LoadedFiles {
        &self.files
    }

    pub fn snap_errors(&self) -> &SnapErrorValues {
        &self.snap_errors
    }

    pub fn jamming(&self) -> &JammingValues {
        &self.jamming
    }

    /// Every track of every file, in tree order.
    pub fn track_refs(&self) -> Vec<TrackRef> {
        self.files
            .files()
            .iter()
            .enumerate()
            .flat_map(|(fi, file)| (0..file.tracks.len()).map(move |ti| track(fi, ti)))
            .collect()
    }

    /// `file.gtd#0`-style label of a track, the way the results panel names it.
    pub fn label(&self, track_ref: TrackRef) -> String {
        let filename = track_ref
            .fi
            .get(self.files.files())
            .map_or_else(String::new, |file| file.metadata.filename.clone());
        format!("{filename}#{}", track_ref.index)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_built_track_carries_the_metadata_loading_would() {
        let dataset = Dataset::single_track(TrackSpec::from_speeds_kmh(&[10.0, 20.0, 30.0]));
        let files = dataset.files().files();
        let loaded = track(0, 0).resolve(files).expect("the track is loaded");
        assert_eq!(loaded.points.len(), 3);
        assert_eq!(loaded.metadata.tpv_count, 3, "metadata counts the points");
        assert_eq!(
            loaded.metadata.duration,
            Duration::seconds(2),
            "one second per point"
        );
        assert!(
            loaded.metadata.distance_km.value > 0.0,
            "the points walk a real distance"
        );
        assert_eq!(dataset.label(track(0, 0)), "track.gtd#0");
    }

    /// A file's tracks are the segments of one recording, so the builder splits
    /// them apart at the gap the specs are laid out with.
    #[test]
    fn a_file_of_several_specs_becomes_that_many_tracks() {
        let dataset = Dataset::one_file(vec![
            TrackSpec::steady(3, 10.0),
            TrackSpec::steady(2, 20.0),
            TrackSpec::steady(4, 30.0),
        ]);
        let files = dataset.files().files();
        let counts: Vec<usize> = dataset
            .track_refs()
            .iter()
            .map(|&track_ref| track_ref.resolve(files).map_or(0, |t| t.points.len()))
            .collect();
        assert_eq!(counts, [3, 2, 4], "each spec keeps its own points");
    }

    /// A recording with no fixes loads as a file with no tracks: the builder
    /// segments points into non-empty tracks, so a track without points cannot
    /// exist.
    #[test]
    fn a_file_without_points_has_no_tracks() {
        let dataset = Dataset::of_files(&[FileSpec::new("silent.gtd")]);
        assert!(dataset.track_refs().is_empty());
    }

    #[test]
    fn per_track_series_land_under_their_track() {
        let dataset = Dataset::one_file(vec![
            TrackSpec::steady(2, 10.0).snap_error(vec![Some(1.0), None]),
            TrackSpec::steady(2, 10.0).jamming(vec![Some(50.0), Some(60.0)]),
        ]);
        assert!(dataset.snap_errors().contains_key(&track(0, 0)));
        assert!(!dataset.snap_errors().contains_key(&track(0, 1)));
        assert!(dataset.jamming().contains_key(&track(0, 1)));
        assert_eq!(dataset.track_refs(), [track(0, 0), track(0, 1)]);
    }
}
