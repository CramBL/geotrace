pub mod fixtures;

pub use fixtures::{
    marker_test_data, nav_data_with_gap, nav_points_from, nav_test_data, single_nav_point,
    stationary_nav_data,
};

/// Asserts that a slice or Vec exactly matches a sequence of patterns, one per element.
///
/// Fails if the length doesn't match or any element fails its pattern.
/// Uses `assert_matches!` internally for each element, so failure messages include
/// the matched value when `Debug` is implemented.
///
/// # Example
/// ```
/// use gt_test_utils::assert_matches_sequence;
/// let v = vec![Some(1u32), None, Some(3)];
/// assert_matches_sequence!(v, [Some(_), None, Some(_)]);
/// ```
#[macro_export]
macro_rules! assert_matches_sequence {
    ($seq:expr, [$($pat:pat),* $(,)?]) => {{
        let seq = $seq;
        let expected_len = [$(stringify!($pat)),*].len();
        assert_eq!(
            seq.len(),
            expected_len,
            "sequence length mismatch: expected {expected_len}, got {}",
            seq.len()
        );
        let mut _idx = 0usize;
        $(
            assert!(
                matches!(&seq[_idx], $pat),
                "element [{}] did not match pattern `{}`",
                _idx,
                stringify!($pat)
            );
            _idx += 1;
        )*
    }};
}
