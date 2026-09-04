use gt_history_types::{
    ATTR_END_US, ATTR_EVENT_MARKER_COUNT, ATTR_GTD_SIZE_BYTES, ATTR_IDENTITY, ATTR_MARKER_COUNT,
    ATTR_NAV_POINT_COUNT, ATTR_SAT_REPORT_COUNT, ATTR_START_US, CURRENT_SCHEMA_VERSION,
    DatabaseRef, DbError, GTD_META_DEVICE_ATTR, GTD_META_NOTES_ATTR, GTD_META_TITLE_ATTR,
    GTD_META_TRAVEL_MODE_ATTR, HistoryDatabase, LogAttachment, LogAttachmentEntry, LogAttachmentId,
    NavPointTimeRange, ReadOnlyHistoryDatabase, RecordingEntry, RecordingMeta, SCHEMA_VERSION_ATTR,
    StoredRecording, StoredSegmentation, TrackRange, TrackState, identity_from_group_name,
};
use hdf5_pure::{AttrValue, FileBuilder};
use parking_lot::Mutex;
use std::ops::Deref;
use std::path::{Path, PathBuf};

static DB_LOCK: Mutex<()> = Mutex::new(());

pub mod copy;

/// Map an `hdf5-pure` failure onto the [`DbError`] the app acts on.
///
/// Two of them mean the file is in use by a writer. `FileMarkedInUse` is
/// the durable superblock status-flags byte, which a crashed writer leaves set
/// and [`PureDb::clear_write_lock`] repairs. `FileLocked` is an OS lock, which
/// only a live process holds and which no repair can take away.
pub(crate) fn classify_hdf5_error(err: hdf5_pure::Error) -> DbError {
    match err {
        hdf5_pure::Error::FileMarkedInUse(_) => DbError::WriteLocked,
        hdf5_pure::Error::FileLocked(_) => DbError::Busy,
        other => DbError::Backend(other.to_string()),
    }
}

/// The history database with no method that writes to the file, as
/// [`Self::open_existing_read_only`] opens it.
pub struct ReadOnlyPureDb {
    path: PathBuf,
}

impl ReadOnlyPureDb {
    /// Open the database at `path` without writing to it: it is not created
    /// where it is missing, not migrated to the current schema, and not
    /// repaired.
    pub fn open_existing_read_only(path: &Path) -> Result<Self, DbError> {
        let _guard = DB_LOCK.lock();
        Self::schema_version_of(path)?;
        Ok(Self {
            path: path.to_owned(),
        })
    }

    /// The schema version the database at `path` records, reading nothing
    /// else and writing nothing. A database written before the attribute
    /// existed reports 0.
    fn schema_version_of(path: &Path) -> Result<i64, DbError> {
        let file = hdf5_pure::File::open(path).map_err(classify_hdf5_error)?;
        let root = file.root();
        let attrs = root.attrs().map_err(classify_hdf5_error)?;

        let schema_version = attrs
            .get(SCHEMA_VERSION_ATTR)
            .and_then(AttrValue::as_i64)
            .unwrap_or(0);

        if schema_version > CURRENT_SCHEMA_VERSION {
            return Err(DbError::SchemaTooNew {
                found: schema_version,
                supported: CURRENT_SCHEMA_VERSION,
            });
        }
        Ok(schema_version)
    }
}

impl ReadOnlyHistoryDatabase for ReadOnlyPureDb {
    fn path(&self) -> &Path {
        &self.path
    }

    fn load(&self, db_ref: &DatabaseRef) -> Result<StoredRecording, DbError> {
        let _guard = DB_LOCK.lock();
        copy::load_recording(&self.path, &db_ref.identity, &db_ref.group_name).map_err(Into::into)
    }

    fn snap_blob(&self, db_ref: &DatabaseRef) -> Result<Option<Vec<u8>>, DbError> {
        let _guard = DB_LOCK.lock();
        copy::snap_blob(&self.path, &db_ref.identity, &db_ref.group_name).map_err(Into::into)
    }

    fn log_attachments(&self, db_ref: &DatabaseRef) -> Result<Vec<LogAttachmentEntry>, DbError> {
        let _guard = DB_LOCK.lock();
        copy::log_attachments(&self.path, &db_ref.identity, &db_ref.group_name).map_err(Into::into)
    }

    fn list_recordings(&self) -> Result<Vec<RecordingEntry>, DbError> {
        let _guard = DB_LOCK.lock();
        let file = hdf5_pure::File::open(&self.path).map_err(classify_hdf5_error)?;
        let root = file.root();
        let Ok(by_id) = root.group("by_identity") else {
            return Ok(vec![]);
        };
        let mut entries = Vec::new();
        for identity in by_id.groups().map_err(classify_hdf5_error)? {
            let Ok(id_grp) = by_id.group(&identity) else {
                continue;
            };
            let id_attrs = id_grp.attrs().map_err(classify_hdf5_error)?;
            let display_identity = string_attr(&id_attrs, ATTR_IDENTITY)
                .or_else(|| identity_from_group_name(&identity))
                .unwrap_or_else(|| identity.clone());
            for rec_name in id_grp.groups().map_err(classify_hdf5_error)? {
                let Ok(rec_grp) = id_grp.group(&rec_name) else {
                    continue;
                };
                let Ok(attrs) = rec_grp.attrs() else {
                    continue;
                };
                if let Some(meta) = recording_meta_from_attrs(&attrs) {
                    let tracks = copy::read_track_table(&rec_grp);
                    let shelved_tracks = tracks
                        .iter()
                        .filter(|t| t.state == TrackState::Shelved)
                        .count();
                    entries.push(RecordingEntry {
                        db_ref: DatabaseRef {
                            identity: display_identity.clone(),
                            group_name: rec_name,
                        },
                        meta,
                        total_tracks: tracks.len(),
                        shelved_tracks,
                        title: string_attr(&attrs, GTD_META_TITLE_ATTR),
                        device: string_attr(&attrs, GTD_META_DEVICE_ATTR),
                        notes: string_attr(&attrs, GTD_META_NOTES_ATTR),
                        travel_mode: string_attr(&attrs, GTD_META_TRAVEL_MODE_ATTR),
                        channels: copy::read_channel_summaries(&rec_grp),
                        log_attachments: copy::log_attachments_in_attrs(&attrs),
                    });
                }
            }
        }
        entries.sort_unstable_by_key(|e| std::cmp::Reverse(e.meta.stored_start_us()));
        Ok(entries)
    }

    fn is_duplicate(&self, meta: &RecordingMeta) -> Result<bool, DbError> {
        let _guard = DB_LOCK.lock();
        let file = hdf5_pure::File::open(&self.path).map_err(classify_hdf5_error)?;
        let root = file.root();

        let Ok(by_id) = root.group("by_identity") else {
            return Ok(false);
        };

        for identity in by_id.groups().map_err(classify_hdf5_error)? {
            let Ok(id_grp) = by_id.group(&identity) else {
                continue;
            };

            for rec_name in id_grp.groups().map_err(classify_hdf5_error)? {
                if let Ok(rec_grp) = id_grp.group(&rec_name)
                    && let Ok(attrs) = rec_grp.attrs()
                    && matches_attrs(meta, &attrs)
                {
                    return Ok(true);
                }
            }
        }
        Ok(false)
    }
}

/// The history database with the methods that write to the file, as
/// [`HistoryDatabase::open_or_create`] opens it.
pub struct PureDb {
    read_only: ReadOnlyPureDb,
}

impl Deref for PureDb {
    type Target = ReadOnlyPureDb;

    fn deref(&self) -> &Self::Target {
        &self.read_only
    }
}

impl ReadOnlyHistoryDatabase for PureDb {
    fn path(&self) -> &Path {
        self.read_only.path()
    }

    fn load(&self, db_ref: &DatabaseRef) -> Result<StoredRecording, DbError> {
        self.read_only.load(db_ref)
    }

    fn snap_blob(&self, db_ref: &DatabaseRef) -> Result<Option<Vec<u8>>, DbError> {
        self.read_only.snap_blob(db_ref)
    }

    fn log_attachments(&self, db_ref: &DatabaseRef) -> Result<Vec<LogAttachmentEntry>, DbError> {
        self.read_only.log_attachments(db_ref)
    }

    fn list_recordings(&self) -> Result<Vec<RecordingEntry>, DbError> {
        self.read_only.list_recordings()
    }

    fn is_duplicate(&self, meta: &RecordingMeta) -> Result<bool, DbError> {
        self.read_only.is_duplicate(meta)
    }
}

impl HistoryDatabase for PureDb {
    fn open_or_create(path: &Path) -> Result<Self, DbError> {
        let _guard = DB_LOCK.lock();
        if path.exists() {
            Self::validate_existing(path)?;
        } else {
            Self::create_new(path)?;
        }
        Ok(Self {
            read_only: ReadOnlyPureDb {
                path: path.to_owned(),
            },
        })
    }

    fn clear_write_lock(path: &Path) -> Result<(), DbError> {
        let _guard = DB_LOCK.lock();
        hdf5_pure::File::clear_swmr_flag(path).map_err(classify_hdf5_error)?;
        log::debug!(
            "Cleared the write lock on history database at {}",
            path.display()
        );
        Ok(())
    }

    fn insert(
        &mut self,
        identity: &str,
        meta: &RecordingMeta,
        tracks: &[TrackRange],
        settings: StoredSegmentation,
        gtd_bytes: &[u8],
    ) -> Result<DatabaseRef, DbError> {
        let _guard = DB_LOCK.lock();
        copy::insert_recording(&self.path, identity, meta, tracks, settings, gtd_bytes)
            .map_err(Into::into)
    }

    fn replace_recording_in_place(
        &mut self,
        db_ref: &DatabaseRef,
        meta: &RecordingMeta,
        tracks: &[TrackRange],
        settings: StoredSegmentation,
        bytes: &[u8],
    ) -> Result<(), DbError> {
        let _guard = DB_LOCK.lock();
        copy::replace_recording(&self.path, db_ref, meta, tracks, settings, bytes)
            .map_err(Into::into)
    }

    fn set_tracks(
        &mut self,
        db_ref: &DatabaseRef,
        tracks: &[TrackRange],
        settings: StoredSegmentation,
    ) -> Result<(), DbError> {
        let _guard = DB_LOCK.lock();
        copy::set_tracks(
            &self.path,
            &db_ref.identity,
            &db_ref.group_name,
            tracks,
            settings,
        )
        .map_err(Into::into)
    }

    fn set_tracks_shelved(
        &mut self,
        db_ref: &DatabaseRef,
        track_indices: &[usize],
        shelved: bool,
    ) -> Result<(), DbError> {
        let _guard = DB_LOCK.lock();
        copy::set_tracks_shelved(
            &self.path,
            &db_ref.identity,
            &db_ref.group_name,
            track_indices,
            shelved,
        )
        .map_err(Into::into)
    }

    fn set_snap_blob(&mut self, db_ref: &DatabaseRef, blob: &[u8]) -> Result<(), DbError> {
        let _guard = DB_LOCK.lock();
        copy::set_snap_blob(&self.path, &db_ref.identity, &db_ref.group_name, blob)
            .map_err(Into::into)
    }

    fn write_log_attachment_attribute(
        &mut self,
        db_ref: &DatabaseRef,
        id: LogAttachmentId,
        attachment: &LogAttachment,
    ) -> Result<(), DbError> {
        let attribute_json = attachment.to_attribute_json()?;
        let _guard = DB_LOCK.lock();
        copy::set_log_attachment_attribute(
            &self.path,
            &db_ref.identity,
            &db_ref.group_name,
            id,
            &attribute_json,
        )
        .map_err(Into::into)
    }

    fn delete_log_attachment_attribute(
        &mut self,
        db_ref: &DatabaseRef,
        id: LogAttachmentId,
    ) -> Result<(), DbError> {
        let _guard = DB_LOCK.lock();
        copy::delete_log_attachment_attribute(&self.path, &db_ref.identity, &db_ref.group_name, id)
            .map_err(Into::into)
    }

    fn delete_batch(&mut self, refs: &[DatabaseRef]) -> Result<(), DbError> {
        let _guard = DB_LOCK.lock();
        if refs.is_empty() {
            return Ok(());
        }
        copy::delete_batch(&self.path, refs).map_err(Into::into)
    }

    fn rename_identity(&mut self, old: &str, new: &str) -> Result<(), DbError> {
        let _guard = DB_LOCK.lock();
        copy::rename_identity(&self.path, old, new).map_err(Into::into)
    }
}

impl PureDb {
    fn create_new(path: &Path) -> Result<(), DbError> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let mut fb = FileBuilder::new();
        fb.set_attr(SCHEMA_VERSION_ATTR, AttrValue::I64(CURRENT_SCHEMA_VERSION));

        let by_identity = fb.create_group("by_identity");
        fb.add_group(by_identity.finish());

        let meta = fb.create_group("meta");
        fb.add_group(meta.finish());

        fb.write(path).map_err(classify_hdf5_error)?;

        log::info!("Created history database at {}", path.display());
        Ok(())
    }

    fn validate_existing(path: &Path) -> Result<(), DbError> {
        let schema_version = ReadOnlyPureDb::schema_version_of(path)?;

        if schema_version < CURRENT_SCHEMA_VERSION {
            log::info!(
                "Migrating history database from schema_version={schema_version} to {}",
                CURRENT_SCHEMA_VERSION
            );
            Self::migrate(path, schema_version)?;
        }

        log::debug!(
            "Opened history database at {} (schema_version={schema_version})",
            path.display()
        );
        Ok(())
    }

    fn migrate(path: &Path, _from_version: i64) -> Result<(), DbError> {
        copy::write_schema_version(path).map_err(Into::into)
    }
}

/// A string attribute's value, or `None` when it is absent or another type.
pub(crate) fn string_attr(
    attrs: &std::collections::HashMap<String, AttrValue>,
    name: &str,
) -> Option<String> {
    attrs
        .get(name)
        .and_then(AttrValue::as_str)
        .map(str::to_owned)
}

fn recording_meta_from_attrs(
    attrs: &std::collections::HashMap<String, AttrValue>,
) -> Option<RecordingMeta> {
    let start_us = attrs.get(ATTR_START_US).and_then(AttrValue::as_i64)?;
    let end_us = attrs
        .get(ATTR_END_US)
        .and_then(AttrValue::as_i64)
        .unwrap_or(start_us);
    let nav_point_count = attrs
        .get(ATTR_NAV_POINT_COUNT)
        .and_then(AttrValue::as_u64)?;
    let time_range = NavPointTimeRange::from_stored_attributes(nav_point_count, start_us..=end_us);
    let sat_report_count = attrs
        .get(ATTR_SAT_REPORT_COUNT)
        .and_then(AttrValue::as_u64)?;
    let marker_count = attrs.get(ATTR_MARKER_COUNT).and_then(AttrValue::as_u64)?;
    let event_marker_count = attrs
        .get(ATTR_EVENT_MARKER_COUNT)
        .and_then(AttrValue::as_u64)?;
    let gtd_size_bytes = attrs
        .get(ATTR_GTD_SIZE_BYTES)
        .and_then(AttrValue::as_u64)
        .unwrap_or(0);
    Some(RecordingMeta {
        time_range,
        nav_point_count,
        sat_report_count,
        marker_count,
        event_marker_count,
        gtd_size_bytes,
    })
}

fn matches_attrs(
    meta: &RecordingMeta,
    attrs: &std::collections::HashMap<String, AttrValue>,
) -> bool {
    let Some(start_us) = attrs.get(ATTR_START_US).and_then(AttrValue::as_i64) else {
        return false;
    };
    let Some(nav_point_count) = attrs.get(ATTR_NAV_POINT_COUNT).and_then(AttrValue::as_u64) else {
        return false;
    };
    let Some(sat_report_count) = attrs.get(ATTR_SAT_REPORT_COUNT).and_then(AttrValue::as_u64)
    else {
        return false;
    };
    let Some(marker_count) = attrs.get(ATTR_MARKER_COUNT).and_then(AttrValue::as_u64) else {
        return false;
    };
    let Some(event_marker_count) = attrs
        .get(ATTR_EVENT_MARKER_COUNT)
        .and_then(AttrValue::as_u64)
    else {
        return false;
    };
    meta.matches(
        start_us,
        nav_point_count,
        sat_report_count,
        marker_count,
        event_marker_count,
    )
}

/// Extract recording metadata from raw GTD file bytes.
pub fn extract_meta(bytes: &[u8]) -> Result<RecordingMeta, DbError> {
    let file = hdf5_pure::File::from_bytes(bytes.to_vec()).map_err(classify_hdf5_error)?;

    let nav_grp = file.group("nav_points").map_err(classify_hdf5_error)?;
    let nav_shape = nav_grp
        .dataset("time")
        .map_err(classify_hdf5_error)?
        .shape()
        .map_err(classify_hdf5_error)?;
    let nav_point_count = nav_shape.first().copied().unwrap_or(0);

    let time_range = if nav_point_count > 0 {
        let times = nav_grp
            .dataset("time")
            .map_err(classify_hdf5_error)?
            .read_i64()
            .map_err(classify_hdf5_error)?;
        NavPointTimeRange::covering(&times)
    } else {
        None
    };

    let sat_report_count = file
        .group("sat_reports")
        .ok()
        .and_then(|g| g.dataset("nav_point_idx").ok())
        .and_then(|ds| ds.shape().ok())
        .and_then(|s| s.first().copied())
        .unwrap_or(0);

    let marker_count = file
        .group("markers")
        .ok()
        .and_then(|g| g.dataset("time").ok())
        .and_then(|ds| ds.shape().ok())
        .and_then(|s| s.first().copied())
        .unwrap_or(0);

    let event_marker_count = file
        .group("event_markers")
        .ok()
        .and_then(|g| g.dataset("sys_time_us").ok())
        .and_then(|ds| ds.shape().ok())
        .and_then(|s| s.first().copied())
        .unwrap_or(0);

    Ok(RecordingMeta {
        time_range,
        nav_point_count,
        sat_report_count,
        marker_count,
        event_marker_count,
        gtd_size_bytes: bytes.len() as u64,
    })
}
