use std::collections::HashMap;
use std::path::Path;

use hdf5_pure::{AttrValue, Dataset, File, Group};

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
        let dataset = self.group.dataset(name)?;
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
