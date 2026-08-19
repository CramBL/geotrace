//! The compress decoder against uncurated input.
//!
//! A mirror's file arrives over the network, so the decoder reads bytes
//! nobody in this workspace wrote. Every property asserts the same thing: no
//! input panics, what comes back stays inside the declared output limit, and
//! a stream cut short decodes to what it did hold.

mod support;

use proptest::prelude::any;
use proptest::test_runner::TestCaseError;

use gt_ionex::unix_compress::{self, MAX_DECOMPRESSED_BYTES};

/// The stream the truncation property cuts, small enough to decode a few
/// hundred times.
fn head_stream() -> Result<Vec<u8>, String> {
    support::compressed_fixture("JPLG0920.24I.head.12bit.Z")
}

fn check_within_the_limit(decompressed: &[u8]) -> Result<(), TestCaseError> {
    if decompressed.len() > MAX_DECOMPRESSED_BYTES {
        return Err(TestCaseError::fail(format!(
            "{} bytes is past the declared limit",
            decompressed.len()
        )));
    }
    Ok(())
}

proptest::proptest! {
    /// Any input at all. Most cases are refused for having no magic.
    #[test]
    fn arbitrary_input_is_decoded_or_refused(
        compressed in proptest::collection::vec(any::<u8>(), 0..2048)
    ) {
        if let Ok(decompressed) = unix_compress::decompress(&compressed) {
            check_within_the_limit(&decompressed)?;
        }
    }

    /// Arbitrary codes behind a header the decoder accepts, which reaches the
    /// table and the code widths far more often than arbitrary bytes do.
    #[test]
    fn an_arbitrary_payload_behind_a_header_is_decoded_or_refused(
        flags in 9_u8..=16,
        block_mode in any::<bool>(),
        payload in proptest::collection::vec(any::<u8>(), 0..2048),
    ) {
        let mut compressed = vec![0x1f, 0x9d, if block_mode { flags | 0x80 } else { flags }];
        compressed.extend(payload);
        if let Ok(decompressed) = unix_compress::decompress(&compressed) {
            check_within_the_limit(&decompressed)?;
        }
    }
}

// Every case decodes a whole stream, so this property runs fewer of them than
// the default.
proptest::proptest! {
    #![proptest_config(proptest::test_runner::Config {
        cases: 64,
        ..proptest::test_runner::Config::default()
    })]

    /// A dropped connection or a half-written cache entry. The codes a
    /// truncated stream still holds sit where they did, so what it decodes to
    /// is the start of what the whole stream does.
    #[test]
    fn a_truncated_stream_decodes_to_the_start_of_the_whole_one(cut in 0_usize..19_000) {
        let stream = head_stream().map_err(TestCaseError::fail)?;
        let whole = unix_compress::decompress(&stream)
            .map_err(|err| TestCaseError::fail(err.to_string()))?;
        let Some(truncated) = stream.get(..cut) else {
            return Ok(());
        };
        let decompressed = unix_compress::decompress(truncated)
            .map_err(|err| TestCaseError::fail(err.to_string()))?;
        check_within_the_limit(&decompressed)?;
        if !whole.starts_with(&decompressed) {
            return Err(TestCaseError::fail(format!(
                "{} bytes from a stream cut at {cut} are not the start of the whole one",
                decompressed.len()
            )));
        }
    }
}
