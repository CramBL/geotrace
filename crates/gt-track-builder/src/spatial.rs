use gt_types::{DataCategory, FileIdx, LoadedFile, PointIdx, SpatialPoint, TrackIdx};

/// Build the global spatial index over all loaded files.
///
/// Every fix is indexed at the position it is drawn at, alongside the custom,
/// generated and event markers.
pub fn build_global_tree(files: &[LoadedFile]) -> rstar::RTree<SpatialPoint> {
    let mut points: Vec<SpatialPoint> = Vec::new();
    for (fi, file) in files.iter().enumerate() {
        let file_index = FileIdx::new(fi);
        for (ti, track) in file.tracks.iter().enumerate() {
            let track_index = TrackIdx::new(ti);
            // A track with no geometry is drawn nowhere, so none of its fixes
            // can be hit on the map.
            if let Some(placed) = track.placed_points() {
                for (pi, p) in placed.iter().enumerate() {
                    points.push(SpatialPoint {
                        merc: p.merc(),
                        file_index,
                        track_index,
                        point_index: PointIdx::new(pi),
                        category: DataCategory::Tpv,
                    });
                }
            }
            for (pi, m) in track.custom_markers.iter().enumerate() {
                points.push(SpatialPoint {
                    merc: m.merc,
                    file_index,
                    track_index,
                    point_index: PointIdx::new(pi),
                    category: DataCategory::CustomMarker,
                });
            }
            for (pi, m) in track.generated_markers.iter().enumerate() {
                points.push(SpatialPoint {
                    merc: m.merc,
                    file_index,
                    track_index,
                    point_index: PointIdx::new(pi),
                    category: DataCategory::GeneratedMarker,
                });
            }
            for (pi, m) in track.event_markers.iter().enumerate() {
                points.push(SpatialPoint {
                    merc: m.merc,
                    file_index,
                    track_index,
                    point_index: PointIdx::new(pi),
                    category: DataCategory::EventMarker,
                });
            }
        }
    }
    rstar::RTree::bulk_load(points)
}
