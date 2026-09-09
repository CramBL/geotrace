use std::num::NonZeroUsize;

use rstest::rstest;

use crate::EM_DASH;

#[rstest]
#[case::shorter_than_the_limit("Alpha", 9, "Alpha")]
#[case::exactly_the_limit("Alpha", 5, "Alpha")]
#[case::one_character_over("Alphas", 5, "Alpha…")]
#[case::multi_byte_characters_count_as_one_each("ærøskøbing", 4, "ærøs…")]
#[case::nothing_to_cut("", 3, "")]
fn a_value_is_cut_to_a_character_count_with_an_ellipsis(
    #[case] value: &str,
    #[case] max_chars: usize,
    #[case] expected: &str,
) {
    let max_chars = NonZeroUsize::new(max_chars).unwrap_or(NonZeroUsize::MIN);
    assert_eq!(crate::truncate_with_ellipsis(value, max_chars), expected);
}

#[rstest]
#[case::nothing(0, "0")]
#[case::two_digits(12, "12")]
#[case::the_last_count_below_a_separator(999, "999")]
#[case::a_thousand(1_000, "1,000")]
#[case::four_digits(8_940, "8,940")]
#[case::two_separators(1_000_000, "1,000,000")]
fn a_count_is_grouped_in_thousands(#[case] count: usize, #[case] expected: &str) {
    assert_eq!(crate::format_count(count), expected);
}

#[rstest]
#[case::nothing(0, EM_DASH)]
#[case::bytes(512, "512 B")]
#[case::exactly_one_kib(1_024, "1.0 KB")]
#[case::kilobytes(1_536, "1.5 KB")]
#[case::exactly_one_mib(1_048_576, "1.0 MB")]
#[case::a_day_of_interference(82_944, "81.0 KB")]
#[case::a_full_interference_archive(132_710_400, "126.6 MB")]
#[case::just_under_a_gib(1_073_741_823, "1024.0 MB")]
#[case::exactly_one_gib(1_073_741_824, "1.0 GB")]
#[case::the_default_storage_limit(10_737_418_240, "10.0 GB")]
#[case::terabytes(2_199_023_255_552, "2.0 TB")]
#[case::past_the_largest_unit(u64::MAX, "16777216.0 TB")]
fn a_byte_count_reads_in_binary_units(#[case] bytes: u64, #[case] expected: &str) {
    assert_eq!(crate::format_bytes(bytes), expected);
}

#[rstest]
#[case::one(1, "recording")]
#[case::none(0, "recordings")]
#[case::several(2, "recordings")]
fn only_a_count_of_one_takes_the_singular_word(#[case] count: usize, #[case] expected: &str) {
    assert_eq!(crate::pluralize(count, "recording", "recordings"), expected);
}
