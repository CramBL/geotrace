//! The reveal animation for a new query run's match halos: every halo starts
//! inflated and brighter and settles back to its normal size and alpha.
//!
//! Everything the animation is made of lives here - when it fires, how long it
//! runs, its easing, and the halo geometry it produces - so the renderers paint
//! whatever [`HaloStyle`] hands them and know nothing about time.

use egui::Color32;
use gt_ui_types::QueryMatches;

use crate::tpv_renderer::{self, TpvDrawStyle};

/// How long a reveal takes to settle.
const REVEAL_DURATION_SEC: f32 = 0.8;

/// Width of the settled halo band. Deliberately wider than the trackline, the
/// quality line (5.0), and typical accuracy-circle bands, so the halo reads as
/// a band around the track rather than a second line under it.
const SETTLED_BAND_WIDTH: f32 = 22.0;

/// Stroke width of the settled ring drawn around a single-point match.
const SETTLED_RING_STROKE_WIDTH: f32 = 3.0;

/// Padding between the icon size and the single-point ring radius, so the ring
/// encloses the fix icon like the plot-hover ring does.
const RING_RADIUS_PADDING: f32 = 5.0;

/// Screen pixels the reveal adds to each dimension at its start, before the
/// zoom scale. Zoomed out the whole budget shrinks with the fix icons, so
/// dense tracks inflate modestly and the alpha boost carries the signal.
const REVEAL_BAND_WIDTH_BUDGET_PX: f32 = 24.0;
const REVEAL_RING_RADIUS_BUDGET_PX: f32 = 18.0;
const REVEAL_RING_STROKE_BUDGET_PX: f32 = 3.0;

/// Reveal clock for the query-match halos.
///
/// Timestamps are egui clock seconds (`InputState::time`), so the animation is
/// deterministic under `egui_kittest`.
#[derive(Debug, Default)]
pub(crate) struct MatchRevealState {
    start: Option<f64>,
    /// The run whose matches the map last saw, revealed or not, so a run that
    /// did not qualify cannot reveal later.
    last_run_seen: u64,
}

impl MatchRevealState {
    /// Start the reveal when `matches` come from a run the map has not seen
    /// yet and that has halos to show.
    ///
    /// A stale run, a run without halos, and matches no run produced (`run` 0)
    /// are recorded without revealing.
    pub(crate) fn start_reveal_for_new_run(&mut self, matches: Option<&QueryMatches>, now: f64) {
        let Some(matches) = matches else {
            return;
        };
        if matches.run == self.last_run_seen {
            return;
        }
        self.last_run_seen = matches.run;
        if matches.run != 0 && !matches.stale && matches.has_halos() {
            self.restart(now);
        }
    }

    /// Play the reveal from the start, whatever run the matches come from.
    pub(crate) fn restart(&mut self, now: f64) {
        self.start = Some(now);
    }

    /// The reveal amount for the current frame: 1.0 at the start of a reveal,
    /// 0.0 once settled. Clears the clock on the frame the animation ends, so
    /// [`is_active`](Self::is_active) reports `false` from then on.
    pub(crate) fn tick(&mut self, now: f64) -> f32 {
        let Some(start) = self.start else {
            return 0.0;
        };
        let elapsed = (now - start) as f32;
        if elapsed >= REVEAL_DURATION_SEC {
            self.start = None;
            return 0.0;
        }
        1.0 - ease_out_cubic(elapsed / REVEAL_DURATION_SEC)
    }

    /// Whether a reveal is running, so the map keeps requesting frames.
    pub(crate) fn is_active(&self) -> bool {
        self.start.is_some()
    }
}

/// Cubic ease-out over `t` in `[0, 1]`: fast at the start, settling at the end.
fn ease_out_cubic(t: f32) -> f32 {
    let remaining = 1.0 - t.clamp(0.0, 1.0);
    1.0 - remaining * remaining * remaining
}

/// The size and colour the match halos paint at this frame: the settled values
/// plus whatever the reveal adds on top. Built once per frame and handed to
/// every halo pass.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct HaloStyle {
    /// Width of the band along a matched stretch.
    pub(crate) band_width: f32,
    /// Radius of the ring around a single-point match, icon padding included.
    pub(crate) ring_radius: f32,
    pub(crate) ring_stroke_width: f32,
    /// How far a layer colour is pushed toward
    /// [`gt_ui_theme::QUERY_MATCH_REVEAL_PEAK_ALPHA`], in `[0, 1]`.
    alpha_boost: f32,
}

impl HaloStyle {
    /// The halos as they paint at `reveal`, where 1.0 is the start of a reveal
    /// and 0.0 the settled state.
    ///
    /// The reveal's extra size is a screen-pixel budget scaled by
    /// [`tpv_renderer::glyph_size_scale`], so it grows and shrinks with the fix
    /// icons instead of swamping a zoomed-out map.
    pub(crate) fn new(style: &TpvDrawStyle, reveal: f32) -> Self {
        let reveal = reveal.clamp(0.0, 1.0);
        let budget = reveal * tpv_renderer::glyph_size_scale(style);
        Self {
            band_width: SETTLED_BAND_WIDTH + budget * REVEAL_BAND_WIDTH_BUDGET_PX,
            ring_radius: style.base_arrow_size
                + RING_RADIUS_PADDING
                + budget * REVEAL_RING_RADIUS_BUDGET_PX,
            ring_stroke_width: SETTLED_RING_STROKE_WIDTH + budget * REVEAL_RING_STROKE_BUDGET_PX,
            alpha_boost: reveal,
        }
    }

    /// A halo colour as it paints this frame: its own alpha while settled,
    /// interpolated toward the reveal's peak alpha while one is running.
    pub(crate) fn revealed_color(self, color: Color32) -> Color32 {
        if self.alpha_boost <= 0.0 {
            return color;
        }
        let [r, g, b, alpha] = color.to_srgba_unmultiplied();
        let headroom = f32::from(gt_ui_theme::QUERY_MATCH_REVEAL_PEAK_ALPHA.saturating_sub(alpha));
        #[expect(
            clippy::cast_sign_loss,
            reason = "headroom and alpha_boost.clamp(0, 1) are both non-negative"
        )]
        let boost = (headroom * self.alpha_boost.clamp(0.0, 1.0)).round() as u8;
        Color32::from_rgba_unmultiplied(r, g, b, alpha.saturating_add(boost))
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use gt_types::{FileIdx, TrackIdx, TrackRef};
    use gt_ui_types::DrawLayer;
    use rstest::rstest;

    use super::*;

    /// The zoom ends of [`tpv_renderer::glyph_size_scale`]: fully shrunk icons
    /// at zoom 12 and below, full size at zoom 18 and above.
    const ZOOMED_OUT: f64 = 12.0;
    const ZOOMED_IN: f64 = 18.0;

    fn matches_with_halos(run: u64, stale: bool) -> QueryMatches {
        let track = TrackRef::new(FileIdx::new(0), TrackIdx::new(0));
        // A range built from arguments, so the single-element vec does not
        // trip clippy's `single_range_in_vec_init`.
        let rng = |start: usize, end: usize| start..end;
        QueryMatches {
            draws: vec![DrawLayer {
                color: 0,
                ranges: HashMap::from([(track, vec![rng(0, 3)])]),
            }],
            stale,
            run,
            ..QueryMatches::default()
        }
    }

    #[test]
    #[expect(clippy::float_cmp, reason = "the easing endpoints are exact")]
    fn easing_rises_monotonically_from_zero_to_one() {
        assert_eq!(ease_out_cubic(0.0), 0.0);
        assert_eq!(ease_out_cubic(1.0), 1.0);
        let mut previous = 0.0;
        for step in 0..=20 {
            let eased = ease_out_cubic(step as f32 / 20.0);
            assert!(eased >= previous, "eased {eased} fell below {previous}");
            previous = eased;
        }
        // Ease-out: more than half the distance is covered in the first half.
        assert!(ease_out_cubic(0.5) > 0.5);
    }

    #[test]
    #[expect(clippy::float_cmp, reason = "the reveal endpoints are exact")]
    fn the_reveal_settles_after_its_duration() {
        let mut state = MatchRevealState::default();
        state.restart(100.0);
        assert_eq!(state.tick(100.0), 1.0, "fully inflated at the start");
        assert!(state.is_active());
        let midway = state.tick(100.4);
        assert!(
            (0.0..1.0).contains(&midway),
            "midway amount {midway} left the range"
        );
        assert_eq!(state.tick(100.8), 0.0, "settled at the end");
        assert!(!state.is_active(), "the clock clears when it settles");
        assert_eq!(state.tick(101.0), 0.0);
    }

    #[rstest]
    #[case::zoomed_out(ZOOMED_OUT, 0.25)]
    #[case::zoomed_in(ZOOMED_IN, 1.0)]
    fn a_reveal_inflates_the_halos_by_the_zoom_scaled_budget(
        #[case] zoom: f64,
        #[case] scale: f32,
    ) {
        let style = tpv_renderer::frame_style(zoom);
        let settled = HaloStyle::new(&style, 0.0);
        let revealed = HaloStyle::new(&style, 1.0);
        let settled_ring_radius = style.base_arrow_size + 5.0;

        for (what, actual, expected) in [
            ("settled band width", settled.band_width, 22.0),
            (
                "settled ring radius",
                settled.ring_radius,
                settled_ring_radius,
            ),
            ("settled ring stroke", settled.ring_stroke_width, 3.0),
            (
                "revealed band width",
                revealed.band_width,
                22.0 + 24.0 * scale,
            ),
            (
                "revealed ring radius",
                revealed.ring_radius,
                settled_ring_radius + 18.0 * scale,
            ),
            (
                "revealed ring stroke",
                revealed.ring_stroke_width,
                3.0 + 3.0 * scale,
            ),
        ] {
            assert!(
                (actual - expected).abs() < 0.001,
                "{what} is {actual}, expected {expected}"
            );
        }
    }

    #[test]
    fn a_reveal_brightens_the_halo_colour_to_the_peak_alpha() {
        let style = tpv_renderer::frame_style(ZOOMED_IN);
        let color = gt_ui_theme::query_halo_color(0, false);
        assert_eq!(
            HaloStyle::new(&style, 0.0).revealed_color(color),
            color,
            "a settled halo keeps its own colour untouched"
        );
        let revealed = HaloStyle::new(&style, 1.0).revealed_color(color);
        assert_eq!(
            revealed.to_srgba_unmultiplied()[3],
            gt_ui_theme::QUERY_MATCH_REVEAL_PEAK_ALPHA
        );
        let halfway = HaloStyle::new(&style, 0.5).revealed_color(color);
        let own_alpha = color.to_srgba_unmultiplied()[3];
        let peak = gt_ui_theme::QUERY_MATCH_REVEAL_PEAK_ALPHA;
        assert!(
            (own_alpha..peak).contains(&halfway.to_srgba_unmultiplied()[3]),
            "a halfway reveal lands between the layer alpha and the peak"
        );
    }

    #[test]
    fn a_new_run_with_halos_reveals_once() {
        let mut state = MatchRevealState::default();
        state.start_reveal_for_new_run(Some(&matches_with_halos(1, false)), 10.0);
        assert!(state.is_active(), "a new run reveals");

        state.tick(11.0);
        assert!(!state.is_active());
        state.start_reveal_for_new_run(Some(&matches_with_halos(1, false)), 12.0);
        assert!(!state.is_active(), "the same run does not reveal again");

        state.start_reveal_for_new_run(Some(&matches_with_halos(2, false)), 13.0);
        assert!(state.is_active(), "the next run reveals");
    }

    #[rstest]
    #[case::no_run(matches_with_halos(0, false))]
    #[case::stale(matches_with_halos(2, true))]
    #[case::without_halos(QueryMatches { run: 3, ..QueryMatches::default() })]
    fn matches_that_never_reveal_are_still_recorded(#[case] matches: QueryMatches) {
        let mut state = MatchRevealState::default();
        state.start_reveal_for_new_run(Some(&matches), 10.0);
        assert!(!state.is_active());

        // Repeating them cannot make the reveal fire later either.
        state.start_reveal_for_new_run(Some(&matches), 11.0);
        assert!(!state.is_active());
    }

    #[test]
    #[expect(clippy::float_cmp, reason = "a restarted reveal is exactly 1.0")]
    fn a_re_fire_reveals_the_run_that_already_revealed() {
        let mut state = MatchRevealState::default();
        let matches = matches_with_halos(1, false);
        state.start_reveal_for_new_run(Some(&matches), 10.0);
        state.tick(11.0);
        assert!(!state.is_active());

        state.restart(12.0);
        assert_eq!(state.tick(12.0), 1.0, "the reveal plays from the start");
        state.start_reveal_for_new_run(Some(&matches), 12.0);
        assert!(
            state.is_active(),
            "seeing the same run again leaves it running"
        );
    }
}
