use egui::{Color32, Response, Stroke, Ui};
use nav_types::{
    DataCategory, GlobalFilter, HighlightScope, LoadedFile, MapHighlight, TripDataVisibility,
    trip_passes_filter,
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

    fn trip_stroke(&self, fi: usize, ti: usize) -> Stroke {
        if self.is_trip_highlighted(fi, ti) {
            Stroke::new(4.0, Color32::from_rgb(100, 200, 255))
        } else {
            Stroke::new(3.0, trip_track_color(fi, ti))
        }
    }

    fn is_trip_highlighted(&self, fi: usize, ti: usize) -> bool {
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

        for (fi, file) in self.files.iter().enumerate() {
            let Some(file_vis) = self.visibility.files.get(fi) else {
                continue;
            };
            if !file_vis.enabled {
                continue;
            }
            for (ti, trip) in file.trips.iter().enumerate() {
                let Some(trip_vis) = file_vis.trips.get(ti) else {
                    continue;
                };
                if !trip_vis.enabled || !trip_vis.track_visible {
                    continue;
                }
                if !trip_passes_filter(&trip.metadata, self.filter) {
                    continue;
                }
                let stroke = self.trip_stroke(fi, ti);
                let path: Vec<egui::Pos2> = trip
                    .points
                    .iter()
                    .filter(|p| nav_types::point_passes_time_filter(p.tpv.time(), self.filter))
                    .map(|p| transform.to_screen(p.merc_x, p.merc_y))
                    .collect();
                if path.len() > 1 {
                    ui.painter().add(egui::Shape::line(path.clone(), stroke));

                    // Blink overlay: draw a bright pulsing stroke on top of
                    // newly loaded trips for the first 3 seconds after load.
                    if self.blink_alpha > 0.0 && fi >= self.new_file_boundary {
                        #[expect(
                            clippy::cast_sign_loss,
                            reason = "blink_alpha is clamped to [0,1] in NavMap::draw so product is non-negative"
                        )]
                        let blink_a = (self.blink_alpha * 200.0) as u8;
                        let blink_color = Color32::from_rgba_unmultiplied(255, 230, 80, blink_a);
                        let blink_stroke = Stroke::new(6.0, blink_color);
                        ui.painter().add(egui::Shape::line(path, blink_stroke));
                    }
                }
            }
        }
    }
}
