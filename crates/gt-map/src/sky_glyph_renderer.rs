//! The map's sky glyphs: a subtle per-point summary of which directions the
//! satellites in the fix came from, so satellite geometry is legible along a
//! track at a glance without hovering every point.
//!
//! Two variants share this module. The minimal **sky ring** is a faint
//! annulus centered on the fix with one bead per fix satellite at its
//! azimuth. Both the map and the ring are north-up, so a gap in the beads
//! points at the obstruction beside the track. The detailed **sky
//! disc** is a miniature sky plot, offset from the fix with a short leader,
//! placing a dot per fix satellite by azimuth and elevation. Report-bearing
//! points are decimated through the shared [`crate::collision_grid`] so
//! glyphs stay readable and viewport-stable.

use std::cmp::Reverse;

use egui::{Pos2, Shape, Stroke, Vec2};

use gt_types::satellites::Satellites;
use gt_types::{LoadedTrack, MercBounds, PlacedPoints, TrackRef};
use gt_ui_types::{SkyGlyphVariant, TrackMatchView};
use smallvec::SmallVec;

use crate::collision_grid;
use crate::transform::MercTransform;

/// Minimum on-screen spacing between sky rings. The decimation cell size,
/// so denser reports thin to at most one ring per this many pixels.
const RING_MIN_SPACING_PX: f32 = 72.0;

/// Minimum on-screen spacing between sky discs. Wider than the rings, since
/// an offset disc occupies more room than a centered ring.
const DISC_MIN_SPACING_PX: f32 = 112.0;

pub(crate) fn min_spacing_px(variant: SkyGlyphVariant) -> f32 {
    match variant {
        SkyGlyphVariant::Ring => RING_MIN_SPACING_PX,
        SkyGlyphVariant::Disc => DISC_MIN_SPACING_PX,
    }
}

/// Zoom at or above which sky glyphs draw, matching where per-fix icons become
/// legible. Below it a track collapses to a few pixels.
pub(crate) const MIN_ZOOM: f64 = 13.0;

/// Outer radius of the ring annulus. The hole keeps the fix's heading arrow
/// visible.
const RING_RADIUS_PX: f32 = 15.0;

/// Stroke width of the ring baseline and the disc rim.
const BASELINE_STROKE_PX: f32 = 1.0;

/// Alpha of the ring baseline, kept low so the ring reads as background
/// context.
const BASELINE_ALPHA: f32 = 0.35;

/// Radius of a satellite bead on the ring.
const BEAD_RADIUS_PX: f32 = 3.0;

/// Stroke width of a hollow (fix-loss) bead.
const HOLLOW_BEAD_STROKE_PX: f32 = 1.4;

/// Dash and gap lengths of the fix-loss baseline ring / disc rim.
const FIX_LOSS_DASH_PX: f32 = 3.0;
const FIX_LOSS_GAP_PX: f32 = 3.0;
/// Polyline segments approximating a dashed fix-loss circle.
const FIX_LOSS_SEGMENTS: u32 = 48;
/// Vertices in that polyline (segments plus the closing point). The circle is
/// a per-glyph, per-frame temporary, so it stacks in a [`SmallVec`] of this
/// capacity.
const FIX_LOSS_RING_POINTS: usize = FIX_LOSS_SEGMENTS as usize + 1;

/// Radius of the sky disc.
const DISC_RADIUS_PX: f32 = 20.0;

/// Fallback offset from the fix to the disc center where the track is
/// straight or the anchor has no usable neighbor - up and to the side, like
/// the satellite-label anchor, so the disc and its leader clear the fix and
/// its heading arrow. On a curve the disc is placed perpendicular to the
/// track instead (see [`disc_offset`]). This vector's length sets that
/// perpendicular distance.
const DISC_OFFSET_PX: Vec2 = Vec2::new(14.0, -36.0);

/// A neighbor sample must lie at least this far from the anchor on screen
/// before it defines the local track tangent, so nearly-coincident points
/// don't yield a noisy direction.
const TANGENT_SAMPLE_MIN_PX: f32 = 8.0;

/// A bend sharper than this (perpendicular offset of the neighbor midpoint
/// from the fix, in screen px) places the disc on the bend's outer side.
/// Below it the track is treated as straight and the disc goes up.
const CURVE_MIN_PX: f32 = 1.5;

/// Alpha of the disc's translucent backing fill, for contrast against map
/// tiles without hiding them.
const DISC_BACKING_ALPHA: f32 = 0.78;

/// Alpha of the disc rim and leader, low like the ring baseline.
const DISC_RIM_ALPHA: f32 = 0.6;

/// Radius of a satellite dot inside the disc.
const DISC_DOT_RADIUS_PX: f32 = 2.2;

/// Reusable glyph-decimation scratch, held across frames by the map widget.
/// Its per-geometry output lists are indexed like the caller's geometry list,
/// the same shape the satellite labels use.
pub(crate) type GlyphSelection = collision_grid::DecimationScratch<Candidate>;

/// The candidate occupying a grid cell. The most informative report (most
/// satellites in the fix) wins, tie-broken by the stable track/point key so
/// the selection cannot flicker between frames.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct Candidate {
    /// Reversed so the highest fix count is the smallest candidate, which
    /// is the one [`collision_grid::DecimationScratch::resolve`] keeps.
    fix_rank: Reverse<u32>,
    track: TrackRef,
    point_index: usize,
    geometry_index: usize,
}

/// Resolve which report-bearing points get a sky ring this frame, decimated
/// across all tracks at once. `tracks` yields each glyph-enabled track with
/// its geometry index, its ref, and its query ranges. `point_passes` applies
/// the caller's per-point conditions (time filter, query hiding). Points
/// outside `viewport` or without a satellite report are skipped.
pub(crate) fn select_glyphs<'s, 'a>(
    scratch: &'s mut GlyphSelection,
    tracks: impl Iterator<Item = (usize, TrackRef, &'a LoadedTrack, TrackMatchView<'a>)>,
    geometry_count: usize,
    viewport: MercBounds,
    cell_merc: f64,
    mut point_passes: impl FnMut(&TrackMatchView<'a>, usize, &gt_types::NavPoint) -> bool,
) -> &'s [Vec<usize>] {
    let candidates = scratch.candidates();
    for (geometry_index, track_ref, track, query_view) in tracks {
        // A track with no geometry is drawn nowhere, so it carries no glyph.
        let Some(placed) = track.placed_points() else {
            continue;
        };
        for (point_index, point) in placed.iter().enumerate() {
            let Some(satellites) = &point.fix.satellites else {
                continue;
            };
            let (x, y) = (point.merc().x, point.merc().y);
            if x < viewport.x_min || x > viewport.x_max || y < viewport.y_min || y > viewport.y_max
            {
                continue;
            }
            if !point_passes(&query_view, point_index, point.fix) {
                continue;
            }
            candidates.push((
                (x, y),
                Candidate {
                    fix_rank: Reverse(satellites.fix_count()),
                    track: track_ref,
                    point_index,
                    geometry_index,
                },
            ));
        }
    }
    scratch.resolve(cell_merc, geometry_count, |c| {
        (c.geometry_index, c.point_index)
    })
}

/// Draw a single sky disc at `fix_pos` for one point's report, for the
/// plot-hover cross-highlight: scrubbing the time-series plot walks the disc
/// along the track. Always the detailed disc, regardless of the overlay's
/// own variant or whether the overlay is shown at all - it is a focus
/// indicator for the one point the plot cursor is on.
pub(crate) fn draw_hover_disc(
    ui: &egui::Ui,
    fix_pos: Pos2,
    satellites: &Satellites,
    size_scale: f32,
) {
    draw_disc(
        ui,
        fix_pos,
        fix_pos + disc_offset_for_samples(None, None, fix_pos, size_scale),
        satellites,
        ui.visuals().weak_text_color(),
        ui.visuals().dark_mode,
        size_scale,
    );
}

/// Draw the sky glyphs of one track at `point_indices`, in `variant`.
///
/// `size_scale` shrinks the glyph in step with the heading arrows at lower
/// zoom (1.0 where the fix icons are full size), so glyphs never stay a
/// fixed pixel size while the track shrinks under them.
pub(crate) fn draw_glyphs(
    ui: &egui::Ui,
    track: &LoadedTrack,
    point_indices: &[usize],
    transform: &MercTransform,
    variant: SkyGlyphVariant,
    size_scale: f32,
) {
    let dark_mode = ui.visuals().dark_mode;
    let baseline_color = ui.visuals().weak_text_color();
    let Some(placed) = track.placed_points() else {
        return;
    };
    for &pi in point_indices {
        let Some(point) = placed.get(pi) else {
            continue;
        };
        let Some(satellites) = &point.fix.satellites else {
            continue;
        };
        let fix_pos = transform.to_screen(point.merc());
        match variant {
            SkyGlyphVariant::Ring => {
                draw_ring(
                    ui,
                    fix_pos,
                    satellites,
                    baseline_color,
                    dark_mode,
                    size_scale,
                );
            }
            SkyGlyphVariant::Disc => {
                let center = fix_pos + disc_offset(placed, pi, transform, fix_pos, size_scale);
                draw_disc(
                    ui,
                    fix_pos,
                    center,
                    satellites,
                    baseline_color,
                    dark_mode,
                    size_scale,
                );
            }
        }
    }
}

/// Draw one sky ring: a faint baseline annulus with one bead per satellite at
/// its azimuth. A report with satellites in the fix draws a solid baseline and
/// filled beads for them. A report with none (a fix loss) draws a dashed
/// baseline and hollow beads for the tracked satellites.
fn draw_ring(
    ui: &egui::Ui,
    center: Pos2,
    satellites: &Satellites,
    baseline_color: egui::Color32,
    dark_mode: bool,
    size_scale: f32,
) {
    let painter = ui.painter();
    let baseline = baseline_color.gamma_multiply(BASELINE_ALPHA);
    let radius = RING_RADIUS_PX * size_scale;
    let bead_radius = BEAD_RADIUS_PX * size_scale;
    let fix_loss = satellites.fix_count() == 0;

    if fix_loss {
        let points = fix_loss_circle_points(center, radius);
        painter.add(Shape::dashed_line(
            &points,
            Stroke::new(BASELINE_STROKE_PX, baseline),
            FIX_LOSS_DASH_PX * size_scale,
            FIX_LOSS_GAP_PX * size_scale,
        ));
        for satellite in satellites.satellites() {
            if let Some(azimuth) = satellite.azimuth() {
                let color = gt_ui_theme::constellation_color(satellite.constellation(), dark_mode);
                painter.circle_stroke(
                    bead_pos(center, azimuth, radius),
                    bead_radius - 0.5,
                    Stroke::new(HOLLOW_BEAD_STROKE_PX, color),
                );
            }
        }
    } else {
        painter.circle_stroke(center, radius, Stroke::new(BASELINE_STROKE_PX, baseline));
        for satellite in satellites.satellites_with_fix() {
            if let Some(azimuth) = satellite.azimuth() {
                let color = gt_ui_theme::constellation_color(satellite.constellation(), dark_mode);
                painter.circle_filled(bead_pos(center, azimuth, radius), bead_radius, color);
            }
        }
    }
}

/// The vertices of a dashed fix-loss circle of `radius` around `center`,
/// north up. Stacks in a [`SmallVec`] since it is a per-glyph, per-frame
/// temporary handed straight to [`Shape::dashed_line`] as a slice.
fn fix_loss_circle_points(center: Pos2, radius: f32) -> SmallVec<[Pos2; FIX_LOSS_RING_POINTS]> {
    (0..=FIX_LOSS_SEGMENTS)
        .map(|i| {
            let angle = i as f32 / FIX_LOSS_SEGMENTS as f32 * std::f32::consts::TAU;
            center + Vec2::new(angle.sin(), -angle.cos()) * radius
        })
        .collect()
}

/// The screen position of a bead at `azimuth_deg` on a ring of `radius`,
/// north up.
fn bead_pos(center: Pos2, azimuth_deg: f32, radius: f32) -> Pos2 {
    let azimuth = azimuth_deg.to_radians();
    center + Vec2::new(azimuth.sin(), -azimuth.cos()) * radius
}

/// Screen position of the first track point, scanning `indices` outward from
/// the anchor, that lies at least [`TANGENT_SAMPLE_MIN_PX`] from `fix_pos` -
/// the sample that defines the local tangent on that side. `None` when the
/// track runs out first (a short or heavily-culled track near its end).
fn tangent_sample(
    points: PlacedPoints<'_>,
    transform: &MercTransform,
    fix_pos: Pos2,
    indices: impl Iterator<Item = usize>,
) -> Option<Pos2> {
    for idx in indices {
        let point = points.get(idx)?;
        let pos = transform.to_screen(point.merc());
        if (pos - fix_pos).length() >= TANGENT_SAMPLE_MIN_PX {
            return Some(pos);
        }
    }
    None
}

/// The offset from a fix to its sky-disc center.
///
/// Placed perpendicular to the local track tangent, on the outer side of a
/// bend, so the disc and its leader clear the trackline. Where the track is
/// straight the perpendicular is ambiguous, so the disc goes up (or, for a
/// vertical track, up-and-right). Where the anchor has no usable neighbor on
/// either side it falls back to [`DISC_OFFSET_PX`].
fn disc_offset(
    points: PlacedPoints<'_>,
    pi: usize,
    transform: &MercTransform,
    fix_pos: Pos2,
    size_scale: f32,
) -> Vec2 {
    let prev = tangent_sample(points, transform, fix_pos, (0..pi).rev());
    let next = tangent_sample(points, transform, fix_pos, pi + 1..points.len());
    disc_offset_for_samples(prev, next, fix_pos, size_scale)
}

/// The disc offset given the neighbor samples already resolved on each side.
/// Split from [`disc_offset`] so the placement geometry is testable without a
/// [`MercTransform`].
fn disc_offset_for_samples(
    prev: Option<Pos2>,
    next: Option<Pos2>,
    fix_pos: Pos2,
    size_scale: f32,
) -> Vec2 {
    let tangent = match (prev, next) {
        (Some(p), Some(n)) => n - p,
        (Some(p), None) => fix_pos - p,
        (None, Some(n)) => n - fix_pos,
        (None, None) => return DISC_OFFSET_PX * size_scale,
    };
    // A hairpin can leave the two samples nearly coincident, so guard the
    // normalization.
    if tangent.length() < f32::EPSILON {
        return DISC_OFFSET_PX * size_scale;
    }
    outward_normal(tangent, fix_pos, prev, next) * (DISC_OFFSET_PX.length() * size_scale)
}

/// A unit normal to `tangent`, on the outer side of the bend defined by the
/// neighbor samples. On a straight run (no measurable bend) it returns the
/// upward normal, breaking a vertical tie toward the right so the disc still
/// reads as sitting above and beside the track.
fn outward_normal(tangent: Vec2, fix_pos: Pos2, prev: Option<Pos2>, next: Option<Pos2>) -> Vec2 {
    let candidate = -tangent.normalized().rot90();
    // Vector from the fix to the midpoint of the neighbor samples. Its
    // component along `candidate` (perpendicular to the tangent) is the bend:
    // the tangential part is projected out, so uneven per-side spacing on a
    // straight run does not register as curvature.
    let to_midpoint = match (prev, next) {
        (Some(p), Some(n)) => (p.to_vec2() + n.to_vec2()) * 0.5 - fix_pos.to_vec2(),
        _ => Vec2::ZERO,
    };
    let bend = candidate.dot(to_midpoint);
    if bend.abs() >= CURVE_MIN_PX {
        // Point away from the concave side (the midpoint lies inside the bend).
        if bend <= 0.0 { candidate } else { -candidate }
    } else {
        // Straight: prefer the upward normal (more negative y). For a vertical
        // track, whose normals are horizontal, break the tie toward the right.
        let up = if candidate.y <= -candidate.y {
            candidate
        } else {
            -candidate
        };
        // `up.y.abs() > EPSILON` is just "the normal isn't horizontal". Only
        // then does the y test above settle it, otherwise fall to the x tie.
        if up.y.abs() > f32::EPSILON || up.x >= 0.0 {
            up
        } else {
            -up
        }
    }
}

/// Draw one sky disc: a miniature sky plot offset from the fix with a short
/// leader, with a dot per fix satellite placed by azimuth and elevation
/// (via the same projection as the full sky plot). A fix loss draws a dashed
/// rim and, having no fix satellites, no dots - "sky seen but unused".
fn draw_disc(
    ui: &egui::Ui,
    fix_pos: Pos2,
    center: Pos2,
    satellites: &Satellites,
    rim_color: egui::Color32,
    dark_mode: bool,
    size_scale: f32,
) {
    let painter = ui.painter();
    let radius = DISC_RADIUS_PX * size_scale;
    let dot_radius = DISC_DOT_RADIUS_PX * size_scale;
    let rim = rim_color.gamma_multiply(DISC_RIM_ALPHA);

    // Leader from the fix to the near edge of the disc rim.
    let to_fix = (fix_pos - center).normalized();
    painter.line_segment(
        [fix_pos, center + to_fix * radius],
        Stroke::new(BASELINE_STROKE_PX, rim),
    );
    painter.circle_filled(
        center,
        radius,
        ui.visuals().panel_fill.gamma_multiply(DISC_BACKING_ALPHA),
    );

    if satellites.fix_count() == 0 {
        let points = fix_loss_circle_points(center, radius);
        painter.add(Shape::dashed_line(
            &points,
            Stroke::new(BASELINE_STROKE_PX, rim),
            FIX_LOSS_DASH_PX * size_scale,
            FIX_LOSS_GAP_PX * size_scale,
        ));
        return;
    }

    painter.circle_stroke(center, radius, Stroke::new(BASELINE_STROKE_PX, rim));
    for satellite in satellites.satellites_with_fix() {
        let (Some(azimuth), Some(elevation)) = (satellite.azimuth(), satellite.elevation()) else {
            continue;
        };
        let color = gt_ui_theme::constellation_color(satellite.constellation(), dark_mode);
        let offset = gt_sky::unit_disc_position(azimuth, elevation) * radius;
        painter.circle_filled(center + offset, dot_radius, color);
    }
}

#[cfg(test)]
mod tests {
    use egui::{pos2, vec2};
    use gt_types::satellites::{Constellation, Satellite, Satellites};
    use gt_types::{FileIdx, TrackIdx, TrackRef};
    use gt_ui_types::{SkyGlyphVariant, TrackMatchView};

    use super::{
        DISC_OFFSET_PX, DISC_RADIUS_PX, GlyphSelection, RING_RADIUS_PX, disc_offset_for_samples,
        draw_disc, draw_ring, min_spacing_px, outward_normal, select_glyphs,
    };

    const WORLD: gt_types::MercBounds = gt_types::MercBounds {
        x_min: 0.0,
        x_max: 1.0,
        y_min: 0.0,
        y_max: 1.0,
    };

    fn sat(constellation: Constellation, azimuth: f32, in_fix: bool) -> Satellite {
        Satellite::new(
            constellation,
            1,
            Some(45.0),
            Some(azimuth),
            Some(40.0),
            in_fix,
        )
    }

    fn report(sats: Vec<Satellite>) -> Satellites {
        Satellites::new(None, None, sats)
    }

    /// A track from `(x_m, y_m, report)` specs. A `None` report is a plain
    /// point that anchors no glyph.
    fn track(points: &[(f64, f64, Option<Satellites>)]) -> gt_types::LoadedTrack {
        let points = points
            .iter()
            .map(|(x_m, y_m, report)| {
                gt_test_utils::nav_point_at_meters(*x_m, *y_m, report.clone())
            })
            .collect();
        gt_test_utils::loaded_track_with_points(points)
    }

    fn select(track: &gt_types::LoadedTrack, cell_merc: f64) -> Vec<Vec<usize>> {
        let mut scratch = GlyphSelection::default();
        select_glyphs(
            &mut scratch,
            [(
                0,
                TrackRef::new(FileIdx::new(0), TrackIdx::new(0)),
                track,
                TrackMatchView::default(),
            )]
            .into_iter(),
            1,
            WORLD,
            cell_merc,
            |_, _, _| true,
        )
        .to_vec()
    }

    #[test]
    fn only_report_bearing_points_anchor_a_glyph() {
        let track = track(&[
            (0.0, 0.0, None),
            (
                5_000.0,
                0.0,
                Some(report(vec![sat(Constellation::Gps, 45.0, true)])),
            ),
            (10_000.0, 0.0, None),
        ]);
        // A cell far smaller than the spacing, so decimation keeps every
        // eligible point - only index 1 carries a report.
        assert_eq!(select(&track, 1e-9), vec![vec![1]]);
    }

    #[test]
    fn dense_reports_thin_to_one_per_cell() {
        // Three report points ~1 m apart, in a ~55 m cell: one survives.
        let track = track(&[
            (
                0.0,
                0.0,
                Some(report(vec![sat(Constellation::Gps, 45.0, true)])),
            ),
            (
                1.0,
                0.0,
                Some(report(vec![sat(Constellation::Gps, 90.0, true)])),
            ),
            (
                2.0,
                0.0,
                Some(report(vec![sat(Constellation::Gps, 180.0, true)])),
            ),
        ]);
        let selected = select(&track, 55.0 / 40_000_000.0);
        assert_eq!(selected.len(), 1);
        assert_eq!(selected[0].len(), 1);
    }

    #[test]
    fn the_most_informative_report_wins_a_cell() {
        // Two reports in one cell: the one with more satellites in the fix
        // wins, regardless of point order.
        let track = track(&[
            (
                0.0,
                0.0,
                Some(report(vec![sat(Constellation::Gps, 45.0, true)])),
            ),
            (
                1.0,
                0.0,
                Some(report(vec![
                    sat(Constellation::Gps, 45.0, true),
                    sat(Constellation::Galileo, 200.0, true),
                    sat(Constellation::Beidou, 300.0, true),
                ])),
            ),
        ]);
        assert_eq!(select(&track, 1.0), vec![vec![1]]);
    }

    #[test]
    fn distant_reports_each_keep_a_glyph() {
        let track = track(&[
            (
                0.0,
                0.0,
                Some(report(vec![sat(Constellation::Gps, 45.0, true)])),
            ),
            (
                1_100_000.0,
                0.0,
                Some(report(vec![sat(Constellation::Gps, 90.0, true)])),
            ),
        ]);
        assert_eq!(select(&track, 55.0 / 40_000_000.0), vec![vec![0, 1]]);
    }

    #[test]
    fn out_of_viewport_reports_are_skipped() {
        let track = track(&[(
            5_000.0,
            5_000.0,
            Some(report(vec![sat(Constellation::Gps, 45.0, true)])),
        )]);
        let nothing = gt_types::MercBounds {
            x_min: 0.9,
            x_max: 1.0,
            y_min: 0.9,
            y_max: 1.0,
        };
        let mut scratch = GlyphSelection::default();
        let selected = select_glyphs(
            &mut scratch,
            [(
                0,
                TrackRef::new(FileIdx::new(0), TrackIdx::new(0)),
                &track,
                TrackMatchView::default(),
            )]
            .into_iter(),
            1,
            nothing,
            1e-9,
            |_, _, _| true,
        );
        assert_eq!(selected.to_vec(), vec![Vec::<usize>::new()]);
    }

    /// Snapshot: the three ring states side by side - open sky (a full bead
    /// circle), a gap toward the north (blocked sky), and a fix loss (dashed
    /// baseline, hollow beads).
    #[test]
    fn sky_rings() {
        let spread = |in_fix: bool| {
            report(
                [20.0, 70.0, 130.0, 200.0, 250.0, 310.0]
                    .into_iter()
                    .map(|az| sat(Constellation::Gps, az, in_fix))
                    .collect(),
            )
        };
        // Beads only on the southern/eastern half: the north reads as blocked.
        let gapped = report(
            [110.0, 150.0, 200.0, 250.0]
                .into_iter()
                .map(|az| sat(Constellation::Galileo, az, true))
                .collect(),
        );
        let mut harness = crate::test_harness::builder()
            .size(egui::vec2(360.0, 120.0))
            .theme(true)
            .ui(move |ui| {
                let baseline = ui.visuals().weak_text_color();
                let dark = ui.visuals().dark_mode;
                let y = 60.0;
                let gap = RING_RADIUS_PX * 4.0;
                draw_ring(ui, egui::pos2(gap, y), &spread(true), baseline, dark, 1.0);
                draw_ring(ui, egui::pos2(gap * 2.0, y), &gapped, baseline, dark, 1.0);
                draw_ring(
                    ui,
                    egui::pos2(gap * 3.0, y),
                    &spread(false),
                    baseline,
                    dark,
                    1.0,
                );
            });
        harness.run();
        harness.snapshot("sky_rings");
    }

    /// Snapshot: the disc variant - a full-sky report offset with its leader,
    /// a report leaning south (northern sky blocked), and a fix loss (dashed
    /// rim, no dots).
    #[test]
    fn sky_discs() {
        let sat_el = |constellation, az: f32, el: f32, in_fix| {
            Satellite::new(constellation, 1, Some(el), Some(az), Some(40.0), in_fix)
        };
        let spread = report(
            [(45.0, 62.0), (110.0, 35.0), (200.0, 71.0), (300.0, 20.0)]
                .into_iter()
                .map(|(az, el)| sat_el(Constellation::Gps, az, el, true))
                .collect(),
        );
        let southern = report(
            [(150.0, 25.0), (200.0, 40.0), (250.0, 18.0)]
                .into_iter()
                .map(|(az, el)| sat_el(Constellation::Galileo, az, el, true))
                .collect(),
        );
        let fix_loss = report(
            [(45.0, 30.0), (200.0, 20.0)]
                .into_iter()
                .map(|(az, el)| sat_el(Constellation::Gps, az, el, false))
                .collect(),
        );
        let mut harness = crate::test_harness::builder()
            .size(egui::vec2(300.0, 140.0))
            .theme(true)
            .ui(move |ui| {
                let rim = ui.visuals().weak_text_color();
                let dark = ui.visuals().dark_mode;
                let y = 110.0;
                let gap = DISC_RADIUS_PX * 3.0;
                let disc = |ui: &egui::Ui, fix: egui::Pos2, sats: &Satellites| {
                    draw_disc(ui, fix, fix + DISC_OFFSET_PX, sats, rim, dark, 1.0);
                };
                disc(ui, egui::pos2(gap, y), &spread);
                disc(ui, egui::pos2(gap * 2.0, y), &southern);
                disc(ui, egui::pos2(gap * 3.0, y), &fix_loss);
            });
        harness.run();
        harness.snapshot("sky_discs");
    }

    /// On a straight run the disc sits above the track: a horizontal track
    /// pushes the disc up, a vertical track up-and-right (there is no "up"
    /// perpendicular, so the tie breaks rightward).
    #[test]
    fn straight_track_places_the_disc_above() {
        let fix = pos2(50.0, 50.0);
        let up = outward_normal(vec2(1.0, 0.0), fix, None, None);
        assert!(
            up.y < 0.0,
            "horizontal track should push the disc up: {up:?}"
        );

        let side = outward_normal(vec2(0.0, 1.0), fix, None, None);
        assert!(
            side.x > 0.0 && side.y.abs() < f32::EPSILON,
            "vertical track should push the disc to the right: {side:?}"
        );
    }

    /// On a bend the disc goes to the outer (convex) side, so it and its
    /// leader clear the trackline. A peak (neighbors below the fix) pushes the
    /// disc up. A valley (neighbors above) pushes it down.
    #[test]
    fn curved_track_places_the_disc_on_the_outer_side() {
        let fix = pos2(50.0, 50.0);
        // Peak: both neighbors sit below the fix (larger y), so the outer side
        // is up.
        let peak = outward_normal(
            vec2(1.0, 0.0),
            fix,
            Some(pos2(30.0, 60.0)),
            Some(pos2(70.0, 60.0)),
        );
        assert!(peak.y < 0.0, "peak should push the disc up: {peak:?}");

        // Valley: both neighbors sit above the fix, so the outer side is down.
        let valley = outward_normal(
            vec2(1.0, 0.0),
            fix,
            Some(pos2(30.0, 40.0)),
            Some(pos2(70.0, 40.0)),
        );
        assert!(
            valley.y > 0.0,
            "valley should push the disc down: {valley:?}"
        );

        // The result is always a unit vector.
        for n in [peak, valley] {
            assert!((n.length() - 1.0).abs() < 1e-4, "normal not unit: {n:?}");
        }
    }

    /// A straight run with unevenly-spaced neighbors (the common case for
    /// irregularly-timestamped tracks) must still read as straight: the bend
    /// test projects out the along-track component, so lopsided spacing does
    /// not masquerade as curvature and flip the disc across the line.
    #[test]
    fn asymmetric_spacing_on_a_straight_run_still_reads_as_straight() {
        let fix = pos2(50.0, 50.0);
        // Collinear, but the next sample is far more distant than the prev one.
        let normal = outward_normal(
            vec2(1.0, 0.0),
            fix,
            Some(pos2(40.0, 50.0)),
            Some(pos2(95.0, 50.0)),
        );
        assert!(
            normal.y < 0.0,
            "asymmetric-but-straight should still push the disc up: {normal:?}"
        );
    }

    /// At a track end only one neighbor exists, so there is no bend to measure:
    /// the disc uses the straight fallback and stays offset by the standard
    /// distance.
    #[test]
    fn track_end_uses_the_straight_fallback() {
        let fix = pos2(50.0, 50.0);
        // Only a prev sample (anchor is the last point), horizontal track.
        let offset = disc_offset_for_samples(Some(pos2(30.0, 50.0)), None, fix, 1.0);
        assert!(
            offset.y < 0.0,
            "one-sided anchor should place up: {offset:?}"
        );
        assert!(
            (offset.length() - DISC_OFFSET_PX.length()).abs() < 1e-3,
            "offset should keep the standard distance: {offset:?}"
        );

        // No neighbors at all falls back to the fixed offset verbatim.
        let none = disc_offset_for_samples(None, None, fix, 1.0);
        assert!(
            (none - DISC_OFFSET_PX).length() < 1e-4,
            "expected fixed fallback: {none:?}"
        );
    }

    /// Discs are offset and larger than rings, so they decimate more
    /// sparsely - a swap of the two match arms would flip this ordering.
    #[test]
    fn discs_decimate_more_sparsely_than_rings() {
        assert!(min_spacing_px(SkyGlyphVariant::Disc) > min_spacing_px(SkyGlyphVariant::Ring));
    }
}
