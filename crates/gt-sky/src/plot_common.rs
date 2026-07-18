//! Small helpers shared by the per-report sky plot and the whole-track
//! trails plot: satellite labels and pointer hit-testing.

use egui::Pos2;

use gt_types::satellites::{Constellation, Prn};

/// The `"G05 GPS"` designator: RINEX prefix, zero-padded PRN, constellation
/// name. Single source for both plots' hover labels.
pub(crate) fn satellite_designator(constellation: Constellation, prn: Prn) -> String {
    format!(
        "{}{:02} {}",
        constellation.prn_prefix(),
        prn.value(),
        constellation.display_name()
    )
}

/// The candidate nearest to `pointer`, within `radius`. `candidates` yields
/// each item with its screen position; the closest one inside the radius
/// wins. Shared by the mark tooltip and the slip tooltip.
pub(crate) fn nearest_within<'a, T>(
    candidates: impl Iterator<Item = (&'a T, Pos2)>,
    pointer: Pos2,
    radius: f32,
) -> Option<&'a T> {
    candidates
        .map(|(item, pos)| (item, pos.distance(pointer)))
        .filter(|(_, distance)| *distance <= radius)
        .min_by(|(_, a), (_, b)| a.total_cmp(b))
        .map(|(item, _)| item)
}

#[cfg(test)]
mod tests {
    use super::nearest_within;

    #[test]
    fn nearest_within_picks_the_closest_inside_the_radius() {
        let a = (1u32, egui::pos2(0.0, 0.0));
        let b = (2u32, egui::pos2(10.0, 0.0));
        let items = [a, b];
        let candidates = || items.iter().map(|(v, p)| (v, *p));

        // Closest of the two, both inside radius.
        assert_eq!(
            nearest_within(candidates(), egui::pos2(6.0, 0.0), 8.0),
            Some(&2)
        );
        assert_eq!(
            nearest_within(candidates(), egui::pos2(3.0, 0.0), 8.0),
            Some(&1)
        );
        // Nothing within the radius.
        assert_eq!(
            nearest_within(candidates(), egui::pos2(20.0, 0.0), 5.0),
            None
        );
    }
}
