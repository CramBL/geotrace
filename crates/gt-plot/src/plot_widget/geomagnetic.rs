//! The geomagnetic index plot lines.
//!
//! One value per fix per index: the planetary activity of the period the fix
//! falls in, read from the fix's own UTC day. Both lines break wherever no
//! value exists, the same treatment [`super::jamming`] gives a fix whose day
//! is not archived.

use std::collections::HashMap;

use egui_plot::PlotPoint;
use gt_egui_mipmap::MipMap;
use gt_types::{MetricKind, TrackRef};
use gt_ui_types::{ArcIdentity, GeomagneticPoint, GeomagneticSeries};

use super::chips::MetricKindUi;
use super::levels::LineViewport;
use super::lines::{LineStroke, add_line, line_runs};

/// One track's index lines, rebuilt only when its source changes.
#[derive(Debug, Clone)]
pub(super) struct GeomagneticPlotCache {
    source: ArcIdentity,
    /// Mipmapped line runs, split where the line breaks.
    hp30_runs: Vec<MipMap>,
    kp_runs: Vec<MipMap>,
}

impl GeomagneticPlotCache {
    /// Both index lines, each with the metric whose chip and color it draws
    /// under.
    pub(super) fn lines(&self) -> [(MetricKind, &[MipMap]); 2] {
        [
            (MetricKind::Hp30, &self.hp30_runs),
            (MetricKind::Kp, &self.kp_runs),
        ]
    }
}

/// Which index lines have values for the visible tracks, gating their chips.
#[derive(Debug, Clone, Copy)]
pub(super) struct GeomagneticAvailability {
    pub(super) hp30: bool,
    pub(super) kp: bool,
}

pub(super) fn geomagnetic_availability(
    visible_tracks: impl Iterator<Item = TrackRef>,
    series: &GeomagneticSeries,
) -> GeomagneticAvailability {
    let mut availability = GeomagneticAvailability {
        hp30: false,
        kp: false,
    };
    for track in visible_tracks {
        let Some(points) = series.points_by_track.get(&track) else {
            continue;
        };
        for point in points.iter() {
            availability.hp30 |= point.hp30.is_some();
            availability.kp |= point.kp.is_some();
            if availability.hp30 && availability.kp {
                return availability;
            }
        }
    }
    availability
}

/// Bring the per-track caches in line with the frame's series: drop tracks
/// that left it, rebuild entries whose source changed.
pub(super) fn sync_geomagnetic_cache(
    cache: &mut HashMap<TrackRef, GeomagneticPlotCache>,
    series: &GeomagneticSeries,
) {
    cache.retain(|track, _| series.points_by_track.contains_key(track));
    for (&track, points) in &series.points_by_track {
        let source = ArcIdentity::of(points);
        if cache
            .get(&track)
            .is_some_and(|entry| entry.source == source)
        {
            continue;
        }
        cache.insert(
            track,
            GeomagneticPlotCache {
                source,
                hp30_runs: index_runs(points, |point| point.hp30),
                kp_runs: index_runs(points, |point| point.kp),
            },
        );
    }
}

/// The mipmapped runs of one index's line.
fn index_runs(
    points: &[GeomagneticPoint],
    value: impl Fn(&GeomagneticPoint) -> Option<f64>,
) -> Vec<MipMap> {
    line_runs(
        points
            .iter()
            .map(|point| value(point).map(|value| PlotPoint::new(point.x_secs, value))),
    )
    .into_iter()
    .map(|run| MipMap::build(run.iter().map(|point| [point.x, point.y]).collect()))
    .collect()
}

/// Draw one track's line for `kind` from its mipmapped runs.
pub(super) fn add_geomagnetic_series<'a>(
    plot_ui: &mut egui_plot::PlotUi<'a>,
    prefix: &str,
    kind: MetricKind,
    runs: &'a [MipMap],
    viewport: LineViewport,
    stroke: LineStroke,
) {
    for (run, selection) in runs.iter().zip(viewport.select_run_levels(runs)) {
        add_line(
            plot_ui,
            run.slice_at(selection),
            format!("{prefix}{}", kind.label()),
            stroke,
        );
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use gt_types::{FileIdx, TrackIdx};

    use super::*;

    fn series_of(points: Vec<GeomagneticPoint>) -> (TrackRef, GeomagneticSeries) {
        let track = TrackRef::new(FileIdx::new(0), TrackIdx::new(0));
        let mut series = GeomagneticSeries::default();
        series.points_by_track.insert(track, Arc::new(points));
        (track, series)
    }

    /// Each index breaks its own line: the fix with no Hp30 value splits the
    /// Hp30 line in two while the Kp line stays whole.
    #[test]
    fn each_index_breaks_its_own_line() {
        let (track, series) = series_of(vec![
            GeomagneticPoint {
                x_secs: 0.0,
                hp30: Some(3.0),
                kp: Some(2.667),
            },
            GeomagneticPoint {
                x_secs: 1.0,
                hp30: Some(3.0),
                kp: Some(2.667),
            },
            GeomagneticPoint {
                x_secs: 2.0,
                hp30: None,
                kp: Some(2.667),
            },
            GeomagneticPoint {
                x_secs: 3.0,
                hp30: Some(4.0),
                kp: Some(2.667),
            },
            GeomagneticPoint {
                x_secs: 4.0,
                hp30: Some(4.0),
                kp: Some(2.667),
            },
        ]);

        let mut cache = HashMap::new();
        sync_geomagnetic_cache(&mut cache, &series);
        let entry = cache.get(&track).expect("the track is cached");
        let [(hp30_kind, hp30_runs), (kp_kind, kp_runs)] = entry.lines();
        assert_eq!((hp30_kind, kp_kind), (MetricKind::Hp30, MetricKind::Kp));
        let spans = |runs: &[MipMap]| -> Vec<Option<(f64, f64)>> {
            runs.iter().map(MipMap::x_range).collect()
        };
        assert_eq!(spans(hp30_runs), [Some((0.0, 1.0)), Some((3.0, 4.0))]);
        assert_eq!(spans(kp_runs), [Some((0.0, 4.0))]);
    }

    /// A track whose day archived Kp alone still offers the Kp chip, and
    /// leaves the Hp30 chip disabled.
    #[test]
    fn availability_is_reported_per_index() {
        let (track, series) = series_of(vec![GeomagneticPoint {
            x_secs: 0.0,
            hp30: None,
            kp: Some(2.667),
        }]);

        let availability = geomagnetic_availability([track].into_iter(), &series);
        assert!(availability.kp);
        assert!(!availability.hp30);
    }

    /// A track left out of the visible set enables neither chip.
    #[test]
    fn an_invisible_track_offers_no_values() {
        let (_, series) = series_of(vec![GeomagneticPoint {
            x_secs: 0.0,
            hp30: Some(3.0),
            kp: Some(2.667),
        }]);

        let availability = geomagnetic_availability(std::iter::empty(), &series);
        assert!(!availability.kp);
        assert!(!availability.hp30);
    }

    /// The cache keys on `Arc` identity: an unchanged series is reused, a
    /// replaced one rebuilds, and a track leaving the series is dropped.
    #[test]
    fn the_cache_syncs_by_arc_identity() {
        let quiet = |x_secs: f64| GeomagneticPoint {
            x_secs,
            hp30: Some(3.0),
            kp: None,
        };
        let (track, mut series) = series_of(vec![quiet(0.0), quiet(1.0)]);

        let mut cache = HashMap::new();
        sync_geomagnetic_cache(&mut cache, &series);
        let first = cache.get(&track).map(|entry| entry.source);
        assert!(first.is_some());

        sync_geomagnetic_cache(&mut cache, &series);
        assert_eq!(cache.get(&track).map(|entry| entry.source), first);

        series
            .points_by_track
            .insert(track, Arc::new(vec![quiet(0.0)]));
        sync_geomagnetic_cache(&mut cache, &series);
        assert_ne!(cache.get(&track).map(|entry| entry.source), first);

        sync_geomagnetic_cache(&mut cache, &GeomagneticSeries::default());
        assert!(cache.is_empty());
    }
}
