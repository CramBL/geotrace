use gt_types::{DataCategory, FileIdx, LoadedFile, PointIdx, SpatialPoint, TrackIdx};

/// The map's spatial index over the loaded files: one R-tree of every track's
/// fixes and one of its custom, generated and event markers.
///
/// Every fix is indexed at the position it is drawn at.
#[derive(Default)]
pub struct SpatialIndex {
    pub fixes: rstar::RTree<SpatialPoint>,
    pub markers: rstar::RTree<SpatialPoint>,
}

impl SpatialIndex {
    pub fn build(files: &[LoadedFile]) -> Self {
        let mut fixes: Vec<SpatialPoint> = Vec::new();
        let mut markers: Vec<SpatialPoint> = Vec::new();
        for (fi, file) in files.iter().enumerate() {
            let file_index = FileIdx::new(fi);
            for (ti, track) in file.tracks.iter().enumerate() {
                let track_index = TrackIdx::new(ti);
                let indexed = |merc, point_index, category| SpatialPoint {
                    merc,
                    file_index,
                    track_index,
                    point_index: PointIdx::new(point_index),
                    category,
                };
                // A track with no geometry is drawn nowhere, so none of its
                // fixes can be hit on the map.
                if let Some(placed) = track.placed_points() {
                    for (pi, p) in placed.iter().enumerate() {
                        fixes.push(indexed(p.merc(), pi, DataCategory::Tpv));
                    }
                }
                for (pi, m) in track.custom_markers.iter().enumerate() {
                    markers.push(indexed(m.merc, pi, DataCategory::CustomMarker));
                }
                for (pi, m) in track.generated_markers.iter().enumerate() {
                    markers.push(indexed(m.merc, pi, DataCategory::GeneratedMarker));
                }
                for (pi, m) in track.event_markers.iter().enumerate() {
                    markers.push(indexed(
                        m.resolved_position.merc(),
                        pi,
                        DataCategory::EventMarker,
                    ));
                }
            }
        }
        Self {
            fixes: rstar::RTree::bulk_load(fixes),
            markers: rstar::RTree::bulk_load(markers),
        }
    }

    /// Every indexed point, the fixes ahead of the markers.
    pub fn points(&self) -> impl Iterator<Item = &SpatialPoint> {
        self.fixes.iter().chain(self.markers.iter())
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use chrono::{DateTime, Duration, Utc};
    use gt_types::coordinates::{Latitude, Longitude};
    use gt_types::markers::{CustomMarker, EventMarker, MarkerIcon};
    use gt_types::nav_point::NavPoint;
    use gt_types::satellites::{Constellation, Satellite, Satellites};
    use gt_types::time_types::GpsTime;
    use gt_types::tpv::TimePositionVelocity;
    use gt_types::track::FileSource;
    use uom::si::angle::degree;
    use uom::si::f64::Angle;

    use super::*;
    use crate::segment::{FileMeta, SegmentationConfig};

    const LATITUDE_DEGREES: f64 = 55.0;

    fn at(second: i64) -> DateTime<Utc> {
        DateTime::<Utc>::UNIX_EPOCH + Duration::seconds(second)
    }

    /// The builder places this fix where the receiver wrote it and generates
    /// no marker for it: it has a heading and a full solution behind it, as a
    /// receiver-measured fix would.
    fn measured_fix(second: i64, lon_degrees: f64) -> NavPoint {
        let time = GpsTime::from_utc(at(second));
        let satellites = (1..=12)
            .map(|prn| Satellite::new(Constellation::Gps, prn, None, None, None, true))
            .collect();
        NavPoint::new(
            TimePositionVelocity::builder()
                .time(time)
                .lat(Latitude::new(LATITUDE_DEGREES))
                .lon(Longitude::new(lon_degrees))
                .heading(Angle::new::<degree>(90.0))
                .build(),
            Some(Satellites::new(Some(time), None, satellites)),
        )
    }

    /// One track of three fixes, with a custom and an event marker on the
    /// first of them.
    fn a_file_with_fixes_and_two_markers() -> Vec<LoadedFile> {
        let lat = Latitude::new(LATITUDE_DEGREES);
        let lon = Longitude::new(12.0);
        vec![crate::segment::build_loaded_file(
            "index.gtd".to_owned(),
            &[
                measured_fix(0, 12.0),
                measured_fix(1, 12.001),
                measured_fix(2, 12.002),
            ],
            &[CustomMarker::new(
                at(0),
                "note".to_owned(),
                MarkerIcon::Pin,
                lat,
                lon,
            )],
            vec![EventMarker::new(
                at(0),
                "power/boot".to_owned(),
                None,
                lat,
                lon,
            )],
            vec![],
            &[],
            &SegmentationConfig::default(),
            FileSource::GtdPath(PathBuf::from("index.gtd")),
            FileMeta::default(),
            vec![],
        )]
    }

    #[test]
    fn the_fix_tree_holds_every_fix_and_no_marker() {
        let index = SpatialIndex::build(&a_file_with_fixes_and_two_markers());

        assert_eq!(
            index.fixes.iter().map(|sp| sp.category).collect::<Vec<_>>(),
            vec![DataCategory::Tpv; 3]
        );
    }

    #[test]
    fn the_marker_tree_holds_every_marker_and_no_fix() {
        let index = SpatialIndex::build(&a_file_with_fixes_and_two_markers());
        let mut categories: Vec<DataCategory> =
            index.markers.iter().map(|sp| sp.category).collect();
        categories.sort();

        assert_eq!(
            categories,
            vec![DataCategory::CustomMarker, DataCategory::EventMarker]
        );
    }
}
