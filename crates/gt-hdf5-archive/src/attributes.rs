//! File-level attributes, which archives use for their schema version.

use hdf5::File;

use crate::ArchiveError;

pub fn write_i64(file: &File, name: &str, value: i64) -> Result<(), ArchiveError> {
    Ok(file.new_attr::<i64>().create(name)?.write_scalar(&value)?)
}

/// The attribute's value, or [`None`] where the file has no such attribute.
pub fn read_i64(file: &File, name: &str) -> Option<i64> {
    file.attr(name)
        .and_then(|attr| attr.read_scalar::<i64>())
        .ok()
}
