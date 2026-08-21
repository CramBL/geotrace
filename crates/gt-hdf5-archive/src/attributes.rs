//! Attributes of a file or of one group in it, which archives use for their
//! schema version and for the state a delete leaves behind.

use hdf5::Group;

use crate::ArchiveError;

pub fn write_i64(group: &Group, name: &str, value: i64) -> Result<(), ArchiveError> {
    Ok(group.new_attr::<i64>().create(name)?.write_scalar(&value)?)
}

/// Write the attribute, creating it where the archive does not carry it yet.
///
/// An attribute created after the object it sits on costs the file a header
/// block, so an attribute written repeatedly is created with the object.
pub fn set_i64(group: &Group, name: &str, value: i64) -> Result<(), ArchiveError> {
    match group.attr(name) {
        Ok(attr) => Ok(attr.write_scalar(&value)?),
        Err(_) => write_i64(group, name, value),
    }
}

/// The attribute's value, or [`None`] where the object has no such attribute.
pub fn read_i64(group: &Group, name: &str) -> Option<i64> {
    group
        .attr(name)
        .and_then(|attr| attr.read_scalar::<i64>())
        .ok()
}
