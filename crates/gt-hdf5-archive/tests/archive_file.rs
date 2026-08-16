//! Opening and creating a real archive file in a temp directory.

use gt_hdf5_archive::{ArchiveError, ArchiveFile};

#[test]
fn creating_an_archive_creates_the_directory_it_sits_in() {
    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().join("nested").join("deeper").join("archive.h5");
    let mut archive = ArchiveFile::new(&path);
    assert!(!archive.exists());

    archive.create().expect("create");

    assert!(archive.exists());
    assert!(path.exists());
}

#[test]
fn a_parent_that_is_a_file_is_reported_as_an_io_failure() {
    let dir = tempfile::tempdir().expect("temp dir");
    let occupied = dir.path().join("occupied");
    std::fs::write(&occupied, b"not a directory").expect("write");

    let err = ArchiveFile::new(occupied.join("archive.h5"))
        .create()
        .err()
        .expect("create under a file");

    assert!(matches!(err, ArchiveError::Io(_)), "{err}");
}

#[test]
fn an_archive_that_does_not_exist_cannot_be_opened() {
    let dir = tempfile::tempdir().expect("temp dir");
    let err = ArchiveFile::new(dir.path().join("absent.h5"))
        .open_read_only()
        .err()
        .expect("open a missing archive");

    assert!(matches!(err, ArchiveError::Backend(_)), "{err}");
}
