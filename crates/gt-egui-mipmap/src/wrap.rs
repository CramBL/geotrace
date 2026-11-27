use egui_plot::PlotPoint;

/// Period at which a series' values wrap, such as 360 degrees for a compass
/// heading or a `.gtd` channel's declared period.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WrapPeriod(f64);

impl WrapPeriod {
    pub const fn full_turn_degrees() -> Self {
        Self(FULL_TURN_DEGREES)
    }

    /// `None` unless `period` is finite and positive.
    pub fn new(period: f64) -> Option<Self> {
        (period.is_finite() && period > 0.0).then_some(Self(period))
    }

    /// Two offsets within `chunk` furthest to either side of its circular mean
    /// direction, or `None` when a value falls outside `[0, period)` or the
    /// samples cancel out below [`MEAN_RESULTANT_FLOOR`].
    pub(crate) fn extremes_either_side_of_mean_direction(
        self,
        chunk: &[PlotPoint],
    ) -> Option<[usize; 2]> {
        if !chunk.iter().all(|point| self.contains(point.y)) {
            return None;
        }
        let mean = self.mean_direction(chunk.iter().map(|point| point.y))?;
        let arc_from_mean = |point: &PlotPoint| self.signed_arc(mean, point.y);
        let (counterclockwise, _) = chunk
            .iter()
            .enumerate()
            .min_by(|(_, a), (_, b)| arc_from_mean(a).total_cmp(&arc_from_mean(b)))?;
        let (clockwise, _) = chunk
            .iter()
            .enumerate()
            .max_by(|(_, a), (_, b)| arc_from_mean(a).total_cmp(&arc_from_mean(b)))?;
        Some([counterclockwise, clockwise])
    }

    /// Signed arc from `from` to `to` in `[-period / 2, period / 2)`:
    /// negative counterclockwise, positive clockwise.
    pub(crate) fn signed_arc(self, from: f64, to: f64) -> f64 {
        let half_period = self.0 / 2.0;
        (to - from + half_period).rem_euclid(self.0) - half_period
    }

    fn mean_direction(self, values: impl Iterator<Item = f64>) -> Option<f64> {
        let scale = std::f64::consts::TAU / self.0;
        let (sines, cosines) = values.fold((0.0, 0.0), |(sines, cosines), value| {
            let radians = value * scale;
            (sines + radians.sin(), cosines + radians.cos())
        });
        (sines.hypot(cosines) > MEAN_RESULTANT_FLOOR).then(|| sines.atan2(cosines) / scale)
    }

    fn contains(self, value: f64) -> bool {
        (0.0..self.0).contains(&value)
    }
}

const FULL_TURN_DEGREES: f64 = 360.0;

/// Minimum resultant length required for a window's circular mean direction to
/// be defined.
const MEAN_RESULTANT_FLOOR: f64 = 1e-9;
