use std::num::NonZeroUsize;

use chrono::{DateTime, Utc};
use gt_history_types::{StoredTrackSnapshot, TrackState};
use gt_loaded_files::{
    ListRecordings as ListLoadedRecordings, LoadedFileEntry, LoadedFilesView, RecordingSummary,
};
use gt_types::{GeoBounds, GpsTime, LoadedTrack, NavPoint, PointIdx, SysTime, TrackIdx, TrackRef};
use serde::{Deserialize, Serialize};
use uom::si::length::meter;
use uom::si::velocity::meter_per_second;
use uuid::Uuid;

use crate::metrics::Metrics;
use crate::protocol::{
    ErrorCode, GetRangeDetails, GetRecording, GetTrack, ListMetrics, ListRecordings,
    NumericObservation, QueryText, Recording, Recordings, Reply, TimeRange, TrackReference,
};
use crate::query_validation::QueryValidation;

pub enum InspectionRequest {
    CheckQuery(QueryText),
    GetRangeDetails(GetRangeDetails),
    GetRecording(GetRecording),
    GetTrack(GetTrack),
    ListMetrics(ListMetrics),
    ListRecordings(ListRecordings),
}

pub struct Inspection<'a> {
    files: LoadedFilesView<'a>,
    name_template: &'a str,
    session: Uuid,
    tree: Option<&'a dyn Fn(TrackRef) -> Option<TrackTreeState>>,
}

impl<'a> Inspection<'a> {
    fn current_snapshot(entry: LoadedFileEntry<'a>) -> Option<&'a StoredTrackSnapshot> {
        entry
            .history()
            .track_snapshot()
            .filter(|snapshot| snapshot.is_current_with_membership())
    }

    pub fn new(files: LoadedFilesView<'a>, name_template: &'a str, session: Uuid) -> Self {
        Self {
            files,
            name_template,
            session,
            tree: None,
        }
    }

    pub fn with_track_tree(mut self, tree: &'a dyn Fn(TrackRef) -> Option<TrackTreeState>) -> Self {
        self.tree = Some(tree);
        self
    }

    pub fn execute(&self, request: InspectionRequest) -> Result<Reply, ErrorCode> {
        match request {
            InspectionRequest::CheckQuery(args) => {
                QueryValidation::check(args.text, self.files.files())
                    .map(|validation| Reply::QueryCheck { validation })
            }
            InspectionRequest::GetRangeDetails(args) => {
                self.range(args).map(|details| Reply::Range { details })
            }
            InspectionRequest::GetRecording(args) => self
                .recording_details(args)
                .map(|details| Reply::Recording { details }),
            InspectionRequest::GetTrack(args) => {
                let (address, entry, track) = self.track(args.track)?;
                Ok(Reply::Track {
                    details: TrackDetails::new(
                        track,
                        self.track_status(address, track, Self::current_snapshot(entry)),
                    ),
                })
            }
            InspectionRequest::ListMetrics(args) => Ok(Reply::Metrics {
                page: Metrics::from_files(self.files.files(), args),
            }),
            InspectionRequest::ListRecordings(args) => {
                self.recordings(args).map(|page| Reply::Recordings { page })
            }
        }
    }

    fn recordings(
        &self,
        ListRecordings { after, limit }: ListRecordings,
    ) -> Result<Recordings, ErrorCode> {
        let after = after
            .map(|reference| reference.number_in(self.session))
            .transpose()?;
        let page = ListLoadedRecordings::new(after, limit.unwrap_or(NonZeroUsize::MAX))
            .execute(self.files);
        Ok(Recordings::new(&page, self.session, self.name_template))
    }

    fn recording_details(
        &self,
        GetRecording {
            recording,
            after,
            stored_track_start,
            limit,
        }: GetRecording,
    ) -> Result<RecordingDetails, ErrorCode> {
        let entry = recording.resolve(self.files, self.session)?;
        let start = if let Some(after) = after {
            if after.recording != recording {
                return Err(ErrorCode::StaleTrack);
            }
            after.resolve(self.files, self.session)?;
            after.index + 1
        } else {
            0
        };
        let count = entry.file().tracks.len();
        let limit = limit.map_or(MAX_TRACKS_PER_PAGE, |limit| {
            limit.get().min(MAX_TRACKS_PER_PAGE)
        });
        let end = start.saturating_add(limit).min(count);
        let snapshot = Self::current_snapshot(entry);
        let mut tracks = Vec::with_capacity(end - start);
        let mut track_states = Vec::with_capacity(end - start);
        for (index, track) in (start..end).zip(entry.file().tracks.iter().skip(start)) {
            let address = TrackRef::new(entry.file_index(), TrackIdx::new(index));
            let reference = self
                .files
                .session_track_ref(address)
                .ok_or(ErrorCode::StaleTrack)?;
            tracks.push(TrackReference::new(self.session, reference));
            track_states.push(TrackPageState {
                index,
                track_number: track.metadata.index,
                state: self.track_status(address, track, snapshot),
            });
        }
        let next_after = if end < count {
            tracks.last().copied()
        } else {
            None
        };
        Ok(RecordingDetails {
            recording: Recording::new(
                &RecordingSummary::new(entry),
                self.session,
                self.name_template,
            ),
            tracks,
            track_states,
            next_after,
            stored_tracks: snapshot
                .map(|snapshot| StoredTrackPage::new(snapshot, stored_track_start, limit)),
        })
    }

    fn track_status(
        &self,
        address: TrackRef,
        track: &LoadedTrack,
        snapshot: Option<&StoredTrackSnapshot>,
    ) -> TrackInspectionState {
        let stored_state = track
            .metadata
            .index
            .checked_sub(1)
            .and_then(|row| snapshot?.rows().get(row))
            .filter(|range| range.state != TrackState::Deleted)
            .map(|range| range.state.into());
        TrackInspectionState {
            tree: self.tree.and_then(|tree| tree(address)),
            stored_state,
        }
    }

    fn range(
        &self,
        GetRangeDetails {
            track,
            start,
            end,
            limit,
        }: GetRangeDetails,
    ) -> Result<RangeDetails, ErrorCode> {
        let (_, _, track) = self.track(track)?;
        let start = start.unwrap_or_default();
        let end = end.unwrap_or(track.points.len());
        if end > track.points.len() {
            return Err(ErrorCode::InvalidRange);
        }
        let limit = limit.map_or(MAX_POINTS_PER_PAGE, |limit| {
            limit.get().min(MAX_POINTS_PER_PAGE)
        });
        let page_end = start.saturating_add(limit).min(end);
        let page = track
            .points
            .get(start..page_end)
            .ok_or(ErrorCode::InvalidRange)?;
        Ok(RangeDetails {
            points: (start..page_end)
                .zip(page)
                .map(|(index, point)| PointDetails::new(PointIdx::new(index), point))
                .collect(),
            next_start: (page_end < end).then_some(PointIdx::new(page_end)),
        })
    }

    fn track(
        &self,
        reference: TrackReference,
    ) -> Result<(TrackRef, LoadedFileEntry<'a>, &'a LoadedTrack), ErrorCode> {
        let address = reference.resolve(self.files, self.session)?;
        let entry = self
            .files
            .entry_for(address.fi)
            .ok_or(ErrorCode::StaleTrack)?;
        let track = address
            .index
            .get(&entry.file().tracks)
            .ok_or(ErrorCode::StaleTrack)?;
        Ok((address, entry, track))
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct RecordingDetails {
    pub recording: Recording,
    pub tracks: Vec<TrackReference>,
    pub track_states: Vec<TrackPageState>,
    pub next_after: Option<TrackReference>,
    pub stored_tracks: Option<StoredTrackPage>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct TrackPageState {
    pub index: usize,
    pub track_number: usize,
    #[serde(flatten)]
    pub state: TrackInspectionState,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct TrackInspectionState {
    pub tree: Option<TrackTreeState>,
    pub stored_state: Option<StoredTrackState>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct TrackTreeState {
    pub enabled: bool,
    pub selected: bool,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum StoredTrackState {
    Deleted,
    Live,
    Shelved,
}

impl From<TrackState> for StoredTrackState {
    fn from(value: TrackState) -> Self {
        match value {
            TrackState::Deleted => Self::Deleted,
            TrackState::Live => Self::Live,
            TrackState::Shelved => Self::Shelved,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct StoredTrackPage {
    pub rows: Vec<StoredTrackRow>,
    pub next_start: Option<NonZeroUsize>,
}

impl StoredTrackPage {
    fn new(snapshot: &StoredTrackSnapshot, start: Option<NonZeroUsize>, limit: usize) -> Self {
        let rows = snapshot.rows();
        let start = (start.unwrap_or(NonZeroUsize::MIN).get() - 1).min(rows.len());
        let end = start.saturating_add(limit).min(rows.len());
        Self {
            rows: (start + 1..=end)
                .zip(rows.iter().skip(start))
                .map(|(track_number, row)| StoredTrackRow {
                    track_number,
                    state: row.state.into(),
                })
                .collect(),
            next_start: (end < rows.len())
                .then(|| NonZeroUsize::new(end + 1))
                .flatten(),
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct StoredTrackRow {
    pub track_number: usize,
    pub state: StoredTrackState,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Serialize)]
pub struct TrackDetails {
    pub track_number: usize,
    pub time_range: TimeRange,
    pub duration_seconds: f64,
    pub distance_m: Option<f64>,
    pub map_bounds: Option<MapBounds>,
    pub point_count: usize,
    pub invalid_position_count: usize,
    pub satellite_report_count: usize,
    pub fix_loss_count: Option<u32>,
    #[serde(flatten)]
    pub state: TrackInspectionState,
}

impl TrackDetails {
    fn new(track: &LoadedTrack, state: TrackInspectionState) -> Self {
        let metadata = &track.metadata;
        let geometry = track.geometry.measured();
        Self {
            track_number: metadata.index,
            time_range: metadata.time_range.into(),
            duration_seconds: metadata.duration.as_seconds_f64(),
            distance_m: geometry.map(|geometry| geometry.distance_km.get::<meter>()),
            map_bounds: geometry.map(|geometry| geometry.bounding_box.into()),
            point_count: track.points.len(),
            invalid_position_count: metadata.invalid_position_count,
            satellite_report_count: metadata.satellite_report_count,
            fix_loss_count: metadata.fix_stats.map(|stats| stats.fix_loss_count),
            state,
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Serialize)]
pub struct MapBounds {
    pub south_deg: f64,
    pub north_deg: f64,
    pub longitude_start_deg: f64,
    pub longitude_span_deg: f64,
}

impl From<GeoBounds> for MapBounds {
    fn from(GeoBounds { lat, lon }: GeoBounds) -> Self {
        Self {
            south_deg: lat.south().as_degrees(),
            north_deg: lat.north().as_degrees(),
            longitude_start_deg: lon.start().as_degrees(),
            longitude_span_deg: lon.span_degrees(),
        }
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct RangeDetails {
    pub points: Vec<PointDetails>,
    pub next_start: Option<PointIdx>,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Serialize)]
pub struct PointDetails {
    pub index: PointIdx,
    pub time: DateTime<Utc>,
    pub gps_time: Option<DateTime<Utc>>,
    pub system_time: Option<DateTime<Utc>>,
    pub position: Option<Position>,
    pub eph_m: NumericObservation,
    pub velocity_mps: NumericObservation,
}

impl PointDetails {
    fn new(index: PointIdx, point: &NavPoint) -> Self {
        let tpv = &point.tpv;
        Self {
            index,
            time: tpv.time().utc(),
            gps_time: tpv.gps_time().map(GpsTime::utc),
            system_time: tpv.sys_time().map(SysTime::utc),
            position: tpv.position().map(|(latitude, longitude)| Position {
                latitude_deg: latitude.as_degrees(),
                longitude_deg: longitude.as_degrees(),
            }),
            eph_m: tpv.eph_m().map(f64::from).into(),
            velocity_mps: tpv
                .velocity()
                .map(|velocity| velocity.get::<meter_per_second>())
                .into(),
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Serialize)]
pub struct Position {
    pub latitude_deg: f64,
    pub longitude_deg: f64,
}

pub const MAX_POINTS_PER_PAGE: usize = 100;
pub const MAX_TRACKS_PER_PAGE: usize = 100;
