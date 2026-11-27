use std::ops::RangeInclusive;

use egui_plot::PlotPoint;
use rstest::rstest;

use crate::downsample::{DOWNSAMPLE_WINDOW, MIN_LEVEL_POINTS};
use crate::{KeyBounds, KeyedMipMap, KeyedSample, MipMap, SelectionRange, WrapPeriod};

fn seq(n: usize) -> Vec<[f64; 2]> {
    (0..n).map(|i| [i as f64, (i % 10) as f64]).collect()
}

fn int_data(n: usize) -> Vec<[f64; 2]> {
    (0..n).map(|i| [i as f64, i as f64]).collect()
}

fn selected_x(range: SelectionRange) -> Vec<i64> {
    MipMap::build(int_data(50))
        .select_slice(range, FINEST)
        .iter()
        .map(|p| p.x as i64)
        .collect()
}

fn cascading_input_len() -> usize {
    MIN_LEVEL_POINTS * DOWNSAMPLE_WINDOW
}

fn sampled_per_second(values: &[f64]) -> Vec<[f64; 2]> {
    values
        .iter()
        .enumerate()
        .map(|(i, &value)| [i as f64, value])
        .collect()
}

fn coarse_values(mipmap: &MipMap) -> Vec<f64> {
    mipmap
        .level(1)
        .map(|level| level.iter().map(|p| p.y).collect())
        .unwrap_or_default()
}

fn keyed_int_mipmap(n: usize) -> KeyedMipMap<i64, usize> {
    KeyedMipMap::build(
        (0..n).map(|i| KeyedSample::with_meta([i as f64, i as f64], i as i64 * 10, i)),
    )
}

fn reference_keyed_select<K: Copy + Ord, M>(
    mipmap: &KeyedMipMap<K, M>,
    viewport: RangeInclusive<f64>,
    bounds: KeyBounds<K>,
    target_count: usize,
) -> &[PlotPoint] {
    if mipmap.is_empty() || bounds.is_empty() {
        return &[];
    }
    let x_min = *viewport.start();
    let x_max = (*viewport.end()).max(x_min);
    for level_idx in (0..mipmap.level_count()).rev() {
        let Some((points, keys)) = mipmap.level_slice(level_idx) else {
            continue;
        };
        let vp_start = points.partition_point(|p| p.x < x_min);
        let vp_end = points.partition_point(|p| p.x <= x_max).max(vp_start);
        let key_start = keys.partition_point(|&k| bounds.is_before(k));
        let key_end = keys
            .partition_point(|&k| !bounds.is_after(k))
            .max(key_start);
        let count = vp_end.min(key_end).saturating_sub(vp_start.max(key_start));
        if count >= target_count || level_idx == 0 {
            let clip_start = vp_start.saturating_sub(1).clamp(key_start, key_end);
            let clip_end = vp_end.saturating_add(1).clamp(key_start, key_end);
            return points.get(clip_start..clip_end).unwrap_or_default();
        }
    }
    &[]
}

#[test]
fn empty_mipmap() {
    let m = MipMap::build(Vec::new());
    assert!(m.is_empty());
    assert_eq!(m.original_len(), 0);
    let empty: &[PlotPoint] = &[];
    assert_eq!(
        m.select_slice(SelectionRange::within_viewport(0.0..=100.0), 10),
        empty
    );
}

#[rstest]
#[case::fifty_points(50)]
#[case::ten_thousand_points(10_000)]
fn a_cascade_reduces_to_one_downsample_window(#[case] point_count: usize) {
    let mipmap = MipMap::build(seq(point_count));
    assert!(
        mipmap.level_count() > 1,
        "{point_count} points produced a single level"
    );
    let coarsest = mipmap.coarsest_level().expect("at least one level");
    assert!(coarsest.len() >= MIN_LEVEL_POINTS);
    assert!(
        coarsest.len() <= DOWNSAMPLE_WINDOW,
        "the coarsest level has {} points",
        coarsest.len()
    );
}

#[rstest]
#[case::one_point(1)]
#[case::two_points(2)]
fn tiny_data_stays_single_level(#[case] count: usize) {
    let m = MipMap::build(int_data(count));
    assert_eq!(m.level_count(), 1);
    assert_eq!(m.original_len(), count);
    let km = keyed_int_mipmap(count);
    assert_eq!(km.level_count(), 1);
    assert_eq!(km.original_len(), count);
}

#[test]
fn outliers_preserved_in_downsampled_level() {
    let mut data: Vec<[f64; 2]> = (0..800).map(|i| [i as f64, 1.0]).collect();
    data[4] = [4.0, 1000.0];
    let m = MipMap::build(data);
    assert!(m.level_count() > 1, "800 points produced a single level");
    let has_spike = m
        .coarsest_level()
        .expect("at least one")
        .iter()
        .any(|p| p.y > 500.0);
    assert!(has_spike, "min/max downsampling must preserve spikes");
}

#[rstest]
#[case::extends_one_point_past_each_edge(
    SelectionRange::within_viewport(10.0..=12.0),
    vec![9, 10, 11, 12, 13]
)]
#[case::no_point_before_the_first(
    SelectionRange::within_viewport(0.0..=2.0),
    vec![0, 1, 2, 3]
)]
#[case::no_point_after_the_last(
    SelectionRange::within_viewport(47.0..=49.0),
    vec![46, 47, 48, 49]
)]
#[case::a_wider_hard_bound_leaves_the_extension(
    SelectionRange::within_viewport(10.0..=12.0).bounded_by(0.0..=49.0),
    vec![9, 10, 11, 12, 13]
)]
#[case::the_hard_bound_edges_include_their_own_points(
    SelectionRange::within_viewport(0.0..=49.0).bounded_by(10.0..=12.0),
    vec![10, 11, 12]
)]
#[case::the_hard_bound_edges_lie_between_points(
    SelectionRange::within_viewport(0.0..=49.0).bounded_by(9.5..=12.5),
    vec![10, 11, 12]
)]
#[case::the_hard_bound_lies_between_two_points(
    SelectionRange::within_viewport(0.0..=49.0).bounded_by(20.2..=20.8),
    vec![]
)]
#[case::the_hard_bound_lies_past_the_data(
    SelectionRange::within_viewport(0.0..=49.0).bounded_by(60.0..=70.0),
    vec![]
)]
#[case::the_viewport_lies_past_the_hard_bound(
    SelectionRange::within_viewport(40.0..=49.0).bounded_by(0.0..=10.0),
    vec![]
)]
fn a_selection_extends_past_the_viewport_and_stops_at_the_hard_bound(
    #[case] range: SelectionRange,
    #[case] expected: Vec<i64>,
) {
    assert_eq!(selected_x(range), expected);
}

#[test]
fn a_coarse_level_excludes_points_outside_the_hard_bound() {
    let m = MipMap::build(int_data(10_000));
    let bound = 100.0..=200.0;
    let range = SelectionRange::within_viewport(0.0..=9_999.0).bounded_by(bound.clone());
    let selection = m.select_indices(range, MIN_LEVEL_POINTS);
    let slice = m.slice_at(selection);
    assert!(
        !selection.is_full_detail(),
        "a target of {MIN_LEVEL_POINTS} over 10 000 points reads a downsampled level"
    );
    assert!(
        slice.iter().all(|p| bound.contains(&p.x)),
        "every point must lie inside the hard bound, got {slice:?}"
    );
    assert!(!slice.is_empty(), "the bound spans 101 of the input points");
}

#[test]
fn select_handles_inverted_and_disjoint_ranges_without_panic() {
    let small = MipMap::build(int_data(50));
    let large_n = cascading_input_len() * 4;
    let large = MipMap::build(int_data(large_n));
    let cases: [(&MipMap, f64, f64, usize, f64, &str); 3] = [
        (
            &small,
            30.0,
            10.0,
            5,
            49.0,
            "inverted range collapses to a point",
        ),
        (
            &small,
            1_000.0,
            2_000.0,
            5,
            49.0,
            "disjoint range entirely past the data",
        ),
        (
            &large,
            large_n as f64 / 2.0,
            5.0,
            50,
            large_n as f64,
            "inverted range on a multi-level cascade",
        ),
    ];
    for (m, x_min, x_max, target_count, x_upper_bound, what) in cases {
        let range = SelectionRange::within_viewport(x_min..=x_max);
        let sel = m.select_indices(range, target_count);
        assert!(
            sel.clip_start <= sel.clip_end,
            "{what}: select_indices must restore clip_start <= clip_end, got {sel:?}"
        );
        for slice in [m.select_slice(range, target_count), m.slice_at(sel)] {
            assert!(
                slice.iter().all(|p| p.x >= 0.0 && p.x <= x_upper_bound),
                "{what}: must not return out-of-range points, got {slice:?}"
            );
        }
    }
}

proptest::proptest! {
    #[test]
    fn select_never_panics_for_arbitrary_ranges(
        x_min in -1.0e6_f64..1.0e6_f64,
        x_max in -1.0e6_f64..1.0e6_f64,
        hard_min in -1.0e6_f64..1.0e6_f64,
        hard_max in -1.0e6_f64..1.0e6_f64,
        target_count in 1_usize..100,
    ) {
        let m = MipMap::build(int_data(50));
        let range = SelectionRange::within_viewport(x_min..=x_max)
            .bounded_by(hard_min..=hard_max);
        for slice in [
            m.select_slice(range, target_count),
            m.slice_at(m.select_indices(range, target_count)),
        ] {
            proptest::prop_assert!(slice.iter().all(|p| p.x >= 0.0 && p.x <= 49.0));
            proptest::prop_assert!(
                slice.iter().all(|p| p.x >= hard_min && p.x <= hard_max)
            );
        }
    }
}

#[test]
fn vertical_line_min_and_max_both_survive_downsampling() {
    let n = cascading_input_len();
    let mut data: Vec<[f64; 2]> = (0..n).map(|i| [i as f64, i as f64]).collect();

    let t = DOWNSAMPLE_WINDOW;
    data[t] = [t as f64, 1.0e6];
    data[t + 1] = [t as f64, -1.0e6];

    let m = MipMap::build(data);
    assert!(
        m.level_count() >= 2,
        "a {n}-point input must produce a downsampled level (got {})",
        m.level_count()
    );

    let downsampled = m.level(1).expect("downsampled level");
    #[expect(
        clippy::float_cmp,
        reason = "checking that planted constants survive the pipeline bit-for-bit"
    )]
    {
        assert!(
            downsampled.iter().any(|p| p.y == 1.0e6),
            "spike lost when min and max share a timestamp"
        );
        assert!(
            downsampled.iter().any(|p| p.y == -1.0e6),
            "dip lost when min and max share a timestamp"
        );
    }
}

#[test]
fn nan_does_not_corrupt_min_tracking() {
    let n = cascading_input_len();
    let mut data: Vec<[f64; 2]> = (0..n).map(|i| [i as f64, 0.0]).collect();

    let t = DOWNSAMPLE_WINDOW;
    data[t] = [t as f64, f64::NAN];
    data[t + 3] = [(t + 3) as f64, -1.0e6];

    let m = MipMap::build(data);
    assert!(
        m.level_count() >= 2,
        "a {n}-point input must produce a downsampled level (got {})",
        m.level_count()
    );

    let downsampled = m.level(1).expect("downsampled level");
    #[expect(
        clippy::float_cmp,
        reason = "checking that a planted constant survives the pipeline bit-for-bit"
    )]
    {
        assert!(
            downsampled.iter().any(|p| p.y == -1.0e6),
            "dip lost when NaN appears earlier in the same window"
        );
    }
}

#[test]
fn a_bucket_of_north_jitter_keeps_northward_samples() {
    let period = WrapPeriod::full_turn_degrees();
    let mipmap = MipMap::build_wrapping(
        sampled_per_second(&[359.0, 1.0, 0.5, 359.5, 358.5, 0.0, 1.5, 359.0]),
        period,
    );
    for value in coarse_values(&mipmap) {
        let arc_from_north = period.signed_arc(0.0, value).abs();
        assert!(
            arc_from_north <= 2.0,
            "{value} deg is {arc_from_north} deg from north, further than any sample"
        );
    }
}

#[test]
fn a_bucket_of_opposite_headings_keeps_its_linear_extremes() {
    let mipmap = MipMap::build_wrapping(
        sampled_per_second(&[0.0, 180.0, 0.0, 180.0, 10.0, 11.0, 12.0, 13.0]),
        WrapPeriod::full_turn_degrees(),
    );
    assert_eq!(coarse_values(&mipmap), [0.0, 180.0, 10.0, 13.0]);
}

#[test]
fn a_bucket_keeps_the_extreme_on_each_side_of_its_mean_direction() {
    let mipmap = MipMap::build_wrapping(
        sampled_per_second(&[0.0, 0.0, 100.0, 190.0]),
        WrapPeriod::full_turn_degrees(),
    );
    assert_eq!(coarse_values(&mipmap), [0.0, 190.0]);
}

#[rstest]
#[case::within_one_period([359.0, 1.0, 180.0, 2.0], [359.0, 180.0])]
#[case::past_a_full_turn([0.0, 315.0, 360.0, 675.0], [0.0, 675.0])]
#[case::not_finite([0.0, 315.0, f64::INFINITY, 350.0], [0.0, f64::INFINITY])]
fn a_wrapping_bucket_is_downsampled_by_the_rule_for_its_samples(
    #[case] headings: [f64; 4],
    #[case] expected: [f64; 2],
) {
    let mipmap = MipMap::build_wrapping(
        sampled_per_second(&headings),
        WrapPeriod::full_turn_degrees(),
    );
    assert_eq!(coarse_values(&mipmap), expected);
}

#[test]
fn a_period_that_is_not_finite_and_positive_is_no_period() {
    for period in [0.0, -360.0, f64::NAN, f64::INFINITY] {
        assert_eq!(WrapPeriod::new(period), None, "{period} is no wrap period");
    }
}

proptest::proptest! {
    #[test]
    fn every_wrapping_level_is_a_subset_of_the_finest(
        values in proptest::collection::vec(0.0_f64..360.0, 1..200),
        period_deg in 0.1_f64..1000.0,
    ) {
        let Some(period) = WrapPeriod::new(period_deg) else {
            return Err(proptest::test_runner::TestCaseError::fail(
                format!("{period_deg} is a finite, positive period"),
            ));
        };
        let mipmap = MipMap::build_wrapping(sampled_per_second(&values), period);
        let finest = mipmap.finest_level();
        for level_idx in 0..mipmap.level_count() {
            let level = mipmap.level(level_idx).unwrap_or_default();
            for point in level {
                proptest::prop_assert!(
                    finest
                        .iter()
                        .any(|sample| sample.x.to_bits() == point.x.to_bits()
                            && sample.y.to_bits() == point.y.to_bits()),
                    "{point:?} is not an input sample"
                );
            }
        }
    }

    #[test]
    fn a_wrapping_bucket_of_out_of_range_values_keeps_its_linear_extremes(
        buckets in proptest::collection::vec(
            proptest::array::uniform4(-720.0_f64..1080.0),
            1..25,
        ),
    ) {
        let values: Vec<f64> = buckets.iter().flatten().copied().collect();
        let mipmap = MipMap::build_wrapping(
            sampled_per_second(&values),
            WrapPeriod::full_turn_degrees(),
        );
        let coarse = mipmap.level(1).unwrap_or_default();
        for (i, bucket) in buckets.iter().enumerate() {
            if bucket.iter().all(|&value| (0.0..360.0).contains(&value)) {
                continue;
            }
            let Some(kept) = coarse.get(i * 2..i * 2 + 2) else {
                return Err(proptest::test_runner::TestCaseError::fail(
                    format!("bucket {i} emitted no pair"),
                ));
            };
            let mut got: Vec<f64> = kept.iter().map(|p| p.y).collect();
            got.sort_by(f64::total_cmp);
            let mut expected = bucket.to_vec();
            expected.sort_by(f64::total_cmp);
            proptest::prop_assert_eq!(
                got,
                vec![expected[0], expected[3]],
                "bucket {:?} lost a linear extreme",
                bucket
            );
        }
    }
}

#[test]
fn a_raw_coarse_level_preserves_input_order_at_equal_x() {
    let mipmap = MipMap::build([
        [0.0, 4.0],
        [1.0, 1.0],
        [1.0, 9.0],
        [2.0, 3.0],
        [3.0, 4.0],
        [4.0, 5.0],
    ]);
    assert_eq!(
        mipmap.select_slice(SelectionRange::within_viewport(0.0..=4.0), 4),
        [
            PlotPoint::new(1.0, 1.0),
            PlotPoint::new(1.0, 9.0),
            PlotPoint::new(3.0, 4.0),
            PlotPoint::new(4.0, 5.0),
        ]
    );
}

#[test]
fn a_keyed_coarse_level_filters_and_identifies_equal_x_samples() {
    let mipmap = KeyedMipMap::build(
        (0_usize..16)
            .map(|index| KeyedSample::with_meta([10.0, index as f64], index as i64, index)),
    );
    let selection = mipmap.select_indices(9.0..=11.0, KeyBounds::inclusive(7..=8), 2);
    assert!(!selection.is_full_detail());
    assert_eq!(
        mipmap.slice_at(selection),
        [PlotPoint::new(10.0, 7.0), PlotPoint::new(10.0, 8.0)]
    );
    assert_eq!(mipmap.key_at(selection, 0), Some(7));
    assert_eq!(mipmap.key_at(selection, 1), Some(8));
    assert_eq!(mipmap.key_at(selection, 2), None);
    assert_eq!(mipmap.meta_at(selection, 0), Some(&7));
    assert_eq!(mipmap.meta_at(selection, 1), Some(&8));
    assert_eq!(mipmap.meta_at(selection, 2), None);
}

#[test]
fn keyed_build_with_period_preserves_wrapping_extremes_and_metadata() {
    let headings = [359.0, 1.0, 180.0, 2.0];
    let mipmap = KeyedMipMap::build_with_period(
        headings
            .iter()
            .enumerate()
            .map(|(i, &heading)| KeyedSample::with_meta([i as f64, heading], i as i64 * 10, i)),
        Some(WrapPeriod::full_turn_degrees()),
    );
    let selection = mipmap.select_indices(0.0..=3.0, KeyBounds::All, 2);
    assert!(!selection.is_full_detail());
    assert_eq!(
        mipmap.slice_at(selection),
        [PlotPoint::new(0.0, 359.0), PlotPoint::new(2.0, 180.0)]
    );
    assert_eq!(mipmap.key_at(selection, 0), Some(0));
    assert_eq!(mipmap.key_at(selection, 1), Some(20));
    assert_eq!(mipmap.meta_at(selection, 0), Some(&0));
    assert_eq!(mipmap.meta_at(selection, 1), Some(&2));
}

#[rstest]
#[case::empty(None, KeyBounds::Empty)]
#[case::all(Some((None, None)), KeyBounds::All)]
#[case::from(Some((Some(10), None)), KeyBounds::From(10))]
#[case::until(Some((None, Some(20))), KeyBounds::Until(20))]
#[case::between(Some((Some(10), Some(20))), KeyBounds::Between { start: 10, end: 20 })]
#[case::inverted(Some((Some(20), Some(10))), KeyBounds::Empty)]
fn key_bounds_from_bounds_maps_optional_endpoints(
    #[case] input: Option<(Option<i64>, Option<i64>)>,
    #[case] expected: KeyBounds<i64>,
) {
    assert_eq!(KeyBounds::from_bounds(input), expected);
}

#[rstest]
#[case::empty(KeyBounds::Empty)]
#[case::inverted_inclusive(KeyBounds::inclusive(RangeInclusive::new(50, 10)))]
#[case::inverted_between(KeyBounds::Between { start: 50, end: 10 })]
fn empty_and_inverted_key_bounds_select_nothing(#[case] bounds: KeyBounds<i64>) {
    let mipmap = keyed_int_mipmap(50);
    assert!(bounds.is_empty());
    assert!(!mipmap.has_samples_in(bounds));
    assert_eq!(mipmap.clipped_x_range(bounds), None);
    assert!(mipmap.select_slice(0.0..=49.0, bounds, 2).is_empty());
}

#[rstest]
#[case::extends_within_all(10.0..=12.0, KeyBounds::All, vec![9, 10, 11, 12, 13])]
#[case::extends_within_wider_bounds(
    10.0..=12.0,
    KeyBounds::inclusive(0..=490),
    vec![9, 10, 11, 12, 13]
)]
#[case::clamped_at_key_bounds(
    0.0..=49.0,
    KeyBounds::inclusive(100..=120),
    vec![10, 11, 12]
)]
#[case::single_key_inside_bounds(
    0.0..=49.0,
    KeyBounds::inclusive(150..=150),
    vec![15]
)]
#[case::viewport_straddling_lower_bound(
    8.0..=12.0,
    KeyBounds::From(100),
    vec![10, 11, 12, 13]
)]
#[case::viewport_straddling_upper_bound(
    8.0..=12.0,
    KeyBounds::Until(100),
    vec![7, 8, 9, 10]
)]
#[case::viewport_entirely_before_key_bounds(
    0.0..=5.0,
    KeyBounds::From(200),
    vec![]
)]
#[case::viewport_entirely_after_key_bounds(
    30.0..=49.0,
    KeyBounds::Until(100),
    vec![]
)]
#[case::viewport_adjacent_to_single_bounded_point_on_left(
    10.0..=20.0,
    KeyBounds::inclusive(90..=90),
    vec![9]
)]
#[case::viewport_adjacent_to_single_bounded_point_on_right(
    10.0..=20.0,
    KeyBounds::inclusive(210..=210),
    vec![21]
)]
#[case::viewport_between_two_points_inside_bounds(
    10.2..=10.8,
    KeyBounds::inclusive(100..=110),
    vec![10, 11]
)]
#[case::bounds_between_two_keys(
    0.0..=49.0,
    KeyBounds::inclusive(102..=108),
    vec![]
)]
fn keyed_selection_clamps_extension_to_key_bounds(
    #[case] viewport: RangeInclusive<f64>,
    #[case] bounds: KeyBounds<i64>,
    #[case] expected: Vec<i64>,
) {
    let mipmap = keyed_int_mipmap(50);
    let got: Vec<i64> = mipmap
        .select_slice(viewport, bounds, FINEST)
        .iter()
        .map(|p| p.x as i64)
        .collect();
    assert_eq!(got, expected);
}

#[rstest]
#[case::all(KeyBounds::All, true, Some((0.0, 49.0)))]
#[case::from_middle(KeyBounds::From(250), true, Some((25.0, 49.0)))]
#[case::until_middle(KeyBounds::Until(100), true, Some((0.0, 10.0)))]
#[case::between_exact(KeyBounds::inclusive(100..=200), true, Some((10.0, 20.0)))]
#[case::between_in_gap(KeyBounds::inclusive(101..=109), false, None)]
#[case::past_end(KeyBounds::From(500), false, None)]
#[case::before_start(KeyBounds::Until(-1), false, None)]
fn keyed_has_samples_and_clipped_x_range(
    #[case] bounds: KeyBounds<i64>,
    #[case] expected_present: bool,
    #[case] expected_range: Option<(f64, f64)>,
) {
    let mipmap = keyed_int_mipmap(50);
    assert_eq!(mipmap.has_samples_in(bounds), expected_present);
    assert_eq!(mipmap.clipped_x_range(bounds), expected_range);
}

proptest::proptest! {
    #[test]
    fn keyed_select_matches_full_level_reference(
        x_min in -10.0_f64..40.0_f64,
        x_max in -10.0_f64..40.0_f64,
        bound_kind in 0_u8..6,
        key_start in -10_i64..140,
        key_end in -10_i64..140,
        target_count in 1_usize..40,
    ) {
        // Groups of 4 samples share one plot x with strictly increasing keys.
        let mipmap = KeyedMipMap::build(
            (0_i64..128)
                .map(|key| KeyedSample::with_meta([(key / 4) as f64, (key % 11) as f64], key, key)),
        );
        let bounds = match bound_kind {
            0 => KeyBounds::All,
            1 => KeyBounds::Empty,
            2 => KeyBounds::From(key_start),
            3 => KeyBounds::Until(key_end),
            4 => KeyBounds::inclusive(key_start..=key_end),
            _ => KeyBounds::Between {
                start: key_start,
                end: key_end,
            },
        };
        let sel = mipmap.select_indices(x_min..=x_max, bounds, target_count);
        let actual = mipmap.slice_at(sel);
        let expected = reference_keyed_select(&mipmap, x_min..=x_max, bounds, target_count);
        proptest::prop_assert_eq!(actual, expected);
        for offset in 0..actual.len() {
            let key = mipmap.key_at(sel, offset);
            proptest::prop_assert!(key.is_some_and(|k| bounds.contains(k)));
            proptest::prop_assert_eq!(mipmap.meta_at(sel, offset).copied(), key);
        }
    }
}

/// Target above every `int_data` fixture, forcing finest-level selection while
/// still exercising the +/-1 edge extension.
const FINEST: usize = 100;
