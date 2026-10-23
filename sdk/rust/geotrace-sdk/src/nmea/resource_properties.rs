use std::time::Duration;

use chrono::{DateTime, TimeDelta, Utc};
use proptest::prelude::*;
use strum::IntoEnumIterator as _;

use super::{NmeaDiagnosticKind, NmeaIngestor, SentenceType};

proptest! {
    #[test]
    fn p8_incomplete_and_malformed_streams_bound_every_retained_collection(
        capacity in 1_usize..=8,
        iterations in 256_usize..=2_048,
        choices in prop::collection::vec(0_u8..=7, 1..=64),
    ) {
        let host = "2026-10-23T12:00:00Z".parse::<DateTime<Utc>>().expect("test timestamp");
        let mut ingestor = NmeaIngestor::builder()
            .max_pending_epochs(capacity)
            .max_pending_reports(capacity)
            .max_gsv_groups(capacity)
            .max_gsv_fragments(capacity)
            .max_buffered_lines(capacity)
            .diagnostic_samples(capacity)
            .max_line_bytes(160)
            .sentence_period(SentenceType::RMC, Duration::from_secs(1))
            .build().expect("configuration");
        for index in 0..iterations {
            let timestamp = host + TimeDelta::seconds(index as i64);
            let body = match choices[index % choices.len()] {
                0 => format!("GPGGA,{},5500.0,N,01200.0,E,4,08,0.7,0,M,0,M,,", timestamp.format("%H%M%S")),
                1 => "GPGSV,2,1,02,01,30,100,40".to_owned(),
                2 => "GAGSV,2,1,02,01,30,100,40".to_owned(),
                3 => "GPGSA,A,3,01,02,,,,,,,,,,,1.2,0.7,0.9".to_owned(),
                4 => "GPVTG,90.0,T,,M,1.0,N,1.852,K,A".to_owned(),
                5 => format!("GPRMC,{},A,5500.0,N,01200.0,E,1.0,90.0,{},,,A", timestamp.format("%H%M%S"), timestamp.format("%d%m%y")),
                6 => "PAIR062,0,1".to_owned(),
                _ => "invalid".repeat(1_000),
            };
            let checksum = body.bytes().fold(0_u8, |checksum, byte| checksum ^ byte);
            let line = format!("${body}*{checksum:02X}");
            let records = ingestor.push_line_at(&line, timestamp);
            prop_assert!(records.len() <= capacity * (capacity + 4));
            prop_assert!(ingestor.epochs.len() <= capacity);
            prop_assert!(ingestor.pending.len() <= capacity);
            prop_assert!(ingestor.gsv_groups.len() <= capacity);
            prop_assert!(ingestor.completed_gsv.len() <= capacity);
            prop_assert!(ingestor.capture_history.recent.len() <= capacity);
            prop_assert!(ingestor.finalized_receiver_epochs.len() <= capacity);
            prop_assert!(ingestor.diagnostics.samples().len() <= capacity);
            prop_assert!(ingestor.diagnostics.counts().len() <= NmeaDiagnosticKind::iter().count());
            for group in ingestor.gsv_groups.iter().chain(&ingestor.completed_gsv) {
                prop_assert!(group.fragments.len() <= capacity);
                prop_assert!(group.fragment_times.len() <= capacity);
                for fragment in group.fragments.values() {
                    prop_assert!(fragment.sats_info.len() <= 4);
                }
            }
            ingestor.schedules.assert_buffer_limits();
            ingestor.receiver_clock.assert_buffer_limits();
        }
        let completion = ingestor.finish();
        prop_assert!(completion.diagnostics.samples().len() <= capacity);
    }
}
