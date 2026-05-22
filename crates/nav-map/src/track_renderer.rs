use egui::{Color32, Response, Stroke, Ui};
use nav_types::{
    DataCategory, GlobalFilter, HighlightScope, LoadedFile, MapHighlight, TripDataVisibility,
    trip_passes_filter,
};
use uom::si::angle::degree;
use walkers::{MapMemory, Plugin, Position, Projector};

pub struct TrackRenderer<'a> {
    files: &'a [LoadedFile],
    visibility: &'a TripDataVisibility,
    highlight: &'a MapHighlight,
    filter: &'a GlobalFilter,
}

impl<'a> TrackRenderer<'a> {
    pub fn new(
        files: &'a [LoadedFile],
        visibility: &'a TripDataVisibility,
        highlight: &'a MapHighlight,
        filter: &'a GlobalFilter,
    ) -> Self {
        Self {
            files,
            visibility,
            highlight,
            filter,
        }
    }

    fn trip_stroke(&self, fi: usize, ti: usize) -> Stroke {
        if self.is_trip_highlighted(fi, ti) {
            Stroke::new(3.5, Color32::from_rgb(100, 200, 255))
        } else {
            Stroke::new(2.0, Color32::from_white_alpha(100))
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
        _map_memory: &MapMemory,
    ) {
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
                    .map(|p| {
                        let pos =
                            Position::new(p.tpv.lon().get::<degree>(), p.tpv.lat().get::<degree>());
                        projector.project(pos).to_pos2()
                    })
                    .collect();
                if path.len() > 1 {
                    ui.painter().add(egui::Shape::line(path, stroke));
                }
            }
        }
    }
}
