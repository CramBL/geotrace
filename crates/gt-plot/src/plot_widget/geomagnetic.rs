//! The geomagnetic index plot lines.
//!
//! One value per fix per index: the planetary activity of the period the fix
//! falls in, read from the fix's own UTC day. Both lines break wherever no
//! value exists, the same treatment [`super::jamming`] gives a fix whose day
//! is not archived.

use std::collections::HashMap;

use chrono::DateTime;
use egui_plot::PlotPoint;
use gt_egui_mipmap::MipMap;
use gt_solar::GeomagneticIndex;
use gt_solar::activity::GeomagneticActivity;
use gt_types::{MetricKind, TrackRef};
use gt_ui_types::{ArcIdentity, GeomagneticPoint, GeomagneticSeries};

use super::chips::MetricKindUi;
use super::levels::LineViewport;
use super::lines::{
    HOVER_RADIUS_PX, LineStroke, NearestHoverLabel, PlotHoverLabel, add_line, line_runs,
    nearest_fix_under_pointer, visible_by_x,
};

/// Format of the period start in the hover label, as the index service writes
/// its period times.
const PERIOD_START_FORMAT: &str = "%Y-%m-%dT%H:%M:%S";

/// One track's index lines, rebuilt only when its source changes.
#[derive(Debug, Clone)]
pub(super) struct GeomagneticPlotCache {
    source: ArcIdentity,
    lines: [IndexLine; 2],
}

impl GeomagneticPlotCache {
    pub(super) fn lines(&self) -> &[IndexLine; 2] {
        &self.lines
    }
}

/// One index's line: its mipmapped runs, split where the line breaks, and
/// every valued fix for the hover.
#[derive(Debug, Clone)]
pub(super) struct IndexLine {
    index: GeomagneticIndex,
    runs: Vec<MipMap>,
    /// Every valued fix with the value of the period it falls in, ascending
    /// by x.
    valued: Vec<(PlotPoint, GeomagneticActivity)>,
}

impl IndexLine {
    /// The plot metric this index draws under, whose chip and color it takes.
    pub(super) fn metric_kind(&self) -> MetricKind {
        match self.index {
            GeomagneticIndex::Hp30 => MetricKind::Hp30,
            GeomagneticIndex::Kp => MetricKind::Kp,
        }
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
                lines: [
                    index_line(points, GeomagneticIndex::Hp30, |point| point.hp30),
                    index_line(points, GeomagneticIndex::Kp, |point| point.kp),
                ],
            },
        );
    }
}

/// One index's line from the track's points. A value outside what the index
/// publishes is dropped, so every hover target carries a classifiable value.
fn index_line(
    points: &[GeomagneticPoint],
    index: GeomagneticIndex,
    value: impl Fn(&GeomagneticPoint) -> Option<f64>,
) -> IndexLine {
    let activity = |point: &GeomagneticPoint| {
        value(point).and_then(|value| GeomagneticActivity::from_published_value(index, value))
    };
    let runs = line_runs(points.iter().map(|point| {
        activity(point).map(|activity| PlotPoint::new(point.x_secs, activity.value()))
    }))
    .into_iter()
    .map(|run| MipMap::build(run.iter().map(|point| [point.x, point.y]).collect()))
    .collect();
    let valued = points
        .iter()
        .filter_map(|point| {
            let activity = activity(point)?;
            Some((PlotPoint::new(point.x_secs, activity.value()), activity))
        })
        .collect();
    IndexLine {
        index,
        runs,
        valued,
    }
}

/// The hovered fix's period, worded by [`gt_solar::text::period_summary`] so
/// the plot says what the settings section says.
pub(super) struct GeomagneticHover {
    /// Track label, shown only when more than one track is visible.
    track: Option<String>,
    lines: Vec<String>,
}

impl GeomagneticHover {
    fn new(
        track_label: Option<&str>,
        index: GeomagneticIndex,
        point: PlotPoint,
        activity: GeomagneticActivity,
    ) -> Self {
        let period_start = DateTime::from_timestamp(point.x as i64, 0)
            .and_then(|time| index.period_start_covering(time))
            .map(|start| start.format(PERIOD_START_FORMAT).to_string())
            .unwrap_or_default();
        Self {
            track: track_label.map(ToOwned::to_owned),
            lines: gt_solar::text::period_summary(index, Some(activity), &period_start),
        }
    }

    pub(super) fn show(&self, ui: &mut egui::Ui) {
        ui.strong(gt_solar::text::LAYER_LABEL);
        if let Some(track) = &self.track {
            ui.label(track);
        }
        for line in &self.lines {
            ui.label(line);
        }
    }
}

#[derive(Clone, Copy)]
pub(super) struct GeomagneticTrack<'a> {
    /// The recording's plot label, `None` while a single track is visible.
    pub(super) track_label: Option<&'a str>,
    pub(super) pointer: Option<egui::Pos2>,
}

/// Draw one track's line for `line`'s index from its mipmapped runs, and
/// hit-test the pointer against its valued fixes so the hover can report the
/// period the value covers.
pub(super) fn add_geomagnetic_series<'a>(
    plot_ui: &mut egui_plot::PlotUi<'a>,
    prefix: &str,
    track: GeomagneticTrack<'_>,
    line: &'a IndexLine,
    viewport: LineViewport,
    stroke: LineStroke,
    nearest: &mut NearestHoverLabel,
) {
    for (run, selection) in line.runs.iter().zip(viewport.select_run_levels(&line.runs)) {
        add_line(
            plot_ui,
            run.slice_at(selection),
            format!("{prefix}{}", line.metric_kind().label()),
            stroke,
        );
    }

    let Some(pointer) = track.pointer else {
        return;
    };
    let visible = visible_by_x(
        &line.valued,
        |(point, _)| point.x,
        viewport.x_min,
        viewport.x_max,
    );
    if let Some((distance, &(point, activity))) = nearest_fix_under_pointer(
        plot_ui,
        visible,
        |&(point, _)| point,
        pointer,
        HOVER_RADIUS_PX,
    ) {
        nearest.offer(distance, || {
            PlotHoverLabel::Geomagnetic(GeomagneticHover::new(
                track.track_label,
                line.index,
                point,
                activity,
            ))
        });
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
        let [hp30, kp] = entry.lines();
        assert_eq!(
            (hp30.metric_kind(), kp.metric_kind()),
            (MetricKind::Hp30, MetricKind::Kp)
        );
        let spans = |line: &IndexLine| -> Vec<Option<(f64, f64)>> {
            line.runs.iter().map(MipMap::x_range).collect()
        };
        assert_eq!(spans(hp30), [Some((0.0, 1.0)), Some((3.0, 4.0))]);
        assert_eq!(spans(kp), [Some((0.0, 4.0))]);
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

    /// Every fix with a value is a hover target, carrying the value that
    /// classifies its period. Hp30 above 9 is a published value and stays one.
    #[test]
    fn valued_fixes_are_hover_targets() {
        let (track, series) = series_of(vec![
            GeomagneticPoint {
                x_secs: 0.0,
                hp30: Some(11.333),
                kp: None,
            },
            GeomagneticPoint {
                x_secs: 1.0,
                hp30: None,
                kp: None,
            },
        ]);

        let mut cache = HashMap::new();
        sync_geomagnetic_cache(&mut cache, &series);
        let entry = cache.get(&track).expect("the track is cached");
        let [hp30, kp] = entry.lines();
        assert_eq!(
            hp30.valued
                .iter()
                .map(|(point, activity)| (point.x, activity.value()))
                .collect::<Vec<_>>(),
            [(0.0, 11.333)]
        );
        assert!(kp.valued.is_empty());
    }

    /// The hover label leads with the index and its value, then the storm
    /// class, then the period the value covers.
    #[test]
    fn the_hover_label_states_the_value_its_class_and_its_period() {
        let hover = GeomagneticHover::new(
            Some("morning drive"),
            GeomagneticIndex::Hp30,
            PlotPoint::new(1_715_364_000.0, 11.333),
            GeomagneticActivity::from_published_value(GeomagneticIndex::Hp30, 11.333)
                .expect("Hp30 publishes values above 9"),
        );
        assert_eq!(
            hover.lines,
            [
                "Hp30 11.333",
                "G5 extreme storm",
                "30 minutes from 2024-05-10T18:00:00 (UTC)",
            ]
        );
    }

    /// A fix inside a Kp period reports the period's start, not its own time.
    #[test]
    fn the_hover_label_reports_the_period_the_fix_falls_in() {
        let hover = GeomagneticHover::new(
            None,
            GeomagneticIndex::Kp,
            PlotPoint::new(1_715_365_800.0, 9.0),
            GeomagneticActivity::from_published_value(GeomagneticIndex::Kp, 9.0)
                .expect("Kp is defined up to 9"),
        );
        assert_eq!(
            hover.lines.last().map(String::as_str),
            Some("3 hours from 2024-05-10T18:00:00 (UTC)")
        );
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
