#![expect(
    clippy::panic_in_result_fn,
    reason = "test functions mix ? propagation with assert! - both are correct in test code"
)]

//! The fixed-width string fields of the `.gtd` format, exercised at the row
//! width of `color_hex` so a whole row fits in a literal.

use geotrace_sdk::{
    AnnotationField, ColorHexField, FixedWidthString, FixedWidthStringError, IconNameField,
    MarkerLabelField, VariantPathField,
};
use rstest::rstest;

#[rstest]
#[case::empty("")]
#[case::shorter_than_the_capacity("#f00")]
#[case::at_the_capacity("#ff8800")]
#[case::multi_byte_ending_at_the_capacity("øre€")]
fn a_value_the_row_holds_round_trips(#[case] value: &str) -> Result<(), FixedWidthStringError> {
    let field = FixedWidthString::<8>::new(value)?;
    assert_eq!(
        FixedWidthString::<8>::decode_row(&field.encode_row())?,
        field
    );
    Ok(())
}

#[rstest]
#[case::empty("", &[0u8; 8])]
#[case::shorter_than_the_capacity("#f00", b"#f00\0\0\0\0")]
#[case::at_the_capacity("#ff8800", b"#ff8800\0")]
fn encode_row_writes_the_content_then_nul_padding(
    #[case] value: &str,
    #[case] expected: &[u8],
) -> Result<(), FixedWidthStringError> {
    assert_eq!(FixedWidthString::<8>::new(value)?.encode_row(), expected);
    Ok(())
}

#[rstest]
#[case::one_byte_over_the_capacity(
    "12345678",
    "\"12345678\" is 8 bytes, past the 7 bytes the field holds"
)]
#[case::a_multi_byte_character_straddling_the_capacity(
    "123456é",
    "\"123456é\" is 8 bytes, past the 7 bytes the field holds"
)]
#[case::a_nul_byte_in_the_value("ab\0cd", "\"ab\\0cd\" has a nul byte at offset 2")]
fn a_value_the_row_cannot_hold_is_refused(#[case] value: &str, #[case] expected: &str) {
    let refusal = FixedWidthString::<8>::new(value)
        .err()
        .map(|error| error.to_string());
    assert_eq!(refusal.as_deref(), Some(expected));
}

#[test]
fn an_all_nul_row_decodes_to_an_empty_value() -> Result<(), FixedWidthStringError> {
    assert!(FixedWidthString::<8>::decode_row(&[0u8; 8])?.is_empty());
    Ok(())
}

#[rstest]
#[case::a_row_of_the_wrong_width(
    b"1234567",
    "a field row of 7 bytes, where the field is 8 bytes wide"
)]
#[case::a_row_without_a_terminator(
    b"12345678",
    "the field row has no nul terminator in its 8 bytes"
)]
#[case::a_byte_past_the_terminator(
    b"ab\0x\0\0\0\0",
    "the field row has a non-nul byte at offset 3, past its nul terminator"
)]
#[case::content_that_is_not_utf8(
    b"a\xff\0\0\0\0\0\0",
    "the field row is not UTF-8: invalid utf-8 sequence of 1 bytes from index 1"
)]
fn a_row_no_writer_could_have_written_is_refused(#[case] row: &[u8], #[case] expected: &str) {
    let refusal = FixedWidthString::<8>::decode_row(row)
        .err()
        .map(|error| error.to_string());
    assert_eq!(refusal.as_deref(), Some(expected));
}

#[test]
fn the_field_capacities_leave_room_for_the_terminator() {
    assert_eq!(VariantPathField::CONTENT_CAPACITY, 255);
    assert_eq!(AnnotationField::CONTENT_CAPACITY, 511);
    assert_eq!(MarkerLabelField::CONTENT_CAPACITY, 255);
    assert_eq!(IconNameField::CONTENT_CAPACITY, 31);
    assert_eq!(ColorHexField::CONTENT_CAPACITY, 7);
}

proptest::proptest! {
    /// Strings long enough that both the accepted and the refused side of the
    /// 31-byte capacity come up.
    #[test]
    fn any_value_that_constructs_round_trips_through_its_row(value in ".{0,40}") {
        if let Ok(field) = FixedWidthString::<32>::new(value) {
            let decoded = FixedWidthString::<32>::decode_row(&field.encode_row()).ok();
            proptest::prop_assert_eq!(decoded.as_ref(), Some(&field));
        }
    }

    /// Reaches both the accepted and the refused side of a row: content
    /// drawn outside ASCII is often not UTF-8. The rows are built the way a
    /// writer builds them, content bytes then nul padding.
    #[test]
    fn any_row_that_decodes_re_encodes_to_itself(
        content in proptest::collection::vec(1u8..=255, 0..8usize)
    ) {
        let mut row = [0u8; 8];
        for (slot, byte) in row.iter_mut().zip(content) {
            *slot = byte;
        }
        if let Ok(field) = FixedWidthString::<8>::decode_row(&row) {
            proptest::prop_assert_eq!(field.encode_row(), row);
        }
    }
}
