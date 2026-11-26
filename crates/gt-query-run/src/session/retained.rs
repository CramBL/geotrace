use std::ops::Range;

use gt_loaded_files::LoadedFilesView;
use gt_types::TrackRef;

use super::QuerySession;
use crate::results::{ChannelResults, PanelQuery, RunResults};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RetainedResultError {
    InvalidPosition,
    Stale,
    StaleTrack,
    Unavailable,
}

pub struct RetainedResults<'a> {
    results: &'a RunResults,
    files: LoadedFilesView<'a>,
}

impl QuerySession {
    /// Filter, visibility and series freshness reflect the last [Self::refresh_staleness] call.
    pub fn retained_results<'a>(
        &'a self,
        files: LoadedFilesView<'a>,
    ) -> Result<RetainedResults<'a>, RetainedResultError> {
        let completed = self
            .completed
            .as_ref()
            .ok_or(RetainedResultError::Unavailable)?;
        if !completed.fingerprint.matches_loaded_files(files) || completed.results.stale() {
            return Err(RetainedResultError::Stale);
        }
        Ok(RetainedResults {
            results: &completed.results,
            files,
        })
    }
}

impl<'a> RetainedResults<'a> {
    pub fn query(&self, ordinal: usize) -> Result<RetainedQuery<'a>, RetainedResultError> {
        let result = match self.results {
            RunResults::Channel(channel) if ordinal == 0 => QuerySource::Channel(channel),
            RunResults::Channel(_) => return Err(RetainedResultError::InvalidPosition),
            RunResults::Points(results) => QuerySource::Points(
                results
                    .queries
                    .get(ordinal)
                    .ok_or(RetainedResultError::InvalidPosition)?,
            ),
        };
        Ok(RetainedQuery {
            result,
            files: self.files,
        })
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct MatchPosition {
    pub track_ordinal: usize,
    pub match_ordinal: usize,
}

pub struct RetainedQuery<'a> {
    result: QuerySource<'a>,
    files: LoadedFilesView<'a>,
}

impl<'a> RetainedQuery<'a> {
    pub fn match_at(
        &self,
        position: MatchPosition,
    ) -> Result<RetainedMatch<'a>, RetainedResultError> {
        let (track, matches, channel) = match self.result {
            QuerySource::Channel(result) => {
                let track = result
                    .tracks
                    .get(position.track_ordinal)
                    .ok_or(RetainedResultError::InvalidPosition)?;
                (
                    track.track,
                    &track.matches,
                    Some((result.channel.as_str(), track.timeline.times.as_slice())),
                )
            }
            QuerySource::Points(query) => {
                let track = query
                    .matches
                    .get(position.track_ordinal)
                    .ok_or(RetainedResultError::InvalidPosition)?;
                (track.track, &track.matches, None)
            }
        };
        let matched = matches
            .get(position.match_ordinal)
            .ok_or(RetainedResultError::InvalidPosition)?;
        let loaded = track
            .resolve(self.files.files())
            .ok_or(RetainedResultError::StaleTrack)?;
        let extent = match channel {
            Some((name, times)) => MatchExtent::Channel {
                name,
                times: times
                    .get(matched.rows.clone())
                    .ok_or(RetainedResultError::InvalidPosition)?,
            },
            None => {
                loaded
                    .points
                    .get(matched.rows.clone())
                    .ok_or(RetainedResultError::InvalidPosition)?;
                MatchExtent::Points(matched.rows.clone())
            }
        };
        Ok(RetainedMatch { track, extent })
    }
}

enum QuerySource<'a> {
    Channel(&'a ChannelResults),
    Points(&'a PanelQuery),
}

#[derive(Debug, PartialEq)]
pub enum MatchExtent<'a> {
    Channel { name: &'a str, times: &'a [f64] },
    Points(Range<usize>),
}

pub struct RetainedMatch<'a> {
    track: TrackRef,
    extent: MatchExtent<'a>,
}

impl<'a> RetainedMatch<'a> {
    pub fn track(&self) -> TrackRef {
        self.track
    }

    pub fn extent(&self) -> &MatchExtent<'a> {
        &self.extent
    }
}

#[cfg(test)]
mod tests;
