//! The flags at the ends of a track: a green flag at the first fix the map
//! draws for it and a chequered flag at the last, or one flag split between
//! green and chequered where the track ends within [`ENDS_MEET_RADIUS_M`] of
//! where it started.
//!
//! See [`crate::icon_mesh::FLAG_ANCHOR_OFFSET_PT`] for the anchor geometry.

use std::f32::consts::FRAC_PI_2;
use std::iter;

use egui::{Color32, Pos2, Vec2};
use gt_filter::GlobalFilter;
use gt_types::{Latitude, Longitude, PlacedPoints};
use smallvec::SmallVec;

use crate::icon_mesh::{
    self, FLAG_ANCHOR_OFFSET_PT, FLAG_CLOTH_WIDTH_PT, FLAG_HALF_EXTENTS_PT, IconId, IconInstance,
    IconMeshBatch,
};
use crate::transform::MercTransform;

/// The first and the last fix the map draws for one track: a placed fix whose
/// time the global filter's window holds.
#[derive(Clone, Copy, Debug)]
pub(crate) struct DrawnTrackEnds<'a> {
    placed: PlacedPoints<'a>,
    first: usize,
    last: usize,
    drawn_as_one_dot: bool,
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
            drawn_as_one_dot: false,
        })
    }

    /// The same ends for a track whose whole line draws as one dot, which gets
    /// the start flag alone unless its ends meet.
    pub(crate) fn collapsed_to_one_dot(self) -> Self {
        Self {
            drawn_as_one_dot: true,
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
    /// whose pole foot sits outside [`FlagStyle::view_rect`] is left out.
    fn flag_instances(
        self,
        style: FlagStyle,
        transform: &MercTransform,
    ) -> SmallVec<[IconInstance; 2]> {
        let Some(start) = self.drawn_end_at(self.first, transform) else {
            return SmallVec::new();
        };
        let finish = (self.last != self.first)
            .then(|| self.drawn_end_at(self.last, transform))
            .flatten();
        let placement = EndpointFlagPlacement::of(
            start,
            finish,
            self.drawn_as_one_dot,
            FLAG_CLOTH_WIDTH_PT * style.scale(),
        );

        let at_the_start = |icon, pose| Flag {
            end: start,
            icon,
            cloth_tint: gt_ui_theme::TRACK_START_FLAG.resolve(style.dark_mode),
            pose,
        };
        let at_the_finish = |pose| {
            finish.map(|end| Flag {
                end,
                icon: IconId::FinishFlag,
                // White keeps the chequerboard baked into the asset.
                cloth_tint: Color32::WHITE,
                pose,
            })
        };
        let (start_flag, finish_flag) = match placement {
            EndpointFlagPlacement::Leaning => (
                at_the_start(IconId::StartFlag, FlagPose::LeaningStart),
                at_the_finish(FlagPose::LeaningFinish),
            ),
            EndpointFlagPlacement::RoundTrip => {
                (at_the_start(IconId::RoundTripFlag, FlagPose::Upright), None)
            }
            EndpointFlagPlacement::StartOnly => {
                (at_the_start(IconId::StartFlag, FlagPose::Upright), None)
            }
            EndpointFlagPlacement::Upright => (
                at_the_start(IconId::StartFlag, FlagPose::Upright),
                at_the_finish(FlagPose::Upright),
            ),
        };
        iter::once(start_flag)
            .chain(finish_flag)
            .filter_map(|flag| flag.instance(style))
            .collect()
    }

    /// `None` for an index past the track's fixes.
    fn drawn_end_at(self, point_index: usize, transform: &MercTransform) -> Option<DrawnEnd> {
        let placed = self.placed.get(point_index)?;
        Some(DrawnEnd {
            position: placed.resolved_position(),
            pole_foot: transform.to_screen(placed.merc()),
        })
    }
}

/// What one track's flags are drawn with this frame.
#[derive(Clone, Copy, Debug)]
pub(crate) struct FlagStyle {
    /// What the icon pass culls a fix against.
    pub(crate) view_rect: egui::Rect,
    pub(crate) dark_mode: bool,
    /// Whether the map draws this track as the highlighted one, which takes
    /// its flags to [`HIGHLIGHT_SCALE`] with a pole and a cloth outline in
    /// [`gt_ui_theme::HIGHLIGHT_BLUE`].
    pub(crate) highlighted: bool,
}

impl FlagStyle {
    /// How much larger than [`FLAG_HALF_EXTENTS_PT`] this track's flags draw.
    fn scale(self) -> f32 {
        if self.highlighted {
            HIGHLIGHT_SCALE
        } else {
            1.0
        }
    }
}

/// Which flags a track's two drawn ends get.
#[derive(Clone, Copy, Debug)]
enum EndpointFlagPlacement {
    /// A start and a finish flag leaning away from each other, for a pair
    /// whose feet are less than one cloth width apart on screen.
    Leaning,
    /// One [`IconId::RoundTripFlag`] at the start, for a track that ends
    /// within [`ENDS_MEET_RADIUS_M`] of where it started.
    RoundTrip,
    /// The start flag alone, for a window keeping one fix and for a track
    /// drawn as one dot.
    StartOnly,
    /// A start and a finish flag, both upright.
    Upright,
}

impl EndpointFlagPlacement {
    /// A track that ends where it started gets [`Self::RoundTrip`] at every
    /// zoom. The scale affects only the choice between [`Self::Leaning`] and
    /// [`Self::Upright`].
    fn of(
        start: DrawnEnd,
        finish: Option<DrawnEnd>,
        drawn_as_one_dot: bool,
        cloth_width_pt: f32,
    ) -> Self {
        let Some(finish) = finish else {
            return Self::StartOnly;
        };
        if start.metres_to(finish) <= ENDS_MEET_RADIUS_M {
            return Self::RoundTrip;
        }
        if drawn_as_one_dot {
            return Self::StartOnly;
        }
        if start.pole_foot.distance(finish.pole_foot) < cloth_width_pt {
            Self::Leaning
        } else {
            Self::Upright
        }
    }
}

#[derive(Clone, Copy, Debug)]
struct DrawnEnd {
    position: (Latitude, Longitude),
    pole_foot: Pos2,
}

impl DrawnEnd {
    fn metres_to(self, other: Self) -> f64 {
        let (lat, lon) = self.position;
        let (other_lat, other_lon) = other.position;
        gt_geo_math::haversine_m(lat, lon, other_lat, other_lon)
    }
}

#[derive(Clone, Copy, Debug)]
struct Flag {
    end: DrawnEnd,
    icon: IconId,
    cloth_tint: Color32,
    pose: FlagPose,
}

impl Flag {
    /// `None` for a pole foot outside [`FlagStyle::view_rect`].
    fn instance(self, style: FlagStyle) -> Option<IconInstance> {
        if !style.view_rect.contains(self.end.pole_foot) {
            return None;
        }
        // The pole and the cloth's outline are white in both themes, the
        // convention the pin and the chevrons follow. A highlighted track
        // takes them to the highlight blue, as its chevrons do.
        let outline_tint = if style.highlighted {
            gt_ui_theme::HIGHLIGHT_BLUE
        } else {
            Color32::WHITE
        };
        let half_extents = self.pose.half_extents(style.scale());
        let direction = self.pose.direction();
        Some(IconInstance {
            icon: self.icon,
            center: self.end.pole_foot + anchor_offset(half_extents, direction),
            half_extents,
            direction,
            // The round trip flag's chequer cells stay white in both themes
            // and on a highlighted track.
            tints: [self.cloth_tint, outline_tint, Color32::WHITE],
        })
    }
}

/// How one flag stands. A pair under one cloth width apart on screen leans
/// apart: the start flag turns [`LEAN_DEGREES`] anticlockwise with its cloth
/// on the left of the pole, the finish flag the same angle clockwise with its
/// cloth on the right.
#[derive(Clone, Copy, Debug)]
enum FlagPose {
    LeaningFinish,
    LeaningStart,
    Upright,
}

impl FlagPose {
    /// A negative x half extent mirrors the asset about its vertical centre
    /// line, which hangs the cloth on the left of the pole.
    fn half_extents(self, scale: f32) -> Vec2 {
        match self {
            Self::LeaningStart => {
                Vec2::new(-FLAG_HALF_EXTENTS_PT.x, FLAG_HALF_EXTENTS_PT.y) * scale
            }
            Self::LeaningFinish | Self::Upright => FLAG_HALF_EXTENTS_PT * scale,
        }
    }

    fn direction(self) -> Option<Vec2> {
        // An angle past straight up turns the flag's top clockwise: screen y
        // points down.
        match self {
            Self::LeaningFinish => Some(Vec2::angled(LEAN_DEGREES.to_radians() - FRAC_PI_2)),
            Self::LeaningStart => Some(Vec2::angled(-LEAN_DEGREES.to_radians() - FRAC_PI_2)),
            Self::Upright => None,
        }
    }
}

/// Offset from a flag's pole foot to the center of the instance drawing it.
///
/// [`FLAG_ANCHOR_OFFSET_PT`] is that offset for an upright instance of
/// [`FLAG_HALF_EXTENTS_PT`]. Scaling it componentwise with the instance's own
/// extents follows the draw scale and the mirror, and the rotation matches the
/// one the batch applies to the asset's vertices.
fn anchor_offset(half_extents: Vec2, direction: Option<Vec2>) -> Vec2 {
    let upright = FLAG_ANCHOR_OFFSET_PT * half_extents / FLAG_HALF_EXTENTS_PT;
    direction.map_or(upright, |direction| {
        icon_mesh::rotate_up_to(upright, direction)
    })
}

/// How much larger a highlighted track's flags draw, which is what picks them
/// out among the flags of every other loaded track.
const HIGHLIGHT_SCALE: f32 = 2.0;

/// How far apart the two drawn ends may lie and still draw one flag.
const ENDS_MEET_RADIUS_M: f64 = 1.0;

/// How far a flag of a leaning pair turns from upright.
const LEAN_DEGREES: f32 = 22.0;

#[cfg(test)]
mod tests {
    use chrono::{DateTime, Duration, Utc};
    use gt_types::{GpsTime, LoadedTrack, NavPoint, TimePositionVelocity};
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
                    .lon(Longitude::new(12.0 + index as f64 * FIX_STEP_DEGREES))
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

    /// A track of three fixes whose last lies `metres` east of its first.
    fn a_track_returning_to_within(metres: f64) -> LoadedTrack {
        let east_of_the_start = metres / metres_per_degree_of_longitude();
        let points: Vec<NavPoint> = [0.0, 0.002, east_of_the_start]
            .into_iter()
            .enumerate()
            .map(|(index, degrees_east)| {
                let tpv = TimePositionVelocity::builder()
                    .time(GpsTime::from_utc(epoch() + Duration::minutes(index as i64)))
                    .lat(Latitude::new(55.0))
                    .lon(Longitude::new(12.0 + degrees_east))
                    .build();
                NavPoint::new(tpv, None)
            })
            .collect();
        gt_test_utils::loaded_track_with_points(points)
    }

    fn metres_per_degree_of_longitude() -> f64 {
        gt_geo_math::haversine_m(
            Latitude::new(55.0),
            Longitude::new(12.0),
            Latitude::new(55.0),
            Longitude::new(13.0),
        )
    }

    fn window(start_minute: Option<i64>, end_minute: Option<i64>) -> GlobalFilter {
        GlobalFilter {
            time_start: start_minute.map(|minute| epoch() + Duration::minutes(minute)),
            time_end: end_minute.map(|minute| epoch() + Duration::minutes(minute)),
            ..GlobalFilter::default()
        }
    }

    /// Longitude between consecutive fixes of [`a_track_stamped_at`].
    const FIX_STEP_DEGREES: f64 = 0.001;

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

    /// A view over the fixture's first fix, at the scale that leaves
    /// `points_apart` between two consecutive fixes of [`a_track_of`]. A
    /// normalised Mercator x spans 360 degrees of longitude.
    fn a_view_spacing_consecutive_fixes(points_apart: f64) -> MercTransform {
        MercTransform::for_test_view(
            points_apart * 360.0 / FIX_STEP_DEGREES,
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

    /// The pole foot of `instance`, derived from its center, half extents and
    /// direction.
    fn pole_foot(instance: &IconInstance) -> Pos2 {
        instance.center - anchor_offset(instance.half_extents, instance.direction)
    }

    fn a_track_and_a_view_over_its_first_fix() -> (LoadedTrack, MercTransform) {
        (a_track_of(5), a_view_over_the_first_fix())
    }

    fn icons_of(instances: &[IconInstance]) -> Vec<IconId> {
        instances.iter().map(|instance| instance.icon).collect()
    }

    #[test]
    fn a_track_gets_a_start_flag_and_a_finish_flag() {
        let (track, transform) = a_track_and_a_view_over_its_first_fix();
        let placed = track.placed_points().unwrap_or_default();
        let ends = DrawnTrackEnds::of(placed, &GlobalFilter::default()).expect("five fixes");

        let instances = ends.flag_instances(a_style(false), &transform);

        assert_eq!(
            icons_of(&instances),
            vec![IconId::StartFlag, IconId::FinishFlag]
        );
    }

    /// The map collapses a whole track to one dot at a far enough zoom-out,
    /// and draws the start flag there on its own.
    #[test]
    fn a_track_collapsed_to_one_dot_gets_the_start_flag_alone() {
        let (track, transform) = a_track_and_a_view_over_its_first_fix();
        let placed = track.placed_points().unwrap_or_default();
        let ends = DrawnTrackEnds::of(placed, &GlobalFilter::default()).expect("five fixes");

        let instances = ends
            .collapsed_to_one_dot()
            .flag_instances(a_style(false), &transform);

        assert_eq!(icons_of(&instances), vec![IconId::StartFlag]);
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

    #[rstest]
    #[case::within_the_radius(0.5, vec![IconId::RoundTripFlag])]
    #[case::past_the_radius(2.0, vec![IconId::StartFlag, IconId::FinishFlag])]
    fn a_track_draws_one_split_flag_within_the_ends_meet_radius_and_a_pair_past_it(
        #[case] metres_from_the_start: f64,
        #[case] expected: Vec<IconId>,
    ) {
        let track = a_track_returning_to_within(metres_from_the_start);
        let placed = track.placed_points().unwrap_or_default();
        let ends = DrawnTrackEnds::of(placed, &GlobalFilter::default()).expect("three fixes");

        let instances = ends.flag_instances(a_style(false), &a_view_over_the_first_fix());

        assert_eq!(icons_of(&instances), expected);
    }

    #[test]
    fn a_track_whose_ends_meet_draws_the_split_flag_when_it_collapses_to_one_dot() {
        let track = a_track_returning_to_within(0.5);
        let placed = track.placed_points().unwrap_or_default();
        let ends = DrawnTrackEnds::of(placed, &GlobalFilter::default()).expect("three fixes");

        let instances = ends
            .collapsed_to_one_dot()
            .flag_instances(a_style(false), &a_view_over_the_first_fix());

        assert_eq!(icons_of(&instances), vec![IconId::RoundTripFlag]);
    }

    /// A highlighted pair stands up at twice the separation the plain pair
    /// does: the threshold is one cloth width at the scale the pair draws.
    #[rstest]
    #[case::plain_and_overlapping(false, 10.0, true)]
    #[case::plain_and_clear(false, 20.0, false)]
    #[case::highlighted_and_overlapping(true, 20.0, true)]
    #[case::highlighted_and_clear(true, 40.0, false)]
    fn a_pair_of_flags_leans_while_its_feet_are_under_one_cloth_width_apart(
        #[case] highlighted: bool,
        #[case] points_between_the_feet: f64,
        #[case] leaning: bool,
    ) {
        let track = a_track_of(2);
        let placed = track.placed_points().unwrap_or_default();
        let ends = DrawnTrackEnds::of(placed, &GlobalFilter::default()).expect("two fixes");

        let instances = ends.flag_instances(
            a_style(highlighted),
            &a_view_spacing_consecutive_fixes(points_between_the_feet),
        );

        let turned = instances
            .iter()
            .filter(|instance| instance.direction.is_some())
            .count();
        assert_eq!(turned, if leaning { 2 } else { 0 });
    }

    #[test]
    fn a_leaning_pair_turns_the_start_flag_left_and_the_finish_flag_right() {
        let track = a_track_of(2);
        let placed = track.placed_points().unwrap_or_default();
        let ends = DrawnTrackEnds::of(placed, &GlobalFilter::default()).expect("two fixes");

        let instances =
            ends.flag_instances(a_style(false), &a_view_spacing_consecutive_fixes(10.0));

        let [start, finish] = instances.as_slice() else {
            panic!("a leaning pair draws two flags, drew {instances:?}");
        };
        assert!(start.direction.is_some_and(|direction| direction.x < 0.0));
        assert!(
            start.half_extents.x < 0.0,
            "the start flag hangs its cloth on the left of the pole"
        );
        assert!(finish.direction.is_some_and(|direction| direction.x > 0.0));
        assert!(finish.half_extents.x > 0.0);
    }

    #[test]
    fn a_leaning_pair_stands_on_the_two_fixes() {
        let track = a_track_of(2);
        let placed = track.placed_points().unwrap_or_default();
        let ends = DrawnTrackEnds::of(placed, &GlobalFilter::default()).expect("two fixes");
        let transform = a_view_spacing_consecutive_fixes(10.0);
        let fixes: Vec<Pos2> = placed
            .iter()
            .map(|placed| transform.to_screen(placed.merc()))
            .collect();

        let instances = ends.flag_instances(a_style(false), &transform);

        let feet: Vec<Pos2> = instances.iter().map(pole_foot).collect();
        assert_eq!(feet.len(), 2);
        for (foot, fix) in feet.iter().zip(fixes.iter()) {
            assert!((*foot - *fix).length() < 1e-3, "{foot:?} != {fix:?}");
        }
    }
}
