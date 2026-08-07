//! Robust statistics shared by the clock analyses.

/// Median of `values` (averaging the two central elements for an even count).
/// Returns 0 for an empty input. Callers guard against that.
pub fn median_i64(values: &[i64]) -> i64 {
    if values.is_empty() {
        return 0;
    }
    let mut sorted = values.to_vec();
    sorted.sort_unstable();
    let mid = sorted.len() / 2;
    let hi = sorted.get(mid).copied().unwrap_or(0);
    if sorted.len() % 2 == 1 {
        hi
    } else {
        let lo = sorted.get(mid - 1).copied().unwrap_or(0);
        // Overflow-safe midpoint: `lo + hi` can exceed i64 for extreme inputs.
        #[expect(
            clippy::cast_possible_truncation,
            reason = "the midpoint of two i64 values is itself within i64 range"
        )]
        let mid_avg = ((i128::from(lo) + i128::from(hi)) / 2) as i64;
        mid_avg
    }
}

#[cfg(test)]
mod tests {
    use super::median_i64;

    #[test]
    fn median_i64_handles_odd_and_even() {
        assert_eq!(median_i64(&[3, 1, 2]), 2);
        assert_eq!(median_i64(&[1, 2, 3, 4]), 2); // (2 + 3) / 2, truncated
        assert_eq!(median_i64(&[]), 0);
        assert_eq!(median_i64(&[7]), 7);
        // Midpoint of the full i64 span: computed in i128, so no overflow.
        assert_eq!(median_i64(&[i64::MIN, i64::MAX]), 0);
    }
}
