use gt_loaded_files::RecordingNames;
use gt_types::{FileIdx, LoadedFile};

/// Holds the names of the loaded recordings for the map's own labels, so a
/// point's context menu and its tooltips say what the side panel says.
#[derive(Clone, Copy)]
pub(crate) struct RecordingLabels<'a> {
    files: &'a [LoadedFile],
    names: &'a RecordingNames,
}

impl<'a> RecordingLabels<'a> {
    pub(crate) fn new(files: &'a [LoadedFile], names: &'a RecordingNames) -> Self {
        Self { files, names }
    }

    /// Falls back to the raw filename when the name template resolved nothing
    /// for this file.
    pub(crate) fn display_name(self, file: FileIdx) -> Option<&'a str> {
        self.names.display_name(self.files, file)
    }

    /// The name for a point tooltip's recording row, which stays empty while a
    /// single file is loaded: every point then comes from the same recording.
    pub(crate) fn name_when_several_files_loaded(self, file: FileIdx) -> Option<&'a str> {
        if self.files.len() < 2 {
            return None;
        }
        self.display_name(file)
    }
}

#[cfg(test)]
mod tests {
    use gt_loaded_files::{FileHistory, LoadedFiles, RecordingNames};
    use gt_types::{FileIdx, FileMetadata, FileSource, LoadedFile};
    use rustc_hash::FxHashMap;

    use super::RecordingLabels;

    fn file(filename: &str, title: &str) -> LoadedFile {
        LoadedFile {
            metadata: FileMetadata {
                filename: filename.to_owned(),
                title: Some(title.to_owned()),
                ..gt_test_utils::empty_file_metadata()
            },
            tracks: Vec::new(),
            event_marker_styles: FxHashMap::default(),
            orphaned_event_markers: Vec::new(),
            source: FileSource::GtdPath(std::path::PathBuf::new()),
            load_warnings: Vec::new(),
        }
    }

    fn loaded(titles: &[(&str, &str)]) -> LoadedFiles {
        let mut files = LoadedFiles::new();
        for &(filename, title) in titles {
            files.push(file(filename, title), FileHistory::None);
        }
        files
    }

    #[test]
    fn tooltip_row_is_empty_with_one_file_and_named_with_several() {
        let single = loaded(&[("a.gtd", "Morning ride")]);
        let names = RecordingNames::resolve(single.view(), "{title}");
        assert_eq!(
            RecordingLabels::new(single.files(), &names)
                .name_when_several_files_loaded(FileIdx::new(0)),
            None
        );

        let several = loaded(&[("a.gtd", "Morning ride"), ("b.gtd", "Evening ride")]);
        let names = RecordingNames::resolve(several.view(), "{title}");
        assert_eq!(
            RecordingLabels::new(several.files(), &names)
                .name_when_several_files_loaded(FileIdx::new(1)),
            Some("Evening ride")
        );
    }

    #[test]
    fn display_name_falls_back_to_the_filename_when_unresolved() {
        let files = loaded(&[("a.gtd", "Morning ride")]);
        let empty = RecordingNames::default();
        assert_eq!(
            RecordingLabels::new(files.files(), &empty).display_name(FileIdx::new(0)),
            Some("a.gtd")
        );
    }
}
