use crate::{Angle, Channel, Meta, NavFile, NavFix, NavPoint};

impl NavFile {
    /// Compares recorded facts, excluding SDK build stamps.
    /// An absent position requires two NaN coordinates, explicit state and unreported origin.
    /// Paired absent positions and paired missing channel samples compare equal.
    /// Derived `PartialEq` and other floating-point fields retain ordinary IEEE equality.
    pub fn equals_ignoring_build_provenance(&self, other: &Self) -> bool {
        let Self {
            meta,
            debug_time_repair,
            nav_points,
            markers,
            event_markers,
            event_marker_styles,
            channels,
            bundled_logs,
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
            && channels.len() == other.channels.len()
            && channels
                .iter()
                .zip(&other.channels)
                .all(|(channel, other)| channel.equals_recorded_content(other))
            && *bundled_logs == other.bundled_logs
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
        if self.position_provenance.is_none()
            && other.position_provenance.is_none()
            && self.state.is_some()
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

impl Channel {
    #[expect(clippy::float_cmp, reason = "Recorded samples use exact equality")]
    fn equals_recorded_content(&self, other: &Self) -> bool {
        let Self {
            name,
            unit,
            period,
            description,
            components,
            times,
            values,
        } = self;
        *name == other.name
            && *unit == other.unit
            && *period == other.period
            && *description == other.description
            && *components == other.components
            && *times == other.times
            && values.len() == other.values.len()
            && values
                .iter()
                .zip(&other.values)
                .all(|(sample, other)| sample == other || (sample.is_nan() && other.is_nan()))
    }
}
