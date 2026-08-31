//! The compress decoder against the streams
//! `just qa::generate-unix-compress-fixtures` writes.
//!
//! Every one of them was checked against the system `gzip` when it was
//! generated, so decoding them here holds the decoder to what an independent
//! implementation reads the same bytes as.

mod support;

use rstest::rstest;

use gt_ionex::unix_compress::{self, UnixCompressError};

/// A stream of the whole capture, and the two written from its first
/// [`support::COMPRESSED_HEAD_BYTES`] bytes.
#[rstest]
#[case::block_mode_at_sixteen_bits("JPLG0920.24I.Z", None)]
#[case::a_table_that_fills_at_twelve_bits(
    "JPLG0920.24I.head.12bit.Z",
    Some(support::COMPRESSED_HEAD_BYTES)
)]
#[case::without_block_mode("JPLG0920.24I.head.no-block.Z", Some(support::COMPRESSED_HEAD_BYTES))]
fn a_generated_stream_decodes_to_the_bytes_it_was_written_from(
    #[case] fixture: &str,
    #[case] head_bytes: Option<usize>,
) {
    let compressed = support::compressed_fixture(fixture).expect("the fixture is generated");
    let capture = support::compressed_capture_bytes().expect("the capture is committed");
    let expected = match head_bytes {
        Some(bytes) => capture
            .get(..bytes)
            .expect("the capture is longer than the head"),
        None => capture.as_slice(),
    };

    let decompressed = unix_compress::decompress(&compressed).expect("the stream decodes");

    assert_eq!(decompressed.len(), expected.len());
    assert!(decompressed == expected, "the decoded bytes differ");
}

/// The decoded capture is the file the parser reads, which is what routing a
/// `.Z` mirror file through the decoder amounts to.
#[test]
fn the_decoded_capture_parses_as_the_file_it_came_from() {
    let compressed =
        support::compressed_fixture("JPLG0920.24I.Z").expect("the fixture is generated");
    let decompressed = unix_compress::decompress(&compressed).expect("the stream decodes");
    let text = String::from_utf8(decompressed).expect("the capture is text");

    let maps = gt_ionex::parse::global_ionosphere_maps(&text).expect("the decoded file parses");

    assert_eq!(maps.maps().len(), 13);
}

/// A crafted stream expands far beyond its own length, so the decode stops
/// instead of growing the output until the allocator fails.
#[test]
fn a_stream_past_the_output_limit_is_rejected() {
    let compressed =
        support::compressed_fixture("past-output-limit.Z").expect("the fixture is generated");

    assert_eq!(
        unix_compress::decompress(&compressed),
        Err(UnixCompressError::TooLarge)
    );
}
