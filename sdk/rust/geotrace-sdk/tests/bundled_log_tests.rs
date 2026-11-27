#![expect(
    clippy::panic_in_result_fn,
    reason = "test functions mix ? propagation with assert! - both are correct in test code"
)]

use std::fs;
use std::path::Path;

use geotrace_sdk::{
    Angle, BuildError, BundledLog, BundledLogError, Error, NavFile, NavFileBuilder, NavFix,
    NavFixTime,
};
use geotrace_sdk_test_util as test_util;
use hdf5_pure::{AttrValue, File, FileBuilder};
use proptest::prelude::*;
use rstest::rstest;
use tempfile::TempDir;

#[rstest]
#[case::syslog_short("device.log", "May 29 18:48:24 host proc: started\n")]
#[case::syslog_short_single_digit_day("device.log", "Jan  5 09:00:00 host proc: started\n")]
#[case::syslog_short_lower_month("device.log", "sep 12 08:00:00 host proc: started\n")]
#[case::syslog_short_leap_day("device.log", "Feb 29 00:00:00 host proc: leap\n")]
#[case::syslog_short_micro("journal.log", "May 29 18:48:24.123456 host proc: started\n")]
#[case::iso8601_space("gnss.log", "2026-05-29 18:48:24 host proc: started\n")]
#[case::iso8601_t_utc("service.log", "2026-05-29T18:48:24Z host proc: started\n")]
#[case::iso8601_t_offset("service.log", "2026-05-29T18:48:24.500+02:00 host proc: started\n")]
#[case::dot_prefixed_name(".hidden.log", "2026-05-29T18:48:24Z started\n")]
#[case::double_dot_inside_name("a..b.log", "2026-05-29T18:48:24Z started\n")]
#[case::timestamp_on_tenth_non_empty_line(
    "header.log",
    "\nbanner 1\nbanner 2\nbanner 3\nbanner 4\nbanner 5\nbanner 6\nbanner 7\nbanner 8\nbanner 9\n2026-05-29T18:48:24Z tenth line\n"
)]
fn valid_bundled_logs_are_accepted(#[case] name: &str, #[case] text: &str) {
    let log = BundledLog::new(name, text).expect("valid bundled log");
    assert_eq!(log.name(), name);
    assert_eq!(log.text(), text);

    let from_bytes = BundledLog::from_utf8_bytes(name, text.as_bytes()).expect("valid bytes");
    assert_eq!(from_bytes, log);
}

#[rstest]
#[case::empty("")]
#[case::dot(".")]
#[case::double_dot("..")]
#[case::slash("dir/device.log")]
#[case::leading_slash("/device.log")]
#[case::nul_byte("device\0.log")]
fn invalid_bundled_log_names_are_rejected(#[case] name: &str) {
    let err = BundledLog::new(name, "2026-05-29T18:48:24Z started\n")
        .expect_err("invalid name must be rejected");
    assert!(
        matches!(&err, BundledLogError::InvalidName { name: rejected } if rejected == name),
        "unexpected error: {err:#}"
    );
}

#[rstest]
#[case::empty_string("")]
#[case::whitespace_only("   \n\t\r\n  ")]
fn empty_bundled_logs_are_rejected(#[case] text: &str) {
    let err = BundledLog::new("device.log", text).expect_err("empty log must be rejected");
    assert!(
        matches!(&err, BundledLogError::Empty { name } if name == "device.log"),
        "unexpected error: {err:#}"
    );
}

#[rstest]
#[case::plain_text("not a timestamp\nstill not a timestamp\n")]
#[case::invalid_month("Mai 29 18:48:24 host proc: msg\n")]
#[case::invalid_day_short_month("Apr 31 00:00:00 host proc: msg\n")]
#[case::invalid_february_day("Feb 30 00:00:00 host proc: msg\n")]
#[case::invalid_time("Jan  1 24:00:00 host proc: msg\n")]
#[case::timestamp_past_head_window(
    "line 1\nline 2\nline 3\nline 4\nline 5\nline 6\nline 7\nline 8\nline 9\nline 10\n2026-05-29T18:48:24Z eleventh line\n"
)]
fn unrecognised_bundled_log_formats_are_rejected(#[case] text: &str) {
    let err = BundledLog::new("device.log", text).expect_err("unrecognised format must fail");
    assert!(
        matches!(&err, BundledLogError::UnrecognisedFormat { name, .. } if name == "device.log"),
        "unexpected error: {err:#}"
    );
}

#[test]
fn unrecognised_format_truncates_long_first_line_excerpt() {
    let long_line = "x".repeat(200);
    let err = BundledLog::new("device.log", long_line).expect_err("must fail");
    let BundledLogError::UnrecognisedFormat { first_line, .. } = err else {
        panic!("expected UnrecognisedFormat, got {err:?}");
    };
    assert_eq!(first_line.chars().count(), 120);
    assert!(first_line.ends_with('…'));
}

#[test]
fn invalid_utf8_bytes_are_rejected() {
    let err = BundledLog::from_utf8_bytes("device.log", &[0xff, 0xfe, b'\n'])
        .expect_err("invalid UTF-8 must fail");
    assert!(
        matches!(&err, BundledLogError::InvalidUtf8 { name, .. } if name == "device.log"),
        "unexpected error: {err:#}"
    );
}

#[test]
fn from_file_derives_name_and_validates_content() -> Result<(), Box<dyn std::error::Error>> {
    let dir = TempDir::new()?;
    let log_path = dir.path().join("gnss-daemon.log");
    let content = "2026-05-29T18:48:24Z receiver ready\n";
    fs::write(&log_path, content)?;

    let log = BundledLog::from_file(&log_path)?;
    assert_eq!(log.name(), "gnss-daemon.log");
    assert_eq!(log.text(), content);

    let missing_err =
        BundledLog::from_file(dir.path().join("missing.log")).expect_err("missing file must fail");
    assert!(matches!(missing_err, BundledLogError::Io { .. }));

    let no_name_err =
        BundledLog::from_file(Path::new("/")).expect_err("root path has no file name");
    assert!(matches!(
        no_name_err,
        BundledLogError::InvalidFileName { .. }
    ));

    let bad_utf8_path = dir.path().join("binary.log");
    fs::write(&bad_utf8_path, [0x80, 0x81, 0x82])?;
    let utf8_err = BundledLog::from_file(&bad_utf8_path).expect_err("invalid UTF-8 must fail");
    assert!(matches!(utf8_err, BundledLogError::InvalidUtf8 { .. }));
    Ok(())
}

#[test]
fn file_without_bundled_logs_omits_logs_group() -> Result<(), Box<dyn std::error::Error>> {
    let nav_file = NavFileBuilder::new()
        .with_scrubbed_provenance()
        .open()
        .finish()?;
    let bytes = test_util::to_bytes(&nav_file)?;

    let hdf5 = File::from_bytes(bytes.clone())?;
    hdf5.group("logs")
        .expect_err("logs group must be omitted when there are no bundled logs");

    let rt = NavFile::read(bytes.as_slice())?;
    assert!(rt.bundled_logs().is_empty());
    Ok(())
}

#[test]
fn single_and_multiple_bundled_logs_round_trip_with_wire_attributes()
-> Result<(), Box<dyn std::error::Error>> {
    let first = BundledLog::new(
        "zeta.log",
        "May 29 18:48:24.123456 host gnss: fix acquired\nMay 29 18:48:25.000000 host gnss: ok\n",
    )?;
    let repeated_lines: String = (0..300)
        .map(|i| {
            format!(
                "2026-05-29T18:48:{:02}Z line {i} with multi-chunk payload\n",
                i % 60
            )
        })
        .collect();
    let second = BundledLog::new("alpha.log", repeated_lines)?;

    let mut recorder = NavFileBuilder::new().with_scrubbed_provenance().open();
    recorder
        .add_nav_fix(
            NavFix::builder()
                .time(NavFixTime::Receiver(test_util::base()))
                .lat(Angle::degrees(51.5))
                .lon(Angle::degrees(-0.1))
                .build(),
        )
        .add_bundled_log(first.clone())
        .add(second.clone());

    let nav_file = recorder.finish()?;
    assert_eq!(nav_file.bundled_logs(), &[second.clone(), first.clone()]);

    let bytes = test_util::to_bytes(&nav_file)?;
    let hdf5 = File::from_bytes(bytes.clone())?;
    let logs_group = hdf5.group("logs")?;
    for expected in [&second, &first] {
        let dataset = logs_group.group(expected.name())?.dataset("text")?;
        assert_eq!(dataset.shape()?, vec![expected.text().len() as u64]);
        let attrs = dataset.attrs()?;
        assert_eq!(
            attrs.get("encoding").and_then(AttrValue::as_str),
            Some("utf-8")
        );
    }

    let rt = NavFile::read(bytes.as_slice())?;
    assert_eq!(rt.bundled_logs(), &[second, first]);
    assert!(rt.equals_ignoring_build_provenance(&nav_file));
    Ok(())
}

#[test]
fn duplicate_bundled_log_names_are_rejected_on_finish() -> Result<(), Box<dyn std::error::Error>> {
    let mut recorder = NavFileBuilder::new().open();
    recorder
        .add_bundled_log(BundledLog::new(
            "device.log",
            "2026-05-29T18:48:24Z first\n",
        )?)
        .add_bundled_log(BundledLog::new(
            "device.log",
            "2026-05-29T18:48:25Z second\n",
        )?);

    let err = recorder
        .finish()
        .expect_err("duplicate bundled log names must be rejected");
    assert!(
        matches!(&err, BuildError::DuplicateBundledLogName { name } if name == "device.log"),
        "unexpected error: {err:#}"
    );
    Ok(())
}

#[test]
fn existing_gtd_fixture_without_logs_group_reads_with_empty_bundled_logs()
-> Result<(), Box<dyn std::error::Error>> {
    let gold = NavFile::open(test_util::fixture_path("gold_dataset/gold.gtd"))?;
    assert!(gold.bundled_logs().is_empty());
    Ok(())
}

fn minimal_v2_builder() -> FileBuilder {
    let mut fb = test_util::file_with_an_empty_nav_points_group();
    fb.set_attr("geotrace_version", AttrValue::String("2".into()));
    fb
}

#[test]
fn logs_entry_that_is_a_dataset_instead_of_a_group_fails_the_read() {
    let mut fb = minimal_v2_builder();
    fb.create_dataset("logs")
        .with_u8_data(b"not a group")
        .with_shape(&[11]);
    let bytes = fb.finish().expect("build");

    let err = NavFile::read(bytes.as_slice()).expect_err("non-group logs must fail");
    assert!(matches!(err, Error::Hdf5(_)), "unexpected error: {err:#}");
}

#[test]
fn bundled_log_group_missing_text_dataset_fails_the_read() {
    let mut fb = minimal_v2_builder();
    let mut logs = fb.create_group("logs");
    let child = logs.create_group("device.log");
    logs.add_group(child.finish());
    fb.add_group(logs.finish());
    let bytes = fb.finish().expect("build");

    let err = NavFile::read(bytes.as_slice()).expect_err("missing text dataset must fail");
    assert!(
        matches!(&err, Error::MissingBundledLogDataset { name } if name == "device.log"),
        "unexpected error: {err:#}"
    );
}

#[test]
fn bundled_log_text_dataset_with_2d_shape_fails_the_read() {
    let mut fb = minimal_v2_builder();
    let mut logs = fb.create_group("logs");
    let mut child = logs.create_group("device.log");
    let payload = b"2026-05-29T18:48:24Z ok\n";
    child
        .create_dataset("text")
        .with_u8_data(payload)
        .with_shape(&[2, (payload.len() / 2) as u64]);
    logs.add_group(child.finish());
    fb.add_group(logs.finish());
    let bytes = fb.finish().expect("build");

    let err = NavFile::read(bytes.as_slice()).expect_err("2-D text dataset must fail");
    assert!(
        matches!(
            &err,
            Error::InvalidBundledLogShape { name, actual }
                if name == "device.log" && actual == &vec![2, (payload.len() / 2) as u64]
        ),
        "unexpected error: {err:#}"
    );
}

#[test]
fn bundled_log_text_dataset_with_invalid_utf8_fails_the_read() {
    let mut fb = minimal_v2_builder();
    let mut logs = fb.create_group("logs");
    let mut child = logs.create_group("device.log");
    child
        .create_dataset("text")
        .with_u8_data(&[0xff, 0xfe, 0xfd])
        .with_shape(&[3]);
    logs.add_group(child.finish());
    fb.add_group(logs.finish());
    let bytes = fb.finish().expect("build");

    let err = NavFile::read(bytes.as_slice()).expect_err("invalid UTF-8 must fail");
    assert!(
        matches!(
            &err,
            Error::InvalidBundledLog {
                source: BundledLogError::InvalidUtf8 { name, .. }
            } if name == "device.log"
        ),
        "unexpected error: {err:#}"
    );
}

fn jenkins_lookup3(data: &[u8]) -> u32 {
    let mut a = 0xdead_beef_u32.wrapping_add(u32::try_from(data.len()).unwrap_or(0));
    let mut b = a;
    let mut c = a;

    let (chunks, rem) = data.as_chunks::<12>();
    for &[b0, b1, b2, b3, b4, b5, b6, b7, b8, b9, b10, b11] in chunks {
        let w0 = u32::from_le_bytes([b0, b1, b2, b3]);
        let w1 = u32::from_le_bytes([b4, b5, b6, b7]);
        let w2 = u32::from_le_bytes([b8, b9, b10, b11]);
        a = a.wrapping_add(w0);
        b = b.wrapping_add(w1);
        c = c.wrapping_add(w2);

        a = a.wrapping_sub(c) ^ c.rotate_left(4);
        c = c.wrapping_add(b);
        b = b.wrapping_sub(a) ^ a.rotate_left(6);
        a = a.wrapping_add(c);
        c = c.wrapping_sub(b) ^ b.rotate_left(8);
        b = b.wrapping_add(a);
        a = a.wrapping_sub(c) ^ c.rotate_left(16);
        c = c.wrapping_add(b);
        b = b.wrapping_sub(a) ^ a.rotate_left(19);
        a = a.wrapping_add(c);
        c = c.wrapping_sub(b) ^ b.rotate_left(4);
        b = b.wrapping_add(a);
    }

    if rem.is_empty() {
        return c;
    }
    let mut tail = [0_u8; 12];
    if let Some(dst) = tail.get_mut(..rem.len()) {
        dst.copy_from_slice(rem);
    }
    let [b0, b1, b2, b3, b4, b5, b6, b7, b8, b9, b10, b11] = tail;
    a = a.wrapping_add(u32::from_le_bytes([b0, b1, b2, b3]));
    b = b.wrapping_add(u32::from_le_bytes([b4, b5, b6, b7]));
    c = c.wrapping_add(u32::from_le_bytes([b8, b9, b10, b11]));

    c = (c ^ b).wrapping_sub(b.rotate_left(14));
    a = (a ^ c).wrapping_sub(c.rotate_left(11));
    b = (b ^ a).wrapping_sub(a.rotate_left(25));
    c = (c ^ b).wrapping_sub(b.rotate_left(16));
    a = (a ^ c).wrapping_sub(c.rotate_left(4));
    b = (b ^ a).wrapping_sub(a.rotate_left(14));
    (c ^ b).wrapping_sub(b.rotate_left(24))
}

fn patch_ohdr_checksum(bytes: &mut [u8], ohdr_start: usize) -> bool {
    if bytes.get(ohdr_start..ohdr_start + 4) != Some(b"OHDR") {
        return false;
    }
    let Some(&flags) = bytes.get(ohdr_start + 5) else {
        return false;
    };
    let size_width = 1_usize << (flags & 0x03);
    let mut cursor = ohdr_start + 6;
    if flags & 0x20 != 0 {
        cursor += 16;
    }
    if flags & 0x10 != 0 {
        cursor += 4;
    }
    let Some(size_bytes) = bytes.get(cursor..cursor + size_width) else {
        return false;
    };
    let mut padded = [0_u8; 8];
    if let Some(dst) = padded.get_mut(..size_width) {
        dst.copy_from_slice(size_bytes);
    }
    let chunk0_size = usize::try_from(u64::from_le_bytes(padded)).unwrap_or(usize::MAX);
    let checksum_pos = cursor + size_width + chunk0_size;
    let Some(header_slice) = bytes.get(ohdr_start..checksum_pos) else {
        return false;
    };
    let checksum = jenkins_lookup3(header_slice).to_le_bytes();
    let Some(slot) = bytes.get_mut(checksum_pos..checksum_pos + 4) else {
        return false;
    };
    slot.copy_from_slice(&checksum);
    true
}

#[test]
fn bundled_log_dataset_declaring_oversized_shape_is_rejected_before_allocation() {
    let mut fb = minimal_v2_builder();
    let mut logs = fb.create_group("logs");
    let mut child = logs.create_group("device.log");
    let sentinel_len: usize = 37;
    let payload = b"2026-05-29T18:48:24Z sentinel payload";
    assert_eq!(payload.len(), sentinel_len);
    child
        .create_dataset("text")
        .with_u8_data(payload)
        .with_shape(&[sentinel_len as u64]);
    logs.add_group(child.finish());
    fb.add_group(logs.finish());
    let bytes = fb.finish().expect("build");

    let needle = (sentinel_len as u64).to_le_bytes();
    let huge = 1_000_000_000_u64.to_le_bytes();
    let ohdr_positions: Vec<usize> = bytes
        .windows(4)
        .enumerate()
        .filter_map(|(pos, window)| (window == b"OHDR").then_some(pos))
        .collect();

    let mut rejected = false;
    for pos in 0..=bytes.len().saturating_sub(8) {
        if bytes.get(pos..pos + 8) != Some(needle.as_slice()) {
            continue;
        }
        let Some(&ohdr_start) = ohdr_positions.iter().rev().find(|&&start| start < pos) else {
            continue;
        };
        let mut candidate = bytes.clone();
        if let Some(slot) = candidate.get_mut(pos..pos + 8) {
            slot.copy_from_slice(&huge);
        }
        if !patch_ohdr_checksum(&mut candidate, ohdr_start) {
            continue;
        }
        if matches!(
            NavFile::read(candidate.as_slice()),
            Err(Error::DatasetSizePastFileLength { path, declared_bytes, .. })
                if path == "logs/device.log/text" && declared_bytes == 1_000_000_000
        ) {
            rejected = true;
            break;
        }
    }
    assert!(
        rejected,
        "expected patched logs/device.log/text dataspace dimension to fail with DatasetSizePastFileLength"
    );
}

proptest! {
    #[test]
    fn arbitrary_bytes_in_logs_text_never_panic_and_match_constructor_validation(
        bytes in prop::collection::vec(any::<u8>(), 0..256),
    ) {
        let mut fb = minimal_v2_builder();
        let mut logs = fb.create_group("logs");
        let mut child = logs.create_group("fuzz.log");
        child
            .create_dataset("text")
            .with_u8_data(&bytes)
            .with_shape(&[bytes.len() as u64]);
        logs.add_group(child.finish());
        fb.add_group(logs.finish());
        let file_bytes = fb.finish().expect("build");

        let read_result = NavFile::read(file_bytes.as_slice());
        let ctor_result = BundledLog::from_utf8_bytes("fuzz.log", &bytes);
        match (read_result, ctor_result) {
            (Ok(nav_file), Ok(expected)) => {
                prop_assert_eq!(nav_file.bundled_logs(), &[expected]);
            }
            (Err(Error::InvalidBundledLog { .. }), Err(_)) => {}
            (read, ctor) => {
                return Err(TestCaseError::fail(format!(
                    "reader ({read:?}) and constructor ({ctor:?}) disagreed"
                )));
            }
        }
    }
}
