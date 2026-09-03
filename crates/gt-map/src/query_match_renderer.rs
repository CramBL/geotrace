//! Query-match halos: wide strokes beneath the track line marking the
//! stretches a query matched, rings for single-point matches, and the match
//! header shown in the hover tooltip.

use egui::RichText;
use egui::{Color32, Pos2, Stroke, Ui};
use gt_types::{LoadedFile, TrackRef};
use gt_ui_types::StaleRunNote;

use crate::match_reveal::HaloStyle;

/// Paint one draw layer's halos from a track's prepared span geometry, in the
/// layer's `color`.
///
/// `span` is one culling span of the track's polyline. `matched` projects
/// each point's key to whether this layer covers it, so no intermediate
/// buffer is built per frame. Consecutive matched points form the halo
/// stretches. Runs of one point (a single-point match, or a longer match
/// reduced to one visible point by LOD selection) draw as a ring so they stay
/// visible.
pub(crate) fn paint_match_halo_span<K>(
    ui: &Ui,
    span: &[(K, Pos2)],
    matched: impl Fn(&K) -> bool,
    halo: HaloStyle,
    color: Color32,
) {
    let stroke = Stroke::new(halo.band_width, color);
    for run in matched_runs(span, &matched) {
        match run {
            [] => {}
            [(_, pos)] => draw_match_ring(ui, *pos, halo, color),
            _ => {
                let points: Vec<Pos2> = run.iter().map(|&(_, pos)| pos).collect();
                ui.painter().add(egui::Shape::line(points, stroke));
            }
        }
    }
}

/// Ring around an isolated matched point (also used when a whole track
/// collapses to a sub-pixel dot).
pub(crate) fn draw_match_ring(ui: &Ui, pos: Pos2, halo: HaloStyle, color: Color32) {
    ui.painter().circle_stroke(
        pos,
        halo.ring_radius,
        Stroke::new(halo.ring_stroke_width, color),
    );
}

/// Maximal runs of consecutive matched points within one span.
///
/// A halo edge exists only when BOTH endpoints matched, so the halo never
/// overshoots the match by an edge.
fn matched_runs<'a, K>(
    span: &'a [(K, Pos2)],
    matched: &'a impl Fn(&K) -> bool,
) -> impl Iterator<Item = &'a [(K, Pos2)]> {
    span.split(move |(key, _)| !matched(key))
        .filter(|run| !run.is_empty())
}

/// The tooltip header for a hovered match: point count and duration, plus
/// the stale note. The point table below it is the standard hover table.
pub(crate) fn match_header_ui(
    ui: &mut Ui,
    files: &[LoadedFile],
    track_ref: TrackRef,
    range: &std::ops::Range<usize>,
    stale: bool,
) {
    let Some(track) = track_ref.resolve(files) else {
        return;
    };
    let count = range.len();
    let duration = gt_fmt::match_duration_seconds(track, range);
    let em_dash = gt_ui_theme::EM_DASH;
    let heading = match duration {
        Some(secs) => format!(
            "Match {em_dash} {count} points over {}",
            gt_fmt::format_match_duration(secs)
        ),
        None => format!("Match {em_dash} {count} points"),
    };
    ui.strong(heading);
    if stale {
        ui.label(
            RichText::new(StaleRunNote::RunAgain.to_string())
                .weak()
                .italics(),
        );
    }
    ui.separator();
}

#[cfg(test)]
mod tests {
    use egui::pos2;

    use super::*;

    fn span(flags: &[bool]) -> Vec<(bool, Pos2)> {
        flags
            .iter()
            .enumerate()
            .map(|(i, &matched)| (matched, pos2(i as f32, 0.0)))
            .collect()
    }

    fn is_matched(key: &bool) -> bool {
        *key
    }

    #[test]
    fn matched_runs_split_on_unmatched_points() {
        let s = span(&[false, true, true, false, true, false, true, true, true]);
        let runs: Vec<usize> = matched_runs(&s, &is_matched).map(<[_]>::len).collect();
        assert_eq!(runs, vec![2, 1, 3]);
    }

    #[test]
    fn matched_runs_of_all_matched_is_one_run() {
        let s = span(&[true, true, true]);
        let runs: Vec<usize> = matched_runs(&s, &is_matched).map(<[_]>::len).collect();
        assert_eq!(runs, vec![3]);
    }

    #[test]
    fn matched_runs_of_none_matched_is_empty() {
        let s = span(&[false, false]);
        assert_eq!(matched_runs(&s, &is_matched).count(), 0);
    }

    /// The header states how long the match ran, over the shared duration
    /// helper. The fixture's points are spaced exactly one second apart.
    #[test]
    fn match_duration_covers_first_to_last_point() {
        let track = gt_test_utils::loaded_track_with_points(gt_test_utils::nav_test_data());
        assert_eq!(gt_fmt::match_duration_seconds(&track, &(0..1)), None);
        assert_eq!(gt_fmt::match_duration_seconds(&track, &(0..3)), Some(2));
        assert_eq!(
            gt_fmt::match_duration_seconds(&track, &(150..300)),
            Some(149)
        );
        assert_eq!(gt_fmt::match_duration_seconds(&track, &(0..10_000)), None);
        assert_eq!(gt_fmt::match_duration_seconds(&track, &(5..5)), None);
    }
}
