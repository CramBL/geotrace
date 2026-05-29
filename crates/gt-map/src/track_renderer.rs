use egui::{Color32, Response, Stroke, Ui};
use gt_types::{
    DataCategory, FileIdx, GlobalFilter, HighlightScope, LoadedFile, MapHighlight, MercBounds,
    TripDataVisibility, TripIdx, trip_passes_filter,
};
use walkers::{MapMemory, Plugin, Projector};

/// Vibrant colors assigned to trip tracks — chosen to stand out on both OSM
/// and satellite map backgrounds. The palette cycles over (file_index, trip_index)
/// using a mixing function so adjacent trips get distinct colours.
const TRACK_COLORS: [Color32; 12] = [
    Color32::from_rgb(255, 85, 0),   // vivid orange
    Color32::from_rgb(220, 20, 220), // magenta
    Color32::from_rgb(0, 210, 100),  // lime green
    Color32::from_rgb(30, 180, 255), // sky blue
    Color32::from_rgb(255, 220, 0),  // bright yellow
    Color32::from_rgb(255, 50, 110), // hot pink
    Color32::from_rgb(0, 230, 230),  // cyan
    Color32::from_rgb(200, 110, 0),  // amber
    Color32::from_rgb(155, 30, 255), // purple
    Color32::from_rgb(0, 255, 160),  // mint
    Color32::from_rgb(255, 140, 30), // golden
    Color32::from_rgb(80, 200, 255), // powder blue
];

fn trip_track_color(fi: usize, ti: usize) -> Color32 {
    // Mix file and trip indices with coprime factors so each (fi,ti) pair
    // maps to a distinct slot even for moderate numbers of files/trips.
    let idx = fi.wrapping_mul(7).wrapping_add(ti.wrapping_mul(3));
    #[expect(
        clippy::indexing_slicing,
        reason = "idx is computed via modulo so always in bounds"
    )]
    TRACK_COLORS[idx % TRACK_COLORS.len()]
}

pub struct TrackRenderer<'a> {
    files: &'a [LoadedFile],
    visibility: &'a TripDataVisibility,
    highlight: &'a MapHighlight,
    filter: &'a GlobalFilter,
    /// First file index that is considered "newly loaded"; files[new_file_boundary..]
    /// receive a blinking overlay while `blink_alpha > 0`.
    new_file_boundary: usize,
    /// Current blink intensity in [0.0, 1.0]. Zero means no overlay.
    blink_alpha: f32,
}

impl<'a> TrackRenderer<'a> {
    pub fn new(
        files: &'a [LoadedFile],
        visibility: &'a TripDataVisibility,
        highlight: &'a MapHighlight,
        filter: &'a GlobalFilter,
        new_file_boundary: usize,
        blink_alpha: f32,
    ) -> Self {
        Self {
            files,
            visibility,
            highlight,
            filter,
            new_file_boundary,
            blink_alpha,
        }
    }

    fn trip_stroke(&self, fi: FileIdx, ti: TripIdx) -> Stroke {
        if self.is_trip_highlighted(fi, ti) {
            Stroke::new(4.0, Color32::from_rgb(100, 200, 255))
        } else {
            Stroke::new(3.0, trip_track_color(fi.0, ti.0))
        }
    }

    fn is_trip_highlighted(&self, fi: FileIdx, ti: TripIdx) -> bool {
        if self
            .highlight
            .sticky
            .is_some_and(|r| r.file_index == fi && r.trip_index == ti)
        {
            return true;
        }
        match self.highlight.hover {
            Some(HighlightScope::File { file_index }) => file_index == fi,
            Some(HighlightScope::Trip {
                file_index,
                trip_index,
            }) => file_index == fi && trip_index == ti,
            Some(HighlightScope::TripCategory {
                file_index,
                trip_index,
                category,
            }) => {
                file_index == fi
                    && trip_index == ti
                    && matches!(category, DataCategory::TripTrack | DataCategory::Tpv)
            }
            Some(HighlightScope::Point(_)) | None => false,
        }
    }
}

impl Plugin for TrackRenderer<'_> {
    fn run(
        self: Box<Self>,
        ui: &mut Ui,
        _response: &Response,
        projector: &Projector,
        map_memory: &MapMemory,
    ) {
        // Build the per-frame coordinate transform once; all per-point calls are
        // then two f64 multiplies + two f64 adds with no large-value cancellation.
        let transform = crate::MercTransform::new(projector, map_memory, ui.max_rect().center());

        // Viewport bounds in Mercator space — used to skip trips that are
        // entirely outside the visible area without iterating any points.
        let view_rect = ui.max_rect();
        let vp_bounds = MercBounds {
            x_min: transform.merc_x_from_screen(view_rect.min.x),
            x_max: transform.merc_x_from_screen(view_rect.max.x),
            y_min: transform.merc_y_from_screen(view_rect.min.y),
            y_max: transform.merc_y_from_screen(view_rect.max.y),
        };

        for (fi, file) in self.files.iter().enumerate() {
            let fi = FileIdx(fi);
            let Some(file_vis) = self.visibility.files.get(fi.0) else {
                continue;
            };
            if !file_vis.enabled {
                continue;
            }
            for (ti, trip) in file.trips.iter().enumerate() {
                let ti = TripIdx(ti);
                let Some(trip_vis) = file_vis.trips.get(ti.0) else {
                    continue;
                };
                if !trip_vis.enabled || !trip_vis.track_visible {
                    continue;
                }
                if !trip_passes_filter(&trip.metadata, self.filter) {
                    continue;
                }
                // Per-trip viewport cull: if the trip's Mercator bounding box
                // does not intersect the viewport, skip it entirely.
                if !trip.metadata.merc_bounds.intersects(vp_bounds) {
                    continue;
                }
                let stroke = self.trip_stroke(fi, ti);
                let path: Vec<egui::Pos2> = trip
                    .points
                    .iter()
                    .filter(|p| gt_types::point_passes_time_filter(p.tpv.time().utc(), self.filter))
                    .map(|p| transform.to_screen(p.merc_x, p.merc_y))
                    .collect();
                if path.len() > 1 {
                    // Only clone the path when the blink overlay will also need
                    // it; otherwise move it directly into the regular stroke to
                    // avoid a per-trip Vec allocation on 99%+ of frames.
                    let need_blink = self.blink_alpha > 0.0 && fi.0 >= self.new_file_boundary;
                    let blink_path = need_blink.then(|| path.clone());
                    ui.painter().add(egui::Shape::line(path, stroke));

                    // Blink overlay: draw a bright pulsing stroke on top of
                    // newly loaded trips for the first 3 seconds after load.
                    if let Some(bp) = blink_path {
                        #[expect(
                            clippy::cast_sign_loss,
                            reason = "blink_alpha is clamped to [0,1] in NavMap::draw so product is non-negative"
                        )]
                        let blink_a = (self.blink_alpha * 200.0) as u8;
                        let blink_color = Color32::from_rgba_unmultiplied(255, 230, 80, blink_a);
                        let blink_stroke = Stroke::new(6.0, blink_color);
                        ui.painter().add(egui::Shape::line(bp, blink_stroke));
                    }
                }
            }
        }
    }
}
