//! How the results tab divides its height between the matches table and the
//! points table below it.

/// Rows either table keeps however far the splitter between them is dragged,
/// so neither can be collapsed to its header alone.
pub(crate) const MIN_SPLIT_ROWS: usize = 2;

/// The share of the results tab the matches table takes. Kept for as long as
/// the query window keeps the sort order and the picked match.
#[derive(Debug, Default, Clone, Copy, PartialEq)]
pub(super) struct ResultsSplit {
    /// `None` until the splitter is dragged: the matches table then opens at
    /// [`SplitGeometry::matches_default`].
    matches_fraction: Option<f32>,
}

impl ResultsSplit {
    /// The height the matches table takes in a tab of `geometry`.
    pub(super) fn matches_height(self, geometry: SplitGeometry) -> f32 {
        let wanted = match self.matches_fraction {
            Some(fraction) => fraction * geometry.available,
            None => geometry.matches_default,
        };
        geometry.clamped_matches_height(wanted)
    }

    /// Records where a splitter drag left the boundary, as the share of the
    /// tab the matches table above it then takes.
    pub(super) fn set_matches_height(&mut self, geometry: SplitGeometry, height: f32) {
        if geometry.available <= 0.0 {
            return;
        }
        self.matches_fraction = Some(geometry.clamped_matches_height(height) / geometry.available);
    }

    /// Back to [`SplitGeometry::matches_default`], as a double-click on the
    /// splitter leaves it.
    pub(super) fn reset(&mut self) {
        self.matches_fraction = None;
    }
}

/// The heights one results tab has to divide between its two tables, measured
/// from the text styles before either one is laid out.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct SplitGeometry {
    /// Everything under the summary strip: both tables, the splitter band, and
    /// the caption stating the picked match.
    pub(super) available: f32,
    /// The matches table's header and [`MIN_SPLIT_ROWS`] of its rows.
    pub(super) matches_minimum: f32,
    /// The matches table's header and every row it lists: the splitter never
    /// leaves it taller than the matches it has to show.
    pub(super) matches_content: f32,
    /// What the matches table takes until the splitter is dragged.
    pub(super) matches_default: f32,
    /// The caption, the points table's header, and [`MIN_SPLIT_ROWS`] of its
    /// rows.
    pub(super) points_minimum: f32,
    /// The splitter band and the gap on either side of it.
    pub(super) splitter: f32,
}

impl SplitGeometry {
    /// `height` brought within what the tab has room for. A tab too short for
    /// both minimums keeps the points table's and leaves the matches table
    /// what remains.
    fn clamped_matches_height(self, height: f32) -> f32 {
        let highest = (self.available - self.splitter - self.points_minimum)
            .min(self.matches_content)
            .max(0.0);
        height.clamp(self.matches_minimum.min(highest), highest)
    }
}

#[cfg(test)]
mod tests {
    use rstest::rstest;

    use super::*;

    /// A 400 px tab whose matches table opens at 100 px and lists more matches
    /// than its 300 px of room can hold, so the points table's minimum is what
    /// bounds a downward drag: 400 - 40 - 60 = 300.
    fn geometry() -> SplitGeometry {
        SplitGeometry {
            available: 400.0,
            matches_minimum: 50.0,
            matches_content: 500.0,
            matches_default: 100.0,
            points_minimum: 60.0,
            splitter: 40.0,
        }
    }

    /// Heights are pixels a table is laid out at: they match within a
    /// hundredth of one, not bit for bit.
    const HEIGHT_TOLERANCE_PX: f32 = 0.01;

    #[track_caller]
    fn assert_matches_height(split: ResultsSplit, geometry: SplitGeometry, expected: f32) {
        let height = split.matches_height(geometry);
        assert!(
            (height - expected).abs() < HEIGHT_TOLERANCE_PX,
            "the matches table took {height}, not {expected}"
        );
    }

    #[test]
    fn an_undragged_split_opens_the_matches_table_at_its_default_height() {
        assert_matches_height(ResultsSplit::default(), geometry(), 100.0);
    }

    #[rstest]
    #[case::under_the_matches_minimum(10.0, 50.0)]
    #[case::over_what_the_points_table_leaves(900.0, 300.0)]
    fn a_dragged_height_is_clamped_to_what_the_tab_has_room_for(
        #[case] dragged: f32,
        #[case] expected: f32,
    ) {
        let mut split = ResultsSplit::default();
        split.set_matches_height(geometry(), dragged);
        assert_matches_height(split, geometry(), expected);
    }

    /// The drag is kept as a share of the tab, so the same split in a tab
    /// twice as tall gives the matches table twice the height.
    #[test]
    fn a_dragged_height_comes_back_as_the_share_of_the_tab_it_was() {
        let mut split = ResultsSplit::default();
        split.set_matches_height(geometry(), 200.0);
        assert_matches_height(split, geometry(), 200.0);

        let taller = SplitGeometry {
            available: 800.0,
            ..geometry()
        };
        assert_matches_height(split, taller, 400.0);
    }

    /// A run of few matches: the table stops where its rows do, well before
    /// the 300 px the points table would still leave it.
    #[test]
    fn the_matches_table_grows_no_taller_than_the_rows_it_lists() {
        let few_matches = SplitGeometry {
            matches_content: 80.0,
            ..geometry()
        };
        let mut split = ResultsSplit::default();
        split.set_matches_height(few_matches, 900.0);
        assert_matches_height(split, few_matches, 80.0);
    }

    /// 120 px leave 20 px once the splitter and the points table's minimum are
    /// taken, which is under the matches table's own minimum.
    #[test]
    fn a_tab_too_short_for_both_minimums_leaves_the_points_table_its_own() {
        let short = SplitGeometry {
            available: 120.0,
            ..geometry()
        };
        let mut split = ResultsSplit::default();
        split.set_matches_height(short, 900.0);
        assert_matches_height(split, short, 20.0);
    }

    #[test]
    fn a_reset_split_opens_the_matches_table_at_its_default_height_again() {
        let mut split = ResultsSplit::default();
        split.set_matches_height(geometry(), 250.0);
        split.reset();
        assert_matches_height(split, geometry(), 100.0);
    }

    #[test]
    fn a_tab_of_no_height_keeps_the_split_it_had() {
        let mut split = ResultsSplit::default();
        split.set_matches_height(geometry(), 200.0);
        split.set_matches_height(
            SplitGeometry {
                available: 0.0,
                ..geometry()
            },
            50.0,
        );
        assert_matches_height(split, geometry(), 200.0);
    }
}
