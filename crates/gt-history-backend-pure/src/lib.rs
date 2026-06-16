use gt_types::DatabaseRef;
use gt_types::history::{
    ATTR_END_US, ATTR_EVENT_MARKER_COUNT, ATTR_GTD_SIZE_BYTES, ATTR_HIDDEN, ATTR_MARKER_COUNT,
    ATTR_NAV_POINT_COUNT, ATTR_SAT_REPORT_COUNT, ATTR_START_US, CURRENT_SCHEMA_VERSION, DbError,
    HistoryDatabase, RecordingEntry, RecordingMeta, SCHEMA_VERSION_ATTR,
};
use hdf5_pure::{AttrValue, FileBuilder};
use parking_lot::Mutex;
use std::path::{Path, PathBuf};

static DB_LOCK: Mutex<()> = Mutex::new(());

pub mod copy;

pub struct PureDb {
    path: PathBuf,
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
            path: path.to_owned(),
        })
    }

    fn insert(
        &mut self,
        identity: &str,
        meta: &RecordingMeta,
        gtd_bytes: &[u8],
    ) -> Result<DatabaseRef, DbError> {
        let _guard = DB_LOCK.lock();
        copy::insert_recording(&self.path, identity, meta, gtd_bytes)
            .map(|rec_name| DatabaseRef {
                identity: identity.to_owned(),
                group_name: rec_name,
            })
            .map_err(Into::into)
    }

    fn delete(&mut self, db_ref: &DatabaseRef) -> Result<(), DbError> {
        let _guard = DB_LOCK.lock();
        copy::delete_recording(&self.path, &db_ref.identity, &db_ref.group_name).map_err(Into::into)
    }

    fn set_hidden(&mut self, refs: &[DatabaseRef], hidden: bool) -> Result<(), DbError> {
        let _guard = DB_LOCK.lock();
        if refs.is_empty() {
            return Ok(());
        }
        copy::set_hidden(&self.path, refs, hidden).map_err(Into::into)
    }

    fn load_bytes(&self, db_ref: &DatabaseRef) -> Result<Vec<u8>, DbError> {
        let _guard = DB_LOCK.lock();
        copy::load_recording_bytes(&self.path, &db_ref.identity, &db_ref.group_name)
            .map_err(Into::into)
    }

    fn list_recordings(&self) -> Result<Vec<RecordingEntry>, DbError> {
        let _guard = DB_LOCK.lock();
        let file =
            hdf5_pure::File::open(&self.path).map_err(|e| DbError::Backend(e.to_string()))?;
        let root = file.root();
        let Ok(by_id) = root.group("by_identity") else {
            return Ok(vec![]);
        };
        let mut entries = Vec::new();
        for identity in by_id
            .groups()
            .map_err(|e| DbError::Backend(e.to_string()))?
        {
            let Ok(id_grp) = by_id.group(&identity) else {
                continue;
            };
            for rec_name in id_grp
                .groups()
                .map_err(|e| DbError::Backend(e.to_string()))?
            {
                let Ok(rec_grp) = id_grp.group(&rec_name) else {
                    continue;
                };
                let Ok(attrs) = rec_grp.attrs() else {
                    continue;
                };
                if let Some(meta) = from_attrs(&attrs) {
                    let hidden =
                        matches!(attrs.get(ATTR_HIDDEN), Some(AttrValue::U64(v)) if *v != 0);
                    entries.push(RecordingEntry {
                        db_ref: DatabaseRef {
                            identity: identity.clone(),
                            group_name: rec_name,
                        },
                        meta,
                        hidden,
                    });
                }
            }
        }
        entries.sort_unstable_by_key(|e| std::cmp::Reverse(e.meta.start_us));
        Ok(entries)
    }

    fn is_duplicate(&self, _identity: &str, meta: &RecordingMeta) -> Result<bool, DbError> {
        let _guard = DB_LOCK.lock();
        let file =
            hdf5_pure::File::open(&self.path).map_err(|e| DbError::Backend(e.to_string()))?;
        let root = file.root();

        let Ok(by_id) = root.group("by_identity") else {
            return Ok(false);
        };

        // Search through ALL identity groups
        for identity in by_id
            .groups()
            .map_err(|e| DbError::Backend(e.to_string()))?
        {
            let Ok(id_grp) = by_id.group(&identity) else {
                continue;
            };

            for rec_name in id_grp
                .groups()
                .map_err(|e| DbError::Backend(e.to_string()))?
            {
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

    fn delete_batch(&mut self, refs: &[DatabaseRef]) -> Result<(), DbError> {
        let _guard = DB_LOCK.lock();
        if refs.is_empty() {
            return Ok(());
        }
        copy::delete_batch(&self.path, refs).map_err(Into::into)
    }

    fn path(&self) -> &Path {
        &self.path
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

        fb.write(path)
            .map_err(|e| DbError::Backend(e.to_string()))?;

        log::info!("Created history database at {}", path.display());
        Ok(())
    }

    fn validate_existing(path: &Path) -> Result<(), DbError> {
        let file = hdf5_pure::File::open(path).map_err(|e| DbError::Backend(e.to_string()))?;
        let root = file.root();
        let attrs = root.attrs().map_err(|e| DbError::Backend(e.to_string()))?;

        let schema_version = match attrs.get(SCHEMA_VERSION_ATTR) {
            Some(AttrValue::I64(v)) => *v,
            _ => 0,
        };

        if schema_version > CURRENT_SCHEMA_VERSION {
            return Err(DbError::SchemaTooNew {
                found: schema_version,
                supported: CURRENT_SCHEMA_VERSION,
            });
        }

        if schema_version < CURRENT_SCHEMA_VERSION {
            log::info!(
                "Migrating history database from schema_version={schema_version} to {}",
                CURRENT_SCHEMA_VERSION
            );
            drop(file);
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

/// Helper for hdf5-pure attribute extraction.
fn from_attrs(attrs: &std::collections::HashMap<String, AttrValue>) -> Option<RecordingMeta> {
    let start_us = match attrs.get(ATTR_START_US)? {
        AttrValue::I64(v) => *v,
        _ => return None,
    };
    let end_us = match attrs.get(ATTR_END_US) {
        Some(AttrValue::I64(v)) => *v,
        _ => start_us,
    };
    let nav_point_count = match attrs.get(ATTR_NAV_POINT_COUNT)? {
        AttrValue::U64(v) => *v,
        _ => return None,
    };
    let sat_report_count = match attrs.get(ATTR_SAT_REPORT_COUNT)? {
        AttrValue::U64(v) => *v,
        _ => return None,
    };
    let marker_count = match attrs.get(ATTR_MARKER_COUNT)? {
        AttrValue::U64(v) => *v,
        _ => return None,
    };
    let event_marker_count = match attrs.get(ATTR_EVENT_MARKER_COUNT)? {
        AttrValue::U64(v) => *v,
        _ => return None,
    };
    let gtd_size_bytes = match attrs.get(ATTR_GTD_SIZE_BYTES) {
        Some(AttrValue::U64(v)) => *v,
        _ => 0,
    };
    Some(RecordingMeta {
        start_us,
        end_us,
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
    let start_us = match attrs.get(ATTR_START_US) {
        Some(AttrValue::I64(v)) => *v,
        _ => return false,
    };
    let nav_point_count = match attrs.get(ATTR_NAV_POINT_COUNT) {
        Some(AttrValue::U64(v)) => *v,
        _ => return false,
    };
    let sat_report_count = match attrs.get(ATTR_SAT_REPORT_COUNT) {
        Some(AttrValue::U64(v)) => *v,
        _ => return false,
    };
    let marker_count = match attrs.get(ATTR_MARKER_COUNT) {
        Some(AttrValue::U64(v)) => *v,
        _ => return false,
    };
    let event_marker_count = match attrs.get(ATTR_EVENT_MARKER_COUNT) {
        Some(AttrValue::U64(v)) => *v,
        _ => return false,
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
    let file =
        hdf5_pure::File::from_bytes(bytes.to_vec()).map_err(|e| DbError::Backend(e.to_string()))?;

    let nav_grp = file
        .group("nav_points")
        .map_err(|e| DbError::Backend(e.to_string()))?;
    let nav_shape = nav_grp
        .dataset("time")
        .map_err(|e| DbError::Backend(e.to_string()))?
        .shape()
        .map_err(|e| DbError::Backend(e.to_string()))?;
    let nav_point_count = nav_shape.first().copied().unwrap_or(0);

    let (start_us, end_us) = if nav_point_count > 0 {
        let times = nav_grp
            .dataset("time")
            .map_err(|e| DbError::Backend(e.to_string()))?
            .read_i64()
            .map_err(|e| DbError::Backend(e.to_string()))?;
        let first = times.first().copied().unwrap_or(0);
        let last = times.last().copied().unwrap_or(0);
        (first, last)
    } else {
        (0, 0)
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
        start_us,
        end_us,
        nav_point_count,
        sat_report_count,
        marker_count,
        event_marker_count,
        gtd_size_bytes: bytes.len() as u64,
    })
}
