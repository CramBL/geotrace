use std::collections::HashMap;
use std::path::Path;

use hdf5_pure::{AttrValue, Dataset, File, FormatError, Group};

use crate::error::Error;

/// A dataset of a well-formed file decodes to at most this multiple of the
/// file's byte length: deflate's maximum expansion ratio is 1032:1. Every
/// `.gtd` dataset is stored whole, with no chunk left to the fill value.
const MAX_DECODED_BYTES_PER_FILE_BYTE: u128 = 1032;

/// A `.gtd` file whose datasets are reachable only through a check of the size
/// they declare against the bytes the file holds.
///
/// A dataset's declared element count and element size are read from its header
/// and are whatever the file says. Reading one allocates their product, so a
/// hostile file can state a size no allocator can serve.
pub(crate) struct SizeCheckedFile {
    file: File,
    file_bytes: u64,
}

impl SizeCheckedFile {
    pub(crate) fn from_bytes(bytes: Vec<u8>) -> Result<Self, Error> {
        Ok(Self::new(File::from_bytes(bytes)?))
    }

    pub(crate) fn open(path: &Path) -> Result<Self, Error> {
        Ok(Self::new(File::open(path)?))
    }

    fn new(file: File) -> Self {
        let file_bytes = file.file_size();
        Self { file, file_bytes }
    }

    pub(crate) fn root(&self) -> SizeCheckedGroup {
        SizeCheckedGroup {
            group: self.file.root(),
            path: String::new(),
            file_bytes: self.file_bytes,
        }
    }

    pub(crate) fn group(&self, name: &str) -> Result<SizeCheckedGroup, Error> {
        Ok(SizeCheckedGroup {
            group: self.file.group(name)?,
            path: name.to_owned(),
            file_bytes: self.file_bytes,
        })
    }

    /// Open a group the file need not hold, returning `Ok(None)` where the file
    /// has no such path. Every other failure, including a path that is not a
    /// group, is an [`Error`].
    pub(crate) fn optional_group(&self, name: &str) -> Result<Option<SizeCheckedGroup>, Error> {
        Ok(
            none_for_a_missing_path(self.file.group(name))?.map(|group| SizeCheckedGroup {
                group,
                path: name.to_owned(),
                file_bytes: self.file_bytes,
            }),
        )
    }
}

pub(crate) struct SizeCheckedGroup {
    group: Group,
    path: String,
    file_bytes: u64,
}

impl SizeCheckedGroup {
    pub(crate) fn attrs(&self) -> Result<HashMap<String, AttrValue>, Error> {
        Ok(self.group.attrs()?)
    }

    pub(crate) fn groups(&self) -> Result<Vec<String>, Error> {
        Ok(self.group.groups()?)
    }

    pub(crate) fn group(&self, name: &str) -> Result<Self, Error> {
        Ok(Self {
            group: self.group.group(name)?,
            path: self.child_path(name),
            file_bytes: self.file_bytes,
        })
    }

    /// Open a dataset, or return [`Error::DatasetSizePastFileLength`] where the
    /// bytes it declares are more than the file's own length can decode to.
    pub(crate) fn dataset(&self, name: &str) -> Result<Dataset, Error> {
        self.size_checked(name, self.group.dataset(name)?)
    }

    /// Open a dataset the group need not hold, returning `Ok(None)` where the
    /// group has no such child. Every other failure, including a child that is
    /// not a dataset, is an [`Error`].
    pub(crate) fn optional_dataset(&self, name: &str) -> Result<Option<Dataset>, Error> {
        none_for_a_missing_path(self.group.dataset(name))?
            .map(|dataset| self.size_checked(name, dataset))
            .transpose()
    }

    fn size_checked(&self, name: &str, dataset: Dataset) -> Result<Dataset, Error> {
        let element_size = u128::from(dataset.element_size()?);
        let declared_bytes = dataset
            .shape()?
            .iter()
            .fold(element_size, |bytes, &dim| bytes.saturating_mul(dim.into()));
        let max_bytes = u128::from(self.file_bytes).saturating_mul(MAX_DECODED_BYTES_PER_FILE_BYTE);
        if declared_bytes > max_bytes {
            return Err(Error::DatasetSizePastFileLength {
                path: self.child_path(name),
                declared_bytes,
                file_bytes: self.file_bytes,
            });
        }
        Ok(dataset)
    }

    fn child_path(&self, name: &str) -> String {
        if self.path.is_empty() {
            name.to_owned()
        } else {
            format!("{}/{name}", self.path)
        }
    }
}

/// `Ok(None)` where the lookup failed because the file holds no such path, and
/// the error itself for every other failure.
fn none_for_a_missing_path<T>(lookup: Result<T, hdf5_pure::Error>) -> Result<Option<T>, Error> {
    match lookup {
        Ok(found) => Ok(Some(found)),
        Err(hdf5_pure::Error::Format(FormatError::PathNotFound(_))) => Ok(None),
        Err(error) => Err(error.into()),
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    /// The crash input from the 2026-09-02 scheduled fuzz run, whose
    /// `tracked_sats/sat_report_idx` declares 5 497 558 139 455 elements of 8
    /// bytes.
    #[test]
    fn a_dataset_declaring_more_bytes_than_the_file_holds_is_rejected() {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../../tests/fixtures/fuzz_regressions/dataset_size_past_file_length.gtd");
        let file = SizeCheckedFile::open(&path).unwrap();

        let error = file
            .group("tracked_sats")
            .unwrap()
            .dataset("sat_report_idx")
            .unwrap_err();

        assert!(
            matches!(error, Error::DatasetSizePastFileLength { .. }),
            "{error:#}"
        );
    }
}
