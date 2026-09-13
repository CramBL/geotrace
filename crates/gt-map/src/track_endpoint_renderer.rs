//! The flags at the ends of a track: a green flag at the first fix the map
//! draws for it and a chequered flag at the last.
//!
//! See [`crate::icon_mesh::FLAG_ANCHOR_OFFSET_PT`] for the anchor geometry.

use std::iter;

use egui::Color32;
use gt_filter::GlobalFilter;
use gt_types::PlacedPoints;
use smallvec::SmallVec;

use crate::icon_mesh::{
    FLAG_ANCHOR_OFFSET_PT, FLAG_HALF_EXTENTS_PT, IconId, IconInstance, IconMeshBatch,
};
use crate::transform::MercTransform;

/// The first and the last fix the map draws for one track: a placed fix whose
/// time the global filter's window holds.
#[derive(Debug, Clone, Copy)]
pub(crate) struct DrawnTrackEnds<'a> {
    placed: PlacedPoints<'a>,
    first: usize,
    last: usize,
}

impl<'a> DrawnTrackEnds<'a> {
    /// `Some` where the window holds the time of at least one fix. `first`
    /// equals `last` where it holds exactly one, and that fix then gets the
    /// start flag alone.
    ///
    /// The ends come from [`gt_filter::time_filtered_range`], which reads the
    /// fixes outside the window and stops at each end of it. A window that
    /// keeps the whole track costs two reads, and the ends are the fixes the
    /// per-fix time filter keeps even where the track's timestamps step
    /// backwards.
    pub(crate) fn of(placed: PlacedPoints<'a>, filter: &GlobalFilter) -> Option<Self> {
        let range = gt_filter::time_filtered_range(placed.fixes(), filter);
        let last = range.end.checked_sub(1)?;
        Some(Self {
            placed,
            first: range.start,
            last,
        })
    }

    /// The same ends with the finish flag moved onto the start, for a track
    /// whose whole line draws as one dot: two flags at one point would cover
    /// each other, and the start flag alone says where the track is.
    pub(crate) fn collapsed_to_the_start(self) -> Self {
        Self {
            last: self.first,
            ..self
        }
    }

    pub(crate) fn push_flags(
        self,
        batch: &mut IconMeshBatch<'_>,
        style: FlagStyle,
        transform: &MercTransform,
    ) {
        for instance in self.flag_instances(style, transform) {
            batch.push(instance);
        }
    }

    /// The instances this track's flags draw as, the start flag first. A flag
    /// whose fix sits outside [`FlagStyle::view_rect`] is left out, and so is
    /// the finish flag where `first` and `last` are one fix.
    fn flag_instances(
        self,
        style: FlagStyle,
        transform: &MercTransform,
    ) -> SmallVec<[IconInstance; 2]> {
        let start = Flag {
            icon: IconId::StartFlag,
            cloth_tint: gt_ui_theme::TRACK_START_FLAG.resolve(style.dark_mode),
            point_index: self.first,
        };
        let finish = (self.last != self.first).then_some(Flag {
            icon: IconId::FinishFlag,
            // White keeps the chequerboard baked into the asset.
            cloth_tint: Color32::WHITE,
            point_index: self.last,
        });
        iter::once(start)
            .chain(finish)
            .filter_map(|flag| flag.instance_at(self.placed, style, transform))
            .collect()
    }
}

/// What one track's flags are drawn with this frame.
#[derive(Debug, Clone, Copy)]
pub(crate) struct FlagStyle {
    /// What the icon pass culls a fix against.
    pub(crate) view_rect: egui::Rect,
    pub(crate) dark_mode: bool,
    /// Whether the map draws this track as the highlighted one, which takes
    /// its flags to [`HIGHLIGHT_SCALE`] with a pole and a cloth outline in
    /// [`gt_ui_theme::HIGHLIGHT_BLUE`].
    pub(crate) highlighted: bool,
}

/// One of the two flags: which asset it draws, the tint its cloth takes, and
/// the fix it stands at.
#[derive(Debug, Clone, Copy)]
struct Flag {
    icon: IconId,
    cloth_tint: Color32,
    point_index: usize,
}

impl Flag {
    /// `None` for a fix outside [`FlagStyle::view_rect`] and for an index past
    /// the track's fixes.
    ///
    /// A highlighted track's flag scales about the pole's foot, so both sizes
    /// stand on the same fix.
    fn instance_at(
        self,
        placed: PlacedPoints<'_>,
        style: FlagStyle,
        transform: &MercTransform,
    ) -> Option<IconInstance> {
        let screen_pos = transform.to_screen(placed.get(self.point_index)?.merc());
        if !style.view_rect.contains(screen_pos) {
            return None;
        }
        let scale = if style.highlighted {
            HIGHLIGHT_SCALE
        } else {
            1.0
        };
        // The pole and the cloth's outline are white in both themes, the
        // convention the pin and the chevrons follow. A highlighted track
        // takes them to the highlight blue, as its chevrons do.
        let outline_tint = if style.highlighted {
            gt_ui_theme::HIGHLIGHT_BLUE
        } else {
            Color32::WHITE
        };
        Some(IconInstance {
            icon: self.icon,
            center: screen_pos + FLAG_ANCHOR_OFFSET_PT * scale,
            half_extents: FLAG_HALF_EXTENTS_PT * scale,
            direction: None,
            tints: [self.cloth_tint, outline_tint],
        })
    }
}

/// How much larger a highlighted track's flags draw, which is what picks them
/// out among the flags of every other loaded track.
const HIGHLIGHT_SCALE: f32 = 2.0;

#[cfg(test)]
mod tests {
    use chrono::{DateTime, Duration, Utc};
    use gt_types::{GpsTime, Latitude, LoadedTrack, Longitude, NavPoint, TimePositionVelocity};
    use rstest::rstest;

    use super::*;

    fn epoch() -> DateTime<Utc> {
        DateTime::UNIX_EPOCH
    }

    /// A track walking east, one fix per entry of `minutes`, each stamped that
    /// many minutes after [`epoch`].
    fn a_track_stamped_at(minutes: &[i64]) -> LoadedTrack {
        let points: Vec<NavPoint> = minutes
            .iter()
            .enumerate()
            .map(|(index, &minute)| {
                let tpv = TimePositionVelocity::builder()
                    .time(GpsTime::from_utc(epoch() + Duration::minutes(minute)))
                    .lat(Latitude::new(55.0))
                    .lon(Longitude::new(12.0 + index as f64 * 0.001))
                    .build();
                NavPoint::new(tpv, None)
            })
            .collect();
        gt_test_utils::loaded_track_with_points(points)
    }

    /// A track of `count` fixes a minute apart, walking east.
    fn a_track_of(count: usize) -> LoadedTrack {
        a_track_stamped_at(&(0..count as i64).collect::<Vec<_>>())
    }

    fn window(start_minute: Option<i64>, end_minute: Option<i64>) -> GlobalFilter {
        GlobalFilter {
            time_start: start_minute.map(|minute| epoch() + Duration::minutes(minute)),
            time_end: end_minute.map(|minute| epoch() + Duration::minutes(minute)),
            ..GlobalFilter::default()
        }
    }

    /// The rect the instance cases cull against, the size of the map viewport.
    const VIEW_RECT: egui::Rect = egui::Rect {
        min: egui::pos2(0.0, 0.0),
        max: egui::pos2(800.0, 600.0),
    };

    /// A view over the fixture's first fix, at a scale that puts its five
    /// fixes about 12 px apart.
    fn a_view_over_the_first_fix() -> MercTransform {
        MercTransform::for_test_view(
            2_f64.powi(22),
            Latitude::new(55.0),
            Longitude::new(12.0),
            VIEW_RECT.center(),
        )
    }

    fn a_style(highlighted: bool) -> FlagStyle {
        FlagStyle {
            view_rect: VIEW_RECT,
            dark_mode: true,
            highlighted,
        }
    }

    /// Where the pole's foot lands for `instance`. The anchor offset scales
    /// with the instance's own extents, which is what stands a flag of either
    /// size on the same fix.
    fn pole_foot(instance: &IconInstance) -> egui::Pos2 {
        let scale = instance.half_extents.y / FLAG_HALF_EXTENTS_PT.y;
        instance.center - FLAG_ANCHOR_OFFSET_PT * scale
    }

    fn a_track_and_a_view_over_its_first_fix() -> (LoadedTrack, MercTransform) {
        (a_track_of(5), a_view_over_the_first_fix())
    }

    #[test]
    fn a_track_gets_a_start_flag_and_a_finish_flag() {
        let (track, transform) = a_track_and_a_view_over_its_first_fix();
        let placed = track.placed_points().unwrap_or_default();
        let ends = DrawnTrackEnds::of(placed, &GlobalFilter::default()).expect("five fixes");

        let icons: Vec<IconId> = ends
            .flag_instances(a_style(false), &transform)
            .iter()
            .map(|instance| instance.icon)
            .collect();

        assert_eq!(icons, vec![IconId::StartFlag, IconId::FinishFlag]);
    }

    /// The map collapses a whole track to one dot at a far enough zoom-out,
    /// and draws the start flag there on its own.
    #[test]
    fn a_track_collapsed_to_one_dot_gets_the_start_flag_alone() {
        let (track, transform) = a_track_and_a_view_over_its_first_fix();
        let placed = track.placed_points().unwrap_or_default();
        let ends = DrawnTrackEnds::of(placed, &GlobalFilter::default()).expect("five fixes");

        let collapsed = ends.collapsed_to_the_start();
        let instances = collapsed.flag_instances(a_style(false), &transform);

        assert_eq!(collapsed.last, collapsed.first);
        assert_eq!(instances.len(), 1);
        assert_eq!(
            instances.first().map(|instance| instance.icon),
            Some(IconId::StartFlag)
        );
    }

    #[test]
    fn a_highlighted_track_draws_larger_blue_flags_on_the_same_fixes() {
        let (track, transform) = a_track_and_a_view_over_its_first_fix();
        let placed = track.placed_points().unwrap_or_default();
        let ends = DrawnTrackEnds::of(placed, &GlobalFilter::default()).expect("five fixes");

        let plain = ends.flag_instances(a_style(false), &transform);
        let highlighted = ends.flag_instances(a_style(true), &transform);

        for (plain, highlighted) in plain.iter().zip(highlighted.iter()) {
            assert_eq!(plain.half_extents, FLAG_HALF_EXTENTS_PT);
            assert_eq!(
                highlighted.half_extents,
                FLAG_HALF_EXTENTS_PT * HIGHLIGHT_SCALE
            );
            assert_eq!(pole_foot(highlighted), pole_foot(plain));
            assert_eq!(plain.tints[1], Color32::WHITE);
            assert_eq!(highlighted.tints[1], gt_ui_theme::HIGHLIGHT_BLUE);
            assert_eq!(highlighted.tints[0], plain.tints[0]);
        }
    }

    /// A receiver that steps its clock back mid-track leaves the fixes in the
    /// order it wrote them, and the map draws every fix the window keeps. The
    /// flags stand at the outermost of those, past the fix stamped ahead of
    /// the window.
    #[test]
    fn the_drawn_ends_span_a_backward_time_step() {
        let track = a_track_stamped_at(&[0, 1, 10, 3, 4]);
        let placed = track.placed_points().unwrap_or_default();
        let filter = window(None, Some(5));
        let kept: Vec<usize> = placed
            .fixes()
            .iter()
            .enumerate()
            .filter(|(_, fix)| gt_filter::point_passes_time_filter(fix.tpv.time().utc(), &filter))
            .map(|(index, _)| index)
            .collect();
        assert_eq!(kept, vec![0, 1, 3, 4], "the window keeps every fix but 2");

        let ends = DrawnTrackEnds::of(placed, &filter);

        assert_eq!(ends.map(|ends| (ends.first, ends.last)), Some((0, 4)));
    }

    /// The window's own semantics are
    /// `gt-filter`'s (`the_filtered_range_agrees_with_the_point_predicate_on_time_ordered_fixes`).
    /// These cases cover what this function adds: the two ends a flag stands
    /// at, and the two shapes that leave a flag out.
    #[rstest]
    #[case::both_ends(5, window(None, None), Some((0, 4)))]
    #[case::a_window_keeping_one_fix(5, window(None, Some(0)), Some((0, 0)))]
    #[case::a_window_keeping_no_fix(5, window(Some(10), Some(20)), None)]
    #[case::a_track_of_no_fix(0, window(None, None), None)]
    fn the_drawn_ends_are_the_first_and_the_last_fix_of_the_window(
        #[case] fix_count: usize,
        #[case] filter: GlobalFilter,
        #[case] expected: Option<(usize, usize)>,
    ) {
        let track = a_track_of(fix_count);
        let placed = track.placed_points().unwrap_or_default();

        let ends = DrawnTrackEnds::of(placed, &filter);

        assert_eq!(ends.map(|ends| (ends.first, ends.last)), expected);
    }
}
