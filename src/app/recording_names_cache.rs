//! The display name of every loaded file, held between the frames its inputs
//! stay the same.

use gt_loaded_files::{IndexAlignedFiles, LoadedFiles, RecordingNames};
use gt_types::Generation;

/// What the names held in [`RecordingNamesCache`] were resolved from: the
/// loaded files as their generation, and the template as a copy.
///
/// The template is a `String` setting. The key holds a copy of it, and
/// comparing the copy costs one string comparison per frame. A newtype inside a
/// [`gt_types::Versioned`] would drop that comparison. Introduce it once a
/// second cache compares the template too.
#[derive(Debug)]
struct ResolveInputs {
    files_generation: Generation<IndexAlignedFiles>,
    template: String,
}

/// The display names of the loaded recordings, resolved again only when the
/// loaded files or the recording-name template change.
///
/// One resolve renders a `String` per loaded file, and the side panel, the map,
/// the plot, the space weather assessment, the log matches and the log viewer
/// all read the names of the same frame.
#[derive(Debug, Default)]
pub(super) struct RecordingNamesCache {
    resolved_from: Option<ResolveInputs>,
    names: RecordingNames,
    #[cfg(test)]
    resolve_count: usize,
}

impl RecordingNamesCache {
    pub(super) fn names(&mut self, files: &LoadedFiles, template: &str) -> &RecordingNames {
        let files_generation = files.generation();
        let resolved_from_the_same_inputs = self.resolved_from.as_ref().is_some_and(|inputs| {
            inputs.files_generation == files_generation && inputs.template == template
        });
        if !resolved_from_the_same_inputs {
            self.names = RecordingNames::resolve(files.view(), template);
            self.resolved_from = Some(ResolveInputs {
                files_generation,
                template: template.to_owned(),
            });
            #[cfg(test)]
            {
                self.resolve_count += 1;
            }
        }
        &self.names
    }

    #[cfg(test)]
    pub(super) fn resolve_count(&self) -> usize {
        self.resolve_count
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use gt_loaded_files::FileHistory;
    use gt_types::{FileIdx, FileSource, LoadedFile};
    use rustc_hash::FxHashMap;

    use super::*;

    fn one_loaded_file() -> LoadedFiles {
        let file = LoadedFile {
            metadata: gt_types::FileMetadata {
                filename: "ride.gtd".to_owned(),
                title: Some("Morning ride".to_owned()),
                ..gt_test_utils::empty_file_metadata()
            },
            tracks: Vec::new(),
            event_marker_styles: FxHashMap::default(),
            orphaned_event_markers: Vec::new(),
            source: FileSource::GtdBytes(Arc::from(Vec::<u8>::new())),
            load_warnings: Vec::new(),
        };
        let mut files = LoadedFiles::new();
        files.push(file, FileHistory::None);
        files
    }

    #[test]
    fn a_second_call_with_the_same_files_and_template_hands_back_the_resolved_names() {
        let files = one_loaded_file();
        let mut cache = RecordingNamesCache::default();

        let first = cache.names(&files, "{title}").clone();
        let second = cache.names(&files, "{title}").clone();

        assert_eq!(cache.resolve_count(), 1);
        assert_eq!(first.get(FileIdx::new(0)), Some("Morning ride"));
        assert_eq!(second, first);
    }

    #[test]
    fn a_changed_template_resolves_again() {
        let files = one_loaded_file();
        let mut cache = RecordingNamesCache::default();
        assert_eq!(
            cache.names(&files, "{title}").get(FileIdx::new(0)),
            Some("Morning ride")
        );

        let names = cache.names(&files, "{filename}");

        assert_eq!(names.get(FileIdx::new(0)), Some("ride.gtd"));
        assert_eq!(cache.resolve_count(), 2);
    }

    /// Re-segmentation and re-placement rewrite a track through
    /// [`LoadedFiles::files_mut`], leaving the file count and the template as
    /// they were.
    #[test]
    fn a_mutation_of_the_loaded_files_resolves_again() {
        let mut files = one_loaded_file();
        let mut cache = RecordingNamesCache::default();
        cache.names(&files, "{title}");

        files.files_mut();
        cache.names(&files, "{title}");

        assert_eq!(cache.resolve_count(), 2);
    }
}
