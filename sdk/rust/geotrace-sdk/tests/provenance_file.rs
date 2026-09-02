use rstest::rstest;

include!("../build_script/provenance_file.rs");

const COMMIT: &str = "0123456789abcdef0123456789abcdef01234567";

#[rstest]
#[case::trailing_newline("0123456789abcdef0123456789abcdef01234567\n2026-02-01T16:00:00+01:00\n")]
#[case::no_trailing_newline("0123456789abcdef0123456789abcdef01234567\n2026-02-01T16:00:00+01:00")]
fn a_well_formed_file_gives_the_commit_and_its_time_in_utc(#[case] contents: &str) {
    assert_eq!(
        parse_provenance_file(contents),
        Ok((COMMIT, "2026-02-01T15:00:00Z".to_owned()))
    );
}

#[rstest]
#[case::empty("")]
#[case::abbreviated_hash("0123456\n2026-02-01T15:00:00Z\n")]
#[case::hash_is_not_hex("zzz3456789abcdef0123456789abcdef01234567\n2026-02-01T15:00:00Z\n")]
#[case::second_line_missing("0123456789abcdef0123456789abcdef01234567\n")]
#[case::time_is_not_rfc3339("0123456789abcdef0123456789abcdef01234567\nlast tuesday\n")]
fn a_malformed_file_is_rejected(#[case] contents: &str) {
    parse_provenance_file(contents).unwrap_err();
}
