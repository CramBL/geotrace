use chrono::{DateTime, Utc};
use egui::Pos2;
use gt_types::filter;
use gt_types::{
    DataCategory, DataPointRef, FileIdx, GlobalFilter, LoadedFile, LoadedTrip, MercBounds,
    PointIdx, TripDataVisibility, TripIdx, TripVisibility,
};
use std::cell::Cell;
use std::rc::Rc;

use crate::generated_marker_renderer::update_hover_candidate;

/// Minimum interface required to cull and project a marker on the map.
///
/// Both `CustomMarker` and `GeneratedMarker` implement this through their
/// direct public fields.
pub(crate) trait MapMarkerPoint {
    fn time(&self) -> DateTime<Utc>;
    fn merc_x(&self) -> f64;
    fn merc_y(&self) -> f64;
}

impl MapMarkerPoint for gt_types::CustomMarker {
    fn time(&self) -> DateTime<Utc> {
        self.time
    }

    fn merc_x(&self) -> f64 {
        self.merc_x
    }

    fn merc_y(&self) -> f64 {
        self.merc_y
    }
}

impl MapMarkerPoint for gt_types::EventMarker {
    fn time(&self) -> DateTime<Utc> {
        self.time
    }

    fn merc_x(&self) -> f64 {
        self.merc_x
    }

    fn merc_y(&self) -> f64 {
        self.merc_y
    }
}

impl MapMarkerPoint for gt_types::GeneratedMarker {
    fn time(&self) -> DateTime<Utc> {
        self.time
    }

    fn merc_x(&self) -> f64 {
        self.merc_x
    }

    fn merc_y(&self) -> f64 {
        self.merc_y
    }
}

/// Iterates every visible, time-filtered, on-screen map marker point, calling
/// `callback(point_ref, screen_pos, point)` for each.
///
/// Handles file/trip visibility guards, the global trip filter, the time filter,
/// Mercator viewport culling, projection to screen space, and the shared
/// hover-out update.
///
/// `get_points` receives `(trip, trip_vis)` and returns
/// `Some((category, slice))` when the trip is eligible for this renderer, or
/// `None` to skip it entirely (e.g. when the category's visibility flag is off).
#[expect(
    clippy::too_many_arguments,
    reason = "renderer state cannot be grouped without inventing a one-off struct"
)]
pub(crate) fn for_each_visible_map_point<P, G, C>(
    files: &[LoadedFile],
    visibility: &TripDataVisibility,
    filter: &GlobalFilter,
    hover_out: &Rc<Cell<Option<(DataPointRef, f32)>>>,
    hover_pos: Option<Pos2>,
    transform: &crate::MercTransform,
    vp_bounds: MercBounds,
    mut get_points: G,
    mut callback: C,
) where
    P: MapMarkerPoint,
    G: for<'a> FnMut(&'a LoadedTrip, &'a TripVisibility) -> Option<(DataCategory, &'a [P])>,
    C: FnMut(DataPointRef, Pos2, &P),
{
    for (fi, file) in files.iter().enumerate() {
        let Some(file_vis) = visibility.files.get(fi) else {
            continue;
        };
        if !file_vis.enabled {
            continue;
        }
        for (ti, trip) in file.trips.iter().enumerate() {
            let Some(trip_vis) = file_vis.trips.get(ti) else {
                continue;
            };
            if !trip_vis.enabled {
                continue;
            }
            if !filter::trip_passes_filter(&trip.metadata, filter) {
                continue;
            }
            if !trip.metadata.merc_bounds.intersects(vp_bounds) {
                continue;
            }
            let Some((category, points)) = get_points(trip, trip_vis) else {
                continue;
            };
            for (pi, point) in points.iter().enumerate() {
                if !filter::point_passes_time_filter(point.time(), filter) {
                    continue;
                }
                if point.merc_x() < vp_bounds.x_min
                    || point.merc_x() > vp_bounds.x_max
                    || point.merc_y() < vp_bounds.y_min
                    || point.merc_y() > vp_bounds.y_max
                {
                    continue;
                }
                let screen_pos = transform.to_screen(point.merc_x(), point.merc_y());
                let point_ref = DataPointRef {
                    file_index: FileIdx(fi),
                    trip_index: TripIdx(ti),
                    category,
                    point_index: PointIdx(pi),
                };
                update_hover_candidate(hover_out, screen_pos, hover_pos, point_ref);
                callback(point_ref, screen_pos, point);
            }
        }
    }
}
