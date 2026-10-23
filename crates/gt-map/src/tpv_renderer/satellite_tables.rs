use egui::{Color32, Grid, RichText, Stroke, Vec2};
use gt_sky::SkyHighlight;
use gt_types::satellites::{Constellation, NO_DATA_SNR_EXPLANATION};
use gt_types::{
    NavPoint, SatelliteQueryScope, SatelliteScope, SkyCounts, SkyGeometry, SkySatellite,
};
use gt_ui_theme::labels::LabelWithHover;
use gt_ui_theme::{EM_DASH, SatelliteCountBadge, SatelliteMembershipMark};
use gt_ui_types::PointWindowFolds;
use smallvec::SmallVec;
use strum::{EnumCount as _, IntoEnumIterator as _};

use super::point_summary::PointSummary;

/// The per-constellation satellite tables. Hovering a constellation name, its
/// fix count, or a satellite row highlights the matching marks on the plot.
pub(super) fn sticky_satellites(
    ui: &mut egui::Ui,
    p: &NavPoint,
    folds: &mut PointWindowFolds,
    highlight: &mut Option<SkyHighlight>,
) {
    let Some(evidence) = p.sky_evidence(SkyGeometry::Identities) else {
        return;
    };
    let rows = evidence.satellites();
    ui.add_space(6.0);

    // Constellation declaration order matches the plot legend.
    let groups: SmallVec<[ConstellationGroup<'_>; Constellation::COUNT]> = Constellation::iter()
        .enumerate()
        .filter_map(|(grid_id, constellation)| {
            let mut satellites: Vec<_> = rows
                .iter()
                .filter(|satellite| satellite.constellation() == constellation)
                .copied()
                .collect();
            if satellites.is_empty() {
                return None;
            }
            satellites.sort_by_key(SkySatellite::prn);
            Some(ConstellationGroup {
                grid_id,
                constellation,
                prn_prefix: constellation.prn_prefix(),
                satellites,
                used: p
                    .resolved_used_count(SatelliteQueryScope::Constellation(constellation))
                    .map(|count| count.count),
                seen: p
                    .resolved_seen_count(SatelliteQueryScope::Constellation(constellation))
                    .map(|count| count.count),
            })
        })
        .collect();

    // Never more than two columns: a third would sit far enough from the plot
    // that correlating a row with its mark gets hard.
    let two_columns = groups.len() > 1 && ui.available_width() >= MIN_TWO_COLUMN_WIDTH_PX;
    if !two_columns {
        for group in &groups {
            constellation_panel(ui, group, folds, highlight);
        }
        return;
    }

    let weights: SmallVec<[usize; Constellation::COUNT]> =
        groups.iter().map(|group| group.weight(*folds)).collect();
    let (left, right) = groups.split_at(balanced_split(&weights));
    ui.horizontal_top(|ui| {
        for (column_i, column) in [left, right].into_iter().enumerate() {
            if column_i > 0 {
                ui.add_space(STICKY_COLUMN_GAP_PX);
            }
            ui.vertical(|ui| {
                for group in column {
                    constellation_panel(ui, group, folds, highlight);
                }
            });
        }
    });
}

pub(super) fn show_satellite_rows(ui: &mut egui::Ui, p: &NavPoint) {
    if p.resolved_used_count(SatelliteQueryScope::Global).is_some()
        || p.resolved_seen_count(SatelliteQueryScope::Global).is_some()
    {
        ui.label(RichText::new("Satellites").strong());
        PointSummary(p).counts_ui(ui, SatelliteQueryScope::Global);
        ui.end_row();
    }
    for constellation in Constellation::iter() {
        let scope = SatelliteQueryScope::Constellation(constellation);
        if p.satellite_evidence.usage.iter().any(|report| {
            report.satellites.iter().any(|row| row.identity.constellation == constellation)
                || matches!(report.metadata.scope, SatelliteScope::Constellations(set) if set.contains(constellation))
        }) || p.satellite_evidence.observations.iter().any(|report| {
            report.satellites.iter().any(|row| row.identity.constellation == constellation)
                || matches!(report.metadata.scope, SatelliteScope::Constellations(set) if set.contains(constellation))
        }) {
            ui.horizontal(|ui| {
                constellation_swatch(ui, constellation);
                ui.label(constellation.display_name());
            });
            PointSummary(p).counts_ui(ui, scope);
            ui.end_row();
        }
    }
}

pub(crate) fn constellation_swatch(ui: &mut egui::Ui, constellation: Constellation) {
    let color = gt_ui_theme::constellation_color(constellation, ui.visuals().dark_mode);
    let (rect, _) = ui.allocate_exact_size(Vec2::splat(SWATCH_SIZE_PX), egui::Sense::hover());
    ui.painter()
        .rect_filled(rect.shrink(SWATCH_MARGIN_PX), SWATCH_ROUNDING_PX, color);
}

/// A fold arrow, tinted with the colour of whatever it folds, so one element
/// shows both the fold state and the key to the plot's marks.
pub(super) fn fold_arrow(ui: &mut egui::Ui, folded: bool, color: Color32) {
    let (rect, _) = ui.allocate_exact_size(Vec2::splat(FOLD_ARROW_BOX_PX), egui::Sense::hover());
    if !ui.is_rect_visible(rect) {
        return;
    }
    // Painted, not set as a glyph: only the Regular phosphor weight is loaded,
    // and its caret is too fine to show the constellation colour.
    let c = rect.center();
    let half = FOLD_ARROW_SIZE_PX / 2.0;
    let points = if folded {
        vec![
            egui::pos2(c.x - half, c.y - half),
            egui::pos2(c.x - half, c.y + half),
            egui::pos2(c.x + half, c.y),
        ]
    } else {
        vec![
            egui::pos2(c.x - half, c.y - half),
            egui::pos2(c.x + half, c.y - half),
            egui::pos2(c.x, c.y + half),
        ]
    };
    ui.painter()
        .add(egui::Shape::convex_polygon(points, color, Stroke::NONE));
}

struct ConstellationGroup<'a> {
    /// Salts the panel's `Grid` id, so the two columns' grids stay distinct.
    grid_id: usize,
    constellation: Constellation,
    prn_prefix: &'a str,
    satellites: Vec<SkySatellite>,
    used: Option<u32>,
    seen: Option<u32>,
}

impl ConstellationGroup<'_> {
    /// Header rows participate in column balancing when the constellation is folded.
    fn weight(&self, folds: PointWindowFolds) -> usize {
        if folds.is_folded(self.constellation) {
            FOLDED_PANEL_ROWS
        } else {
            self.satellites.len() + PANEL_HEADER_ROWS
        }
    }
}

fn constellation_panel(
    ui: &mut egui::Ui,
    group: &ConstellationGroup<'_>,
    folds: &mut PointWindowFolds,
    highlight: &mut Option<SkyHighlight>,
) {
    let &ConstellationGroup {
        grid_id,
        constellation,
        prn_prefix,
        ..
    } = group;
    let satellites = group.satellites.as_slice();
    let folded = folds.is_folded(constellation);
    ui.vertical(|ui| {
        let dark_mode = ui.visuals().dark_mode;
        let const_fix = group.used;
        // The full header toggles the fold. The used count highlights the used subset.
        // The remaining header highlights the constellation.
        let header = ui.horizontal(|ui| {
            fold_arrow(
                ui,
                folded,
                gt_ui_theme::constellation_color(constellation, dark_mode),
            );
            ui.label(RichText::new(constellation.display_name()).strong());
            let counts = SatelliteCountBadge(SkyCounts {
                used: const_fix,
                seen: group.seen,
            })
            .ui(ui);
            ui.rect_contains_pointer(counts.used.rect)
        });
        let over_fix_count = header.inner;
        // Interact with an explicit id: sibling panels lay out identically,
        // so the auto-generated container ids collide and the click lands on
        // the wrong panel - or nowhere at all.
        let header = ui.interact(
            header.response.rect,
            ui.id().with(("constellation_fold", constellation)),
            egui::Sense::click(),
        );
        if crate::hover_labels::hover_affordance(ui, header.rect) {
            *highlight = Some(if over_fix_count {
                SkyHighlight::constellation_in_fix(constellation)
            } else {
                SkyHighlight::constellation(constellation)
            });
        }
        if header.clicked() {
            folds.toggle(constellation);
        }
        if folded {
            return;
        }
        Grid::new(("sticky_sats", grid_id))
            .num_columns(3)
            .striped(true)
            .show(ui, |ui| {
                ui.label(RichText::new("PRN").weak().small());
                ui.label(RichText::new("SNR").weak().small());
                ui.label(RichText::new("Used").weak().small());
                ui.end_row();

                let in_fix_color = gt_ui_theme::SatCountTier::Good.color(dark_mode);
                let muted_color = ui.visuals().weak_text_color();
                for sat in satellites {
                    let in_fix = sat.in_fix();
                    let prn_color = if in_fix { in_fix_color } else { muted_color };
                    let prn_resp = ui.label(
                        RichText::new(format!("{}{:02}", prn_prefix, sat.prn())).color(prn_color),
                    );
                    let snr_resp = match sat.snr() {
                        Some(snr) => {
                            let reading = RichText::new(format!("{:.1}", snr.value()))
                                .color(gt_ui_theme::snr_color(snr.quality(), dark_mode));
                            if snr.is_no_data_sentinel() {
                                LabelWithHover::plain(reading)
                                    .explanation_ui(ui, NO_DATA_SNR_EXPLANATION)
                            } else {
                                ui.label(reading)
                            }
                        }
                        None => ui.label(RichText::new(EM_DASH).color(muted_color)),
                    };
                    let color = gt_ui_theme::constellation_color(constellation, dark_mode);
                    let fix_resp = SatelliteMembershipMark {
                        membership: sat.membership,
                        radius: MEMBERSHIP_MARK_RADIUS_PX,
                        minimum_unknown_radius: gt_ui_theme::SKY_UNKNOWN_MEMBERSHIP_MIN_RADIUS_PX,
                        color,
                        edge: if in_fix {
                            Stroke::NONE
                        } else {
                            Stroke::new(MEMBERSHIP_MARK_STROKE_PX, color)
                        },
                        hollow_fill: egui::Color32::TRANSPARENT,
                    }
                    .ui(ui);
                    // The gaps between cells participate in row hover.
                    let row = prn_resp.union(snr_resp).union(fix_resp);
                    if crate::hover_labels::row_hover_affordance(ui, row.rect) {
                        *highlight = Some(SkyHighlight::satellite(constellation, sat.prn()));
                    }
                    ui.end_row();
                }
            });
    });
    ui.add_space(6.0);
}

/// Preserves constellation order while balancing panel row counts.
fn balanced_split(weights: &[usize]) -> usize {
    if weights.len() < 2 {
        return weights.len();
    }
    let total: usize = weights.iter().sum();
    let mut left = 0;
    let mut best_index = 1;
    let mut best_imbalance = usize::MAX;
    for (i, weight) in weights.iter().enumerate().take(weights.len() - 1) {
        left += weight;
        let imbalance = left.abs_diff(total - left);
        if imbalance < best_imbalance {
            best_imbalance = imbalance;
            best_index = i + 1;
        }
    }
    best_index
}

const SWATCH_SIZE_PX: f32 = 10.0;
const SWATCH_ROUNDING_PX: f32 = 2.0;
const SWATCH_MARGIN_PX: f32 = 1.0;

const FOLD_ARROW_SIZE_PX: f32 = 9.0;

const FOLD_ARROW_BOX_PX: f32 = 12.0;

/// Gap between the sticky popup's plot column and its satellite column, and
/// between the two satellite columns.
pub(super) const STICKY_COLUMN_GAP_PX: f32 = 12.0;

/// The readable column width is shared by the table-packing and side-by-side thresholds.
pub(super) const MIN_SATELLITE_COLUMN_WIDTH_PX: f32 = 140.0;

/// Width the satellite area needs before it splits into two columns - two
/// readable columns and the gap between them. Below it the constellations
/// stack in one column.
const MIN_TWO_COLUMN_WIDTH_PX: f32 = 2.0 * MIN_SATELLITE_COLUMN_WIDTH_PX + STICKY_COLUMN_GAP_PX;

const PANEL_HEADER_ROWS: usize = 2;

const FOLDED_PANEL_ROWS: usize = 1;

const MEMBERSHIP_MARK_RADIUS_PX: f32 = 3.0;
const MEMBERSHIP_MARK_STROKE_PX: f32 = 1.5;

#[cfg(test)]
mod tests {
    use gt_types::satellites::Satellite;
    use rstest::rstest;

    use super::*;

    #[rstest]
    #[case::four_constellations(&[13, 10, 8, 17], 2)]
    #[case::two(&[13, 10], 1)]
    #[case::lopsided(&[30, 3, 3], 1)]
    #[case::even(&[10, 10, 10, 10], 2)]
    #[case::one_panel(&[7], 1)]
    #[case::no_panel(&[], 0)]
    fn balanced_split_cuts_where_the_columns_even_out(
        #[case] weights: &[usize],
        #[case] expected: usize,
    ) {
        assert_eq!(balanced_split(weights), expected);
    }

    #[test]
    fn folded_panels_weigh_only_their_header() {
        let group = ConstellationGroup {
            grid_id: 0,
            constellation: Constellation::Gps,
            prn_prefix: "G",
            used: Some(11),
            seen: Some(11),
            satellites: vec![
                SkySatellite::from_legacy(&Satellite::new(
                    Constellation::Gps,
                    1,
                    Some(45.0),
                    Some(40.0),
                    Some(40.0),
                    true
                ));
                11
            ],
        };
        let unfolded = gt_ui_types::PointWindowFolds::default();
        assert_eq!(group.weight(unfolded), 11 + PANEL_HEADER_ROWS);

        let mut folded = unfolded;
        folded.toggle(Constellation::Gps);
        assert_eq!(group.weight(folded), FOLDED_PANEL_ROWS);
    }
}
