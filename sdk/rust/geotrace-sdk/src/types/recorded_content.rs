use crate::{Angle, Meta, NavFile, NavFix, NavPoint};

impl NavFile {
    /// Compares recorded facts, excluding SDK build stamps and NaN encodings of absent positions.
    /// Both coordinates must be NaN with an explicit state for a position to be absent.
    /// Other floating-point fields retain their ordinary equality semantics.
    pub fn equals_ignoring_build_provenance(&self, other: &Self) -> bool {
        let Self {
            meta,
            debug_time_repair,
            nav_points,
            markers,
            event_markers,
            event_marker_styles,
            channels,
        } = self;
        meta.equals_ignoring_build_provenance(&other.meta)
            && *debug_time_repair == other.debug_time_repair
            && nav_points.len() == other.nav_points.len()
            && nav_points
                .iter()
                .zip(&other.nav_points)
                .all(|(point, other)| point.equals_recorded_content(other))
            && *markers == other.markers
            && *event_markers == other.event_markers
            && *event_marker_styles == other.event_marker_styles
            && *channels == other.channels
    }
}

impl Meta {
    fn equals_ignoring_build_provenance(&self, other: &Self) -> bool {
        let Self {
            title,
            device,
            notes,
            identity,
            travel_mode,
            sdk_version: _,
            sdk_git_commit: _,
            sdk_commit_time: _,
        } = self;
        *title == other.title
            && *device == other.device
            && *notes == other.notes
            && *identity == other.identity
            && *travel_mode == other.travel_mode
    }
}

impl NavPoint {
    fn equals_recorded_content(&self, other: &Self) -> bool {
        let Self {
            fix,
            satellite_evidence,
            legacy_satellites,
        } = self;
        fix.equals_recorded_content(&other.fix)
            && *satellite_evidence == other.satellite_evidence
            && *legacy_satellites == other.legacy_satellites
    }
}

impl NavFix {
    fn equals_recorded_content(&self, other: &Self) -> bool {
        let mut recorded = *self;
        let mut compared = *other;
        if self.state.is_some()
            && other.state.is_some()
            && self.lat.as_degrees().is_nan()
            && self.lon.as_degrees().is_nan()
            && other.lat.as_degrees().is_nan()
            && other.lon.as_degrees().is_nan()
        {
            recorded.lat = Angle::degrees(0.0);
            recorded.lon = Angle::degrees(0.0);
            compared.lat = Angle::degrees(0.0);
            compared.lon = Angle::degrees(0.0);
        }
        recorded == compared
    }
}
