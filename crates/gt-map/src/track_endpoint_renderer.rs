//! The flags at the ends of a track: a green flag at the first fix the map
//! draws for it and a chequered flag at the last, or one flag split between
//! green and chequered where the track ends within [`ENDS_MEET_RADIUS_M`] of
//! where it started.
//!
//! The flags of two or more tracks within one cloth width of each other draw
//! as one cluster flag with a count. The highlighted track's flags stay out
//! of every cluster: hovering a track draws its own flags at their own
//! position and size.
//!
//! See [`crate::icon_mesh::FLAG_ANCHOR_OFFSET_PT`] for the anchor geometry.

use std::f32::consts::FRAC_PI_2;
use std::iter;
use std::num::NonZeroUsize;

use egui::{Align2, Color32, FontId, Pos2, Ui, Vec2};
use gt_filter::GlobalFilter;
use gt_types::{Latitude, Longitude, MercBounds, MercPoint, PlacedPoints, TrackRef};
use smallvec::SmallVec;

use crate::collision_grid;
use crate::icon_mesh::{
    self, FLAG_ANCHOR_OFFSET_PT, FLAG_CLOTH_TOP_RIGHT_PT, FLAG_CLOTH_WIDTH_PT,
    FLAG_HALF_EXTENTS_PT, IconId, IconInstance, IconMeshBatch, IconMeshLibrary,
};
use crate::text_badge::{BadgePlateHeight, TextBadge};
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

    /// The flags at this track's ends, the start flag first.
    pub(crate) fn flags(
        self,
        style: FlagStyle,
        transform: &MercTransform,
    ) -> SmallVec<[StandingFlag; 2]> {
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

        let at_the_start = |ends, pose| StandingFlag {
            merc: start.merc,
            ends,
            pose,
        };
        let at_the_finish = |pose| {
            finish.map(|end| StandingFlag {
                merc: end.merc,
                ends: FlagEnds::Finish,
                pose,
            })
        };
        let (start_flag, finish_flag) = match placement {
            EndpointFlagPlacement::Leaning => (
                at_the_start(FlagEnds::Start, FlagPose::LeaningStart),
                at_the_finish(FlagPose::LeaningFinish),
            ),
            EndpointFlagPlacement::RoundTrip => (
                at_the_start(FlagEnds::StartAndFinish, FlagPose::Upright),
                None,
            ),
            EndpointFlagPlacement::StartOnly => {
                (at_the_start(FlagEnds::Start, FlagPose::Upright), None)
            }
            EndpointFlagPlacement::Upright => (
                at_the_start(FlagEnds::Start, FlagPose::Upright),
                at_the_finish(FlagPose::Upright),
            ),
        };
        iter::once(start_flag).chain(finish_flag).collect()
    }

    /// `None` for an index past the track's fixes.
    fn drawn_end_at(self, point_index: usize, transform: &MercTransform) -> Option<DrawnEnd> {
        let placed = self.placed.get(point_index)?;
        Some(DrawnEnd {
            position: placed.resolved_position(),
            merc: placed.merc(),
            pole_foot: transform.to_screen(placed.merc()),
        })
    }
}

/// The flags of every track but the highlighted one, as this frame placed
/// them: gathered while each track draws, and painted together once every
/// track has placed its own.
///
/// A steady stream of frames reuses the buffer's allocation: the map holds
/// the buffer across frames.
#[derive(Default)]
pub(crate) struct PendingEndpointFlags {
    flags: Vec<TrackEndFlag>,
}

impl PendingEndpointFlags {
    pub(crate) fn clear(&mut self) {
        self.flags.clear();
    }

    pub(crate) fn push_flags_of_track(
        &mut self,
        track: TrackRef,
        flags: impl IntoIterator<Item = StandingFlag>,
    ) {
        self.flags
            .extend(flags.into_iter().map(|flag| TrackEndFlag { track, flag }));
    }

    /// Paints one flag per place: a cluster flag with a count where two or
    /// more tracks meet, and every other flag where its own track placed it.
    pub(crate) fn paint(
        &self,
        ui: &Ui,
        view_rect: egui::Rect,
        transform: &MercTransform,
        icon_meshes: Option<&IconMeshLibrary>,
    ) {
        let style = FlagStyle {
            view_rect,
            dark_mode: ui.visuals().dark_mode,
            highlighted: false,
        };
        let cloth_width_merc = f64::from(FLAG_CLOTH_WIDTH_PT) / transform.px_per_merc();
        let grouped = group_flags_by_position(
            &self.flags,
            cloth_width_merc,
            transform.viewport_merc_bounds(view_rect),
        );
        let mut batch = IconMeshBatch::gpu_when_available(ui, icon_meshes);
        for grouped_flag in &grouped {
            if let Some(instance) = grouped_flag.flag().instance(style, transform) {
                batch.push(instance);
            }
        }
        batch.paint(ui.painter());
        // The counts go on after the batch, which puts each badge above every
        // flag the batch drew.
        for grouped_flag in &grouped {
            if let GroupedFlag::Cluster { flag, count } = *grouped_flag
                && let Some(pole_foot) = flag.pole_foot(style, transform)
            {
                draw_cluster_count(ui, pole_foot + FLAG_CLOTH_TOP_RIGHT_PT, count);
            }
        }
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

/// One flag standing on the map: a track's own, or the one the flags of
/// several tracks at one place draw as.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct StandingFlag {
    merc: MercPoint,
    ends: FlagEnds,
    pose: FlagPose,
}

impl StandingFlag {
    fn standing_upright(self) -> Self {
        Self {
            pose: FlagPose::Upright,
            ..self
        }
    }

    /// `None` for a pole foot outside [`FlagStyle::view_rect`].
    fn instance(self, style: FlagStyle, transform: &MercTransform) -> Option<IconInstance> {
        let pole_foot = self.pole_foot(style, transform)?;
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
            icon: self.ends.icon(),
            center: pole_foot + anchor_offset(half_extents, direction),
            half_extents,
            direction,
            // The round trip flag's chequer cells stay white in both themes
            // and on a highlighted track.
            tints: [
                self.ends.cloth_tint(style.dark_mode),
                outline_tint,
                Color32::WHITE,
            ],
        })
    }

    /// `None` for a pole foot outside [`FlagStyle::view_rect`].
    fn pole_foot(self, style: FlagStyle, transform: &MercTransform) -> Option<Pos2> {
        let pole_foot = transform.to_screen(self.merc);
        style.view_rect.contains(pole_foot).then_some(pole_foot)
    }
}

/// One track's flag as this frame placed it. The cluster pass reads the
/// [`TrackRef`]: a group of one track's own flags draws each of them where
/// that track placed it.
#[derive(Clone, Copy, Debug)]
struct TrackEndFlag {
    track: TrackRef,
    flag: StandingFlag,
}

/// Which ends of its track one flag stands for.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum FlagEnds {
    Finish,
    Start,
    StartAndFinish,
}

impl FlagEnds {
    /// The ends of two flags merged at one place: a start merged with a
    /// finish becomes [`Self::StartAndFinish`], which draws the split cloth.
    fn merged_with(self, other: Self) -> Self {
        if self == other {
            self
        } else {
            Self::StartAndFinish
        }
    }

    fn icon(self) -> IconId {
        match self {
            Self::Finish => IconId::FinishFlag,
            Self::Start => IconId::StartFlag,
            Self::StartAndFinish => IconId::RoundTripFlag,
        }
    }

    /// White keeps the chequerboard the finish flag's asset bakes in.
    fn cloth_tint(self, dark_mode: bool) -> Color32 {
        match self {
            Self::Finish => Color32::WHITE,
            Self::Start | Self::StartAndFinish => gt_ui_theme::TRACK_START_FLAG.resolve(dark_mode),
        }
    }
}

/// What the flags at one place draw as.
#[derive(Clone, Copy, Debug, PartialEq)]
enum GroupedFlag {
    /// The flags of two or more tracks within one cloth width of each other,
    /// as one flag stating how many it stands for.
    Cluster {
        flag: StandingFlag,
        count: NonZeroUsize,
    },
    /// One track's flag, where that track placed it.
    Loose(StandingFlag),
}

impl GroupedFlag {
    fn flag(self) -> StandingFlag {
        match self {
            Self::Cluster { flag, .. } | Self::Loose(flag) => flag,
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
    merc: MercPoint,
    pole_foot: Pos2,
}

impl DrawnEnd {
    fn metres_to(self, other: Self) -> f64 {
        let (lat, lon) = self.position;
        let (other_lat, other_lon) = other.position;
        gt_geo_math::haversine_m(lat, lon, other_lat, other_lon)
    }
}

/// How one flag stands. A pair under one cloth width apart on screen leans
/// apart: the start flag turns [`LEAN_DEGREES`] anticlockwise with its cloth
/// on the left of the pole, the finish flag the same angle clockwise with its
/// cloth on the right.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
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

/// Paints `flags` as one batch, which is how the highlighted track's flags
/// draw on the map: they stay out of every cluster and keep their own size.
pub(crate) fn paint_flags(
    ui: &Ui,
    flags: &[StandingFlag],
    style: FlagStyle,
    transform: &MercTransform,
    icon_meshes: Option<&IconMeshLibrary>,
) {
    let mut batch = IconMeshBatch::gpu_when_available(ui, icon_meshes);
    for flag in flags {
        if let Some(instance) = flag.instance(style, transform) {
            batch.push(instance);
        }
    }
    batch.paint(ui.painter());
}

/// Groups the flags several tracks placed: those of two or more tracks within
/// `spacing_merc` of each other into one cluster flag, and every other flag as
/// its own track placed it.
///
/// A flag left alone in its group stands upright. Its leaning partner joined
/// a cluster, and nothing stands beside it to lean away from. A pair that
/// leans is a pair that groups together: the two thresholds are the same
/// width.
fn group_flags_by_position(
    flags: &[TrackEndFlag],
    spacing_merc: f64,
    viewport: MercBounds,
) -> Vec<GroupedFlag> {
    let mut grouped: Vec<GroupedFlag> = Vec::new();
    for cluster in collision_grid::cluster_positions(
        flags.iter().map(|flag| flag.flag.merc),
        spacing_merc,
        viewport,
    ) {
        let mut members = cluster.members.iter().filter_map(|&index| flags.get(index));
        let Some(first) = members.next() else {
            continue;
        };
        let mut ends = first.flag.ends;
        let mut count = NonZeroUsize::MIN;
        let mut one_track_placed_them_all = true;
        for member in members {
            ends = ends.merged_with(member.flag.ends);
            count = count.saturating_add(1);
            one_track_placed_them_all &= member.track == first.track;
        }
        if !one_track_placed_them_all {
            grouped.push(GroupedFlag::Cluster {
                flag: StandingFlag {
                    merc: cluster.merc,
                    ends,
                    pose: FlagPose::Upright,
                },
                count,
            });
            continue;
        }
        let alone = count == NonZeroUsize::MIN;
        grouped.extend(
            cluster
                .members
                .iter()
                .filter_map(|&index| flags.get(index))
                .map(|member| {
                    GroupedFlag::Loose(if alone {
                        member.flag.standing_upright()
                    } else {
                        member.flag
                    })
                }),
        );
    }
    grouped
}

/// The badge on a cluster flag's cloth, stating how many flags it stands for.
///
/// The backplate takes the height of the digits: the leading of the laid-out
/// text would cover the cloth under it.
fn draw_cluster_count(ui: &Ui, cloth_corner: Pos2, count: NonZeroUsize) {
    TextBadge {
        text: count.to_string(),
        font: FontId::monospace(COUNT_BADGE_FONT_PX),
        text_color: Color32::WHITE,
        fill: COUNT_BADGE_FILL,
        padding_pt: COUNT_BADGE_PADDING_PT,
        corner_radius_pt: COUNT_BADGE_CORNER_RADIUS_PT,
        plate_height: BadgePlateHeight::Font,
    }
    .draw(ui, cloth_corner, Align2::CENTER_CENTER);
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

/// Height of the count on a cluster flag's badge, the height of the log
/// hexagons' own count.
const COUNT_BADGE_FONT_PX: f32 = 11.0;

const COUNT_BADGE_PADDING_PT: f32 = 1.5;

const COUNT_BADGE_CORNER_RADIUS_PT: f32 = 3.0;

/// Fill behind a cluster flag's count, dark enough for white digits over the
/// cloth and over the tiles.
const COUNT_BADGE_FILL: Color32 = Color32::from_black_alpha(200);

#[cfg(test)]
mod tests {
    use chrono::{DateTime, Duration, Utc};
    use gt_types::{FileIdx, GpsTime, LoadedTrack, NavPoint, TimePositionVelocity, TrackIdx};
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

    /// The instances one track's ends draw as, in the order they are placed.
    fn instances_of(
        ends: DrawnTrackEnds<'_>,
        style: FlagStyle,
        transform: &MercTransform,
    ) -> Vec<IconInstance> {
        ends.flags(style, transform)
            .iter()
            .filter_map(|flag| flag.instance(style, transform))
            .collect()
    }

    fn icons_of(instances: &[IconInstance]) -> Vec<IconId> {
        instances.iter().map(|instance| instance.icon).collect()
    }

    #[test]
    fn a_track_gets_a_start_flag_and_a_finish_flag() {
        let (track, transform) = a_track_and_a_view_over_its_first_fix();
        let placed = track.placed_points().unwrap_or_default();
        let ends = DrawnTrackEnds::of(placed, &GlobalFilter::default()).expect("five fixes");

        let instances = instances_of(ends, a_style(false), &transform);

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

        let instances = instances_of(ends.collapsed_to_one_dot(), a_style(false), &transform);

        assert_eq!(icons_of(&instances), vec![IconId::StartFlag]);
    }

    #[test]
    fn a_highlighted_track_draws_larger_blue_flags_on_the_same_fixes() {
        let (track, transform) = a_track_and_a_view_over_its_first_fix();
        let placed = track.placed_points().unwrap_or_default();
        let ends = DrawnTrackEnds::of(placed, &GlobalFilter::default()).expect("five fixes");

        let plain = instances_of(ends, a_style(false), &transform);
        let highlighted = instances_of(ends, a_style(true), &transform);

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

        let instances = instances_of(ends, a_style(false), &a_view_over_the_first_fix());

        assert_eq!(icons_of(&instances), expected);
    }

    #[test]
    fn a_track_whose_ends_meet_draws_the_split_flag_when_it_collapses_to_one_dot() {
        let track = a_track_returning_to_within(0.5);
        let placed = track.placed_points().unwrap_or_default();
        let ends = DrawnTrackEnds::of(placed, &GlobalFilter::default()).expect("three fixes");

        let instances = instances_of(
            ends.collapsed_to_one_dot(),
            a_style(false),
            &a_view_over_the_first_fix(),
        );

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

        let instances = instances_of(
            ends,
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

        let instances = instances_of(
            ends,
            a_style(false),
            &a_view_spacing_consecutive_fixes(10.0),
        );

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

        let instances = instances_of(ends, a_style(false), &transform);

        let feet: Vec<Pos2> = instances.iter().map(pole_foot).collect();
        assert_eq!(feet.len(), 2);
        for (foot, fix) in feet.iter().zip(fixes.iter()) {
            assert!((*foot - *fix).length() < 1e-3, "{foot:?} != {fix:?}");
        }
    }

    fn a_track(index: usize) -> TrackRef {
        TrackRef::new(FileIdx::new(0), TrackIdx::new(index))
    }

    /// A flag of `track` standing `merc_east` east of the grouping cases'
    /// meeting place.
    fn a_flag(track: TrackRef, merc_east: f64, ends: FlagEnds) -> TrackEndFlag {
        TrackEndFlag {
            track,
            flag: StandingFlag {
                merc: MercPoint {
                    x: MEETING_PLACE_MERC.x + merc_east,
                    y: MEETING_PLACE_MERC.y,
                },
                ends,
                pose: FlagPose::Upright,
            },
        }
    }

    fn a_leaning_flag(track: TrackRef, merc_east: f64, pose: FlagPose) -> TrackEndFlag {
        let placed = a_flag(track, merc_east, FlagEnds::Start);
        TrackEndFlag {
            flag: StandingFlag {
                pose,
                ..placed.flag
            },
            ..placed
        }
    }

    fn two() -> NonZeroUsize {
        NonZeroUsize::MIN.saturating_add(1)
    }

    /// Where the grouping cases put the flags that meet.
    const MEETING_PLACE_MERC: MercPoint = MercPoint { x: 0.53, y: 0.33 };

    /// A grouping case keeps every flag passed to it: the bounds span the
    /// whole Mercator square.
    const THE_WHOLE_WORLD: MercBounds = MercBounds {
        x_min: 0.0,
        x_max: 1.0,
        y_min: 0.0,
        y_max: 1.0,
    };

    /// One cloth width in Mercator units. The grouping cases place their flags
    /// within this width of each other, and past it.
    const GROUPING_SPACING_MERC: f64 = 0.001;

    /// A cluster flag's cloth shows what its members are: the start flag where
    /// every one of them is a start, the finish flag where every one is an
    /// end, and the split flag where both are there. A member that is itself a
    /// split flag counts as both.
    #[rstest]
    #[case::two_starts(FlagEnds::Start, FlagEnds::Start, FlagEnds::Start)]
    #[case::two_finishes(FlagEnds::Finish, FlagEnds::Finish, FlagEnds::Finish)]
    #[case::a_start_and_a_finish(FlagEnds::Start, FlagEnds::Finish, FlagEnds::StartAndFinish)]
    #[case::a_split_flag_and_a_start(
        FlagEnds::StartAndFinish,
        FlagEnds::Start,
        FlagEnds::StartAndFinish
    )]
    fn the_flags_of_two_tracks_at_one_place_draw_one_cluster_flag(
        #[case] one: FlagEnds,
        #[case] other: FlagEnds,
        #[case] expected: FlagEnds,
    ) {
        let flags = [
            a_flag(a_track(0), 0.0, one),
            a_flag(a_track(1), GROUPING_SPACING_MERC / 2.0, other),
        ];

        let grouped = group_flags_by_position(&flags, GROUPING_SPACING_MERC, THE_WHOLE_WORLD);

        assert_eq!(
            grouped,
            vec![GroupedFlag::Cluster {
                flag: StandingFlag {
                    merc: MEETING_PLACE_MERC,
                    ends: expected,
                    pose: FlagPose::Upright,
                },
                count: two(),
            }]
        );
    }

    #[test]
    fn one_tracks_own_pair_stays_loose_and_keeps_leaning() {
        let flags = [
            a_leaning_flag(a_track(0), 0.0, FlagPose::LeaningStart),
            a_leaning_flag(
                a_track(0),
                GROUPING_SPACING_MERC / 2.0,
                FlagPose::LeaningFinish,
            ),
        ];

        let grouped = group_flags_by_position(&flags, GROUPING_SPACING_MERC, THE_WHOLE_WORLD);

        assert_eq!(
            grouped,
            vec![
                GroupedFlag::Loose(flags[0].flag),
                GroupedFlag::Loose(flags[1].flag),
            ]
        );
    }

    #[test]
    fn a_flag_further_than_one_cloth_width_away_stays_loose() {
        let flags = [
            a_flag(a_track(0), 0.0, FlagEnds::Start),
            a_flag(a_track(1), GROUPING_SPACING_MERC * 2.0, FlagEnds::Start),
        ];

        let grouped = group_flags_by_position(&flags, GROUPING_SPACING_MERC, THE_WHOLE_WORLD);

        assert_eq!(
            grouped,
            vec![
                GroupedFlag::Loose(flags[0].flag),
                GroupedFlag::Loose(flags[1].flag),
            ]
        );
    }

    #[test]
    fn a_flag_whose_pair_joined_a_cluster_stands_alone_and_upright() {
        let flags = [
            a_flag(a_track(1), 0.0, FlagEnds::Start),
            a_leaning_flag(
                a_track(0),
                GROUPING_SPACING_MERC * 0.75,
                FlagPose::LeaningStart,
            ),
            a_leaning_flag(
                a_track(0),
                GROUPING_SPACING_MERC * 1.5,
                FlagPose::LeaningFinish,
            ),
        ];

        let grouped = group_flags_by_position(&flags, GROUPING_SPACING_MERC, THE_WHOLE_WORLD);

        assert_eq!(
            grouped,
            vec![
                GroupedFlag::Cluster {
                    flag: StandingFlag {
                        merc: MEETING_PLACE_MERC,
                        ends: FlagEnds::Start,
                        pose: FlagPose::Upright,
                    },
                    count: two(),
                },
                GroupedFlag::Loose(StandingFlag {
                    pose: FlagPose::Upright,
                    ..flags[2].flag
                }),
            ]
        );
    }
}
