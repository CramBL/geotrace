//! Robust statistics shared by the clock analyses.

/// Median of `values`, averaging the two central elements for an even count.
pub fn median_i64(values: &[i64]) -> Option<i64> {
    if values.is_empty() {
        return None;
    }
    let mut sorted = values.to_vec();
    sorted.sort_unstable();
    let mid = sorted.len() / 2;
    let hi = sorted.get(mid).copied()?;
    if sorted.len() % 2 == 1 {
        Some(hi)
    } else {
        let lo = mid.checked_sub(1).and_then(|i| sorted.get(i).copied())?;
        // Overflow-safe midpoint: `lo + hi` can exceed i64 for extreme inputs.
        #[expect(
            clippy::cast_possible_truncation,
            reason = "the midpoint of two i64 values is itself within i64 range"
        )]
        let mid_avg = ((i128::from(lo) + i128::from(hi)) / 2) as i64;
        Some(mid_avg)
    }
}

#[cfg(test)]
mod tests {
    use super::median_i64;

    #[test]
    fn median_i64_handles_odd_and_even() {
        assert_eq!(median_i64(&[3, 1, 2]), Some(2));
        assert_eq!(median_i64(&[1, 2, 3, 4]), Some(2)); // (2 + 3) / 2, truncated
        assert_eq!(median_i64(&[]), None);
        assert_eq!(median_i64(&[7]), Some(7));
        // Midpoint of the full i64 span: computed in i128, so no overflow.
        assert_eq!(median_i64(&[i64::MIN, i64::MAX]), Some(0));
    }
}
