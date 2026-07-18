//! The map's sky glyphs: a subtle per-point summary of which directions the
//! satellites in the fix came from, so satellite geometry is legible along a
//! track at a glance without hovering every point.
//!
//! Two variants share this module. The minimal **sky ring** is a faint
//! annulus centered on the fix with one bead per fix satellite at its
//! azimuth; because both the map and the ring are north-up, a gap in the
//! beads points at the obstruction beside the track. The detailed **sky
//! disc** is a miniature sky plot, offset from the fix with a short leader,
//! placing a dot per fix satellite by azimuth and elevation. Report-bearing
//! points are decimated through the shared [`crate::collision_grid`] so
//! glyphs stay readable and viewport-stable.

use std::cmp::Reverse;

use egui::{Pos2, Shape, Stroke, Vec2};

use gt_types::satellites::Satellites;
use gt_types::{LoadedTrack, MercBounds, TrackRef};
use gt_ui_types::SkyGlyphVariant;

use crate::collision_grid;
use crate::transform::MercTransform;

/// Minimum on-screen spacing between sky rings. The decimation cell size,
/// so denser reports thin to at most one ring per this many pixels.
const RING_MIN_SPACING_PX: f32 = 72.0;

/// Minimum on-screen spacing between sky discs. Wider than the rings, since
/// an offset disc occupies more room than a centered ring.
const DISC_MIN_SPACING_PX: f32 = 112.0;

/// The decimation spacing for the given variant.
pub(crate) fn min_spacing_px(variant: SkyGlyphVariant) -> f32 {
    match variant {
        SkyGlyphVariant::Ring => RING_MIN_SPACING_PX,
        SkyGlyphVariant::Disc => DISC_MIN_SPACING_PX,
    }
}

/// Zoom at or above which sky glyphs draw. Below it a track collapses to a
/// few pixels and per-point glyphs would be noise, so the overlay stays
/// quiet - matching where per-fix icons become legible.
pub(crate) const MIN_ZOOM: f64 = 13.0;

/// Outer radius of the ring annulus. The hole keeps the fix's heading arrow
/// visible.
const RING_RADIUS_PX: f32 = 15.0;

/// Stroke width of the ring baseline and the disc rim.
const BASELINE_STROKE_PX: f32 = 1.0;

/// Alpha of the ring baseline, kept low so the ring reads as background
/// context rather than competing with the track ink.
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

/// Radius of the sky disc.
const DISC_RADIUS_PX: f32 = 20.0;

/// Offset from the fix to the disc center - up and to the side, like the
/// satellite-label anchor, so the disc and its leader clear the fix position
/// and its heading arrow.
const DISC_OFFSET_PX: Vec2 = Vec2::new(14.0, -36.0);

/// Alpha of the disc's translucent backing fill, for contrast against map
/// tiles without hiding them.
const DISC_BACKING_ALPHA: f32 = 0.78;

/// Alpha of the disc rim and leader, low like the ring baseline.
const DISC_RIM_ALPHA: f32 = 0.6;

/// Radius of a satellite dot inside the disc.
const DISC_DOT_RADIUS_PX: f32 = 2.2;

/// Per-geometry point indices carrying a sky ring this frame, indexed like
/// the caller's geometry list. The same shape the satellite labels use.
pub(crate) type SelectedGlyphs = Vec<Vec<usize>>;

/// The candidate occupying a grid cell. The most informative report (most
/// satellites in the fix) wins, tie-broken by the stable track/point key so
/// the selection cannot flicker between frames.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct Candidate {
    /// Reversed so the highest fix count is the smallest candidate, which
    /// is the one [`collision_grid::winners_per_cell`] keeps.
    fix_rank: Reverse<u32>,
    track: TrackRef,
    point_index: usize,
    geometry_index: usize,
}

/// Resolve which report-bearing points get a sky ring this frame, decimated
/// across all tracks at once. `tracks` yields each glyph-enabled track with
/// its geometry index and ref; `point_passes` applies the caller's per-point
/// conditions (time filter, query hiding). Points outside `viewport` or
/// without a satellite report are skipped.
pub(crate) fn select_glyphs<'a>(
    tracks: impl Iterator<Item = (usize, TrackRef, &'a LoadedTrack)>,
    geometry_count: usize,
    viewport: MercBounds,
    cell_merc: f64,
    mut point_passes: impl FnMut(TrackRef, usize, &gt_types::NavPoint) -> bool,
) -> SelectedGlyphs {
    let mut candidates: Vec<((f64, f64), Candidate)> = Vec::new();
    for (geometry_index, track_ref, track) in tracks {
        for (point_index, point) in track.points.iter().enumerate() {
            let Some(satellites) = &point.satellites else {
                continue;
            };
            let (x, y) = (point.merc.x, point.merc.y);
            if x < viewport.x_min || x > viewport.x_max || y < viewport.y_min || y > viewport.y_max
            {
                continue;
            }
            if !point_passes(track_ref, point_index, point) {
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
    let winners = collision_grid::winners_per_cell(candidates, cell_merc)
        .map(|c| (c.geometry_index, c.point_index));
    collision_grid::group_by_geometry(winners, geometry_count)
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
        satellites,
        ui.visuals().weak_text_color(),
        ui.visuals().dark_mode,
        size_scale,
    );
}

/// Draw the sky glyphs of one track at the given selected point indices, in
/// the chosen variant.
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
    for &pi in point_indices {
        let Some(point) = track.points.get(pi) else {
            continue;
        };
        let Some(satellites) = &point.satellites else {
            continue;
        };
        let center = transform.to_screen(point.merc);
        match variant {
            SkyGlyphVariant::Ring => {
                draw_ring(
                    ui,
                    center,
                    satellites,
                    baseline_color,
                    dark_mode,
                    size_scale,
                );
            }
            SkyGlyphVariant::Disc => {
                draw_disc(
                    ui,
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

/// Draw one sky ring: a faint baseline annulus with one bead per satellite
/// at its azimuth. A report with satellites in the fix draws a solid
/// baseline and filled beads for the fix satellites; a report with none (a
/// fix loss) draws a dashed baseline and hollow beads for the tracked
/// satellites, so "sky seen but unused" reads differently from a normal fix.
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
        let points: Vec<Pos2> = (0..=FIX_LOSS_SEGMENTS)
            .map(|i| {
                let angle = i as f32 / FIX_LOSS_SEGMENTS as f32 * std::f32::consts::TAU;
                center + Vec2::new(angle.sin(), -angle.cos()) * radius
            })
            .collect();
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

/// The screen position of a bead at `azimuth_deg` on a ring of the given
/// radius, north up.
fn bead_pos(center: Pos2, azimuth_deg: f32, radius: f32) -> Pos2 {
    let azimuth = azimuth_deg.to_radians();
    center + Vec2::new(azimuth.sin(), -azimuth.cos()) * radius
}

/// Draw one sky disc: a miniature sky plot offset above the fix with a short
/// leader, with a dot per fix satellite placed by azimuth and elevation
/// (via the same projection as the full sky plot). A fix loss draws a dashed
/// rim and, having no fix satellites, no dots - "sky seen but unused".
fn draw_disc(
    ui: &egui::Ui,
    fix_pos: Pos2,
    satellites: &Satellites,
    rim_color: egui::Color32,
    dark_mode: bool,
    size_scale: f32,
) {
    let painter = ui.painter();
    let radius = DISC_RADIUS_PX * size_scale;
    let dot_radius = DISC_DOT_RADIUS_PX * size_scale;
    let center = fix_pos + DISC_OFFSET_PX * size_scale;
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
        let points: Vec<Pos2> = (0..=FIX_LOSS_SEGMENTS)
            .map(|i| {
                let angle = i as f32 / FIX_LOSS_SEGMENTS as f32 * std::f32::consts::TAU;
                center + Vec2::new(angle.sin(), -angle.cos()) * radius
            })
            .collect();
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
    use gt_test_utils::TestHarness;
    use gt_types::satellites::{Constellation, Satellite, Satellites};
    use gt_types::{FileIdx, TrackIdx, TrackRef};

    use super::{
        DISC_RADIUS_PX, RING_RADIUS_PX, SelectedGlyphs, draw_disc, draw_ring, min_spacing_px,
        select_glyphs,
    };
    use gt_ui_types::SkyGlyphVariant;

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

    /// A track from `(x_m, y_m, report)` specs; a `None` report is a plain
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

    fn select(track: &gt_types::LoadedTrack, cell_merc: f64) -> SelectedGlyphs {
        select_glyphs(
            [(0, TrackRef::new(FileIdx::new(0), TrackIdx::new(0)), track)].into_iter(),
            1,
            WORLD,
            cell_merc,
            |_, _, _| true,
        )
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
        let selected = select_glyphs(
            [(0, TrackRef::new(FileIdx::new(0), TrackIdx::new(0)), &track)].into_iter(),
            1,
            nothing,
            1e-9,
            |_, _, _| true,
        );
        assert_eq!(selected, vec![Vec::<usize>::new()]);
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
        // Beads only on the southern/eastern half; the north reads as blocked.
        let gapped = report(
            [110.0, 150.0, 200.0, 250.0]
                .into_iter()
                .map(|az| sat(Constellation::Galileo, az, true))
                .collect(),
        );
        let mut harness = TestHarness::builder()
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
        let mut harness = TestHarness::builder()
            .size(egui::vec2(300.0, 140.0))
            .theme(true)
            .ui(move |ui| {
                let rim = ui.visuals().weak_text_color();
                let dark = ui.visuals().dark_mode;
                let y = 110.0;
                let gap = DISC_RADIUS_PX * 3.0;
                draw_disc(ui, egui::pos2(gap, y), &spread, rim, dark, 1.0);
                draw_disc(ui, egui::pos2(gap * 2.0, y), &southern, rim, dark, 1.0);
                draw_disc(ui, egui::pos2(gap * 3.0, y), &fix_loss, rim, dark, 1.0);
            });
        harness.run();
        harness.snapshot("sky_discs");
    }

    /// Discs are offset and larger than rings, so they decimate more
    /// sparsely - a swap of the two match arms would flip this ordering.
    #[test]
    fn discs_decimate_more_sparsely_than_rings() {
        assert!(min_spacing_px(SkyGlyphVariant::Disc) > min_spacing_px(SkyGlyphVariant::Ring));
    }
}
