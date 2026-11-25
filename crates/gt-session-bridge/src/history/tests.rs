use gt_history_types::RecordingMeta;

use super::*;
use crate::framing::{Frame, MAX_REPLY_BYTES};
use crate::protocol::Reply;

#[test]
fn a_cursor_continues_only_the_same_snapshot_and_dates() {
    let day = NaiveDate::from_ymd_opt(1970, 1, 2).unwrap();
    let dates = Some(HistoryDates {
        from: Some(day),
        through: Some(day),
    });
    let mut entries = vec![
        self::entry("first"),
        self::entry("empty"),
        self::entry("last"),
    ];
    entries[0].meta.time_range = NavPointTimeRange::covering(&[UTC_DAY_MICROS - 1, UTC_DAY_MICROS]);
    entries[2].meta.time_range = NavPointTimeRange::covering(&[2 * UTC_DAY_MICROS - 1]);
    let entries = HistoryEntries::new(entries);
    let mut listing = HistoryListing {
        database: Path::new("history.hdf5"),
        entries: &entries,
        revision: 0,
        session: Uuid::from_u128(1),
    };
    let mut request = ListHistoryRecordings {
        cursor: None,
        dates,
        limit: NonZeroUsize::new(1),
    };
    let first = listing.page(request).unwrap();
    assert_eq!(first.recordings[0].identity, "first");
    request.cursor = first.next_cursor;
    assert!(request.cursor.is_some());
    let last = listing.page(request).unwrap();
    assert_eq!(last.recordings[0].identity, "last");
    assert_eq!(last.next_cursor, None);

    let inverted = ListHistoryRecordings {
        cursor: None,
        dates: Some(HistoryDates {
            from: day.succ_opt(),
            through: Some(day),
        }),
        ..request
    };
    assert_eq!(
        listing.page(inverted),
        Err(ErrorCode::InvalidHistoryDateRange)
    );

    assert_eq!(
        listing.page(ListHistoryRecordings {
            dates: None,
            ..request
        }),
        Err(ErrorCode::InvalidHistoryCursor)
    );
    listing.session = Uuid::from_u128(2);
    assert_eq!(listing.page(request), Err(ErrorCode::InvalidHistoryCursor));
    listing.session = Uuid::from_u128(1);
    listing.revision = 1;
    assert_eq!(listing.page(request), Err(ErrorCode::InvalidHistoryCursor));
}

#[test]
fn a_page_with_large_utf8_metadata_fits_the_wire_limit() {
    let preview = "\0".repeat(MAX_METADATA_BYTES - 1);
    let text = format!("{preview}é");
    let entries = HistoryEntries::new(
        (0..=MAX_PER_PAGE)
            .map(|index| {
                let mut entry = self::entry(&text);
                entry.title = Some(text.clone());
                entry.db_ref.group_name = index.to_string();
                entry
            })
            .collect(),
    );
    let page = HistoryListing {
        database: Path::new("history.hdf5"),
        entries: &entries,
        session: Uuid::nil(),
        revision: 0,
    }
    .page(ListHistoryRecordings {
        cursor: None,
        dates: None,
        limit: Some(NonZeroUsize::MAX),
    })
    .unwrap();
    assert_eq!(page.recordings.len(), MAX_PER_PAGE);
    assert!(page.next_cursor.is_some());
    assert!(page.recordings.iter().all(|recording| {
        recording.metadata_truncated
            && recording.identity == preview
            && recording.title.as_deref() == Some(preview.as_str())
    }));
    Frame::encode(&Reply::HistoryRecordings { page }, MAX_REPLY_BYTES)
        .expect("the largest page fits in one reply");
}

#[test]
fn current_group_ids_survive_identity_renames_and_legacy_groups_use_identity() {
    let database = Path::new("data/history.hdf5");
    let entries = HistoryEntries::new(Vec::new());
    let mut reference = self::entry("first").db_ref;
    let legacy = HistoryRecordingId::of(database, &entries, &reference);
    let legacy_entries = HistoryEntries::new(vec![self::entry("first")]);
    let listing = HistoryListing {
        database,
        entries: &legacy_entries,
        revision: 0,
        session: Uuid::from_u128(1),
    };
    assert_eq!(
        listing.select_for_open(legacy).err(),
        Some(ErrorCode::HistorySelectionRequiresDesktop)
    );
    reference.identity = "second".to_owned();
    assert_ne!(
        HistoryRecordingId::of(database, &entries, &reference),
        legacy
    );

    reference.group_name = format!("1970-01-02T00:00:00Z_{}", Uuid::new_v4());
    let current = HistoryRecordingId::of(database, &entries, &reference);
    reference.identity = "first".to_owned();
    assert_eq!(
        HistoryRecordingId::of(database, &entries, &reference),
        current
    );
    assert_ne!(
        HistoryRecordingId::of(Path::new("backup/history.hdf5"), &entries, &reference),
        current
    );
}

#[test]
fn repeated_uuid_groups_have_distinct_ids_across_pages_and_filters() {
    let mut first = self::entry("first");
    first.db_ref.group_name = format!("1970-01-02T00:00:00Z_{}", Uuid::new_v4());
    first.meta.time_range = NavPointTimeRange::covering(&[UTC_DAY_MICROS]);
    let mut second = self::entry("second");
    second.db_ref.group_name = first.db_ref.group_name.clone();
    second.meta.time_range = NavPointTimeRange::covering(&[0]);
    let entries = HistoryEntries::new(vec![first, second]);
    let listing = HistoryListing {
        database: Path::new("data/history.hdf5"),
        entries: &entries,
        revision: 0,
        session: Uuid::nil(),
    };
    let request = ListHistoryRecordings {
        cursor: None,
        dates: None,
        limit: Some(NonZeroUsize::MIN),
    };
    let first_page = listing.page(request).unwrap();
    let second_page = listing
        .page(ListHistoryRecordings {
            cursor: first_page.next_cursor,
            ..request
        })
        .unwrap();
    assert_eq!(first_page.recordings[0].identity, "first");
    assert_eq!(second_page.recordings[0].identity, "second");
    assert_ne!(first_page.recordings[0].id, second_page.recordings[0].id);

    let day = NaiveDate::from_ymd_opt(1970, 1, 2).unwrap();
    let filtered = listing
        .page(ListHistoryRecordings {
            dates: Some(HistoryDates {
                from: Some(day),
                through: Some(day),
            }),
            ..request
        })
        .unwrap();
    assert_eq!(filtered.recordings, first_page.recordings);
    assert_eq!(filtered.next_cursor, None);
}

fn entry(identity: &str) -> RecordingEntry {
    RecordingEntry {
        db_ref: DatabaseRef {
            identity: identity.to_owned(),
            group_name: "legacy".to_owned(),
        },
        meta: RecordingMeta::default(),
        total_tracks: 0,
        shelved_tracks: 0,
        title: None,
        device: None,
        notes: None,
        travel_mode: None,
        channels: Vec::new(),
        log_attachments: Vec::new(),
    }
}
