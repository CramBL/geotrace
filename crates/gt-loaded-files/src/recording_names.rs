//! Per-file recording display names, resolved from the user's name template.

use gt_fmt::{NameFields, render_name_template};
use gt_types::{FileIdx, LoadedFile, TrackRef};

use crate::{LoadedFileId, LoadedFilesView};

/// The display name of every loaded file, resolved from the user's
/// recording-name template.
///
/// A surface that names a recording reads it from here, so a template change
/// moves all of them together. The raw filename still shows where the file
/// itself is the subject - a file row's hover path, the recording-details
/// header - and those read `FileMetadata` directly.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RecordingNames {
    names: Vec<String>,
}

impl RecordingNames {
    /// Resolve `template` against every loaded file.
    ///
    /// The `{filename}` token gets the name with the longest directory prefix
    /// shared by all loaded files removed, so files like
    /// `/home/user/recordings/a.gtd` and `/home/user/recordings/b.gtd` show as
    /// `a.gtd` and `b.gtd`.
    pub fn resolve(files: LoadedFilesView<'_>, template: &str) -> Self {
        let filenames: Vec<&str> = files
            .files()
            .iter()
            .map(|file| file.metadata.filename.as_str())
            .collect();
        let prefix_len = common_path_prefix_len(&filenames);
        let names = files
            .entries()
            .map(|entry| {
                let metadata = &entry.file().metadata;
                let filename = metadata.filename.as_str();
                // `prefix_len` is always a valid char boundary (guaranteed by
                // `common_path_prefix_len`), but use `get` to satisfy the
                // `clippy::string_slice` lint and handle degenerate inputs safely.
                let stripped = filename
                    .get(prefix_len..)
                    .filter(|name| !name.is_empty())
                    .unwrap_or(filename);
                let fields = NameFields {
                    title: metadata.title.as_deref(),
                    device: metadata.device.as_deref(),
                    // The identity's internal `auto:` marker never reaches a
                    // label.
                    identity: entry.identity().map(|id| crate::display_identity(id).0),
                    filename: stripped,
                };
                render_name_template(template, &fields)
            })
            .collect();
        Self { names }
    }

    /// The display name of `file`, or `None` when no such file is loaded.
    pub fn get(&self, file: FileIdx) -> Option<&str> {
        self.names.get(file.as_usize()).map(String::as_str)
    }

    /// The name a surface shows for `file`, falling back to the raw filename
    /// where the template resolved nothing, and `None` when no such file is
    /// loaded.
    pub fn display_name<'a>(&'a self, files: &'a [LoadedFile], file: FileIdx) -> Option<&'a str> {
        self.get(file)
            .or_else(|| file.get(files).map(|file| file.metadata.filename.as_str()))
    }

    /// [`RecordingNames::display_name`] of the recording `id` identifies,
    /// `None` once that recording is no longer loaded.
    pub fn display_name_of_loaded_recording<'a>(
        &'a self,
        recordings: LoadedFilesView<'a>,
        id: LoadedFileId,
    ) -> Option<&'a str> {
        let index = recordings.entries().position(|entry| entry.id() == id)?;
        self.display_name(recordings.files(), FileIdx::new(index))
    }

    /// How a surface names one track: the recording's display name, with the
    /// track number appended where the recording split into several tracks.
    ///
    /// [`None`] when no such file is loaded. The raw filename stands in where
    /// the template resolved no name for the recording.
    pub fn track_label(&self, files: &[LoadedFile], track: TrackRef) -> Option<String> {
        let file = track.fi.get(files)?;
        let name = self
            .get(track.fi)
            .unwrap_or(file.metadata.filename.as_str());
        if file.tracks.len() < 2 {
            return Some(name.to_owned());
        }
        Some(format!("{name} (track {})", track.index.as_usize() + 1))
    }

    pub fn len(&self) -> usize {
        self.names.len()
    }

    pub fn is_empty(&self) -> bool {
        self.names.is_empty()
    }
}

/// Returns the length of the longest common directory prefix shared by all
/// names: the byte offset at which each name's display form begins.
///
/// Returns `0` when there is nothing meaningful to strip: fewer than two names,
/// no name contains a path separator (so there is no directory structure to
/// collapse), or the common bytes do not reach a separator boundary.
fn common_path_prefix_len(names: &[&str]) -> usize {
    if names.len() < 2 {
        return 0;
    }
    if !names.iter().any(|n| n.contains(['/', '\\'])) {
        return 0;
    }
    let Some(&first) = names.first() else {
        return 0;
    };
    // Count matching bytes between `first` and every other name.
    let common_bytes = names.iter().skip(1).fold(first.len(), |acc, name| {
        let len = first
            .bytes()
            .zip(name.bytes())
            .take_while(|(a, b)| a == b)
            .count();
        acc.min(len)
    });
    // `common_bytes` may land in the middle of a multi-byte character, so snap
    // to a valid char boundary before searching for the last separator. Because the
    // leading `common_bytes` bytes are identical in all names, the resulting
    // index is a valid char boundary in every name.
    let common_bytes = first.floor_char_boundary(common_bytes);
    match first.get(..common_bytes).and_then(|s| s.rfind(['/', '\\'])) {
        // +1 to start the display name after the separator itself.
        Some(pos) => pos + 1,
        None => 0,
    }
}

#[cfg(test)]
mod tests {
    use rustc_hash::FxHashMap;

    use gt_types::{FileIdx, FileMetadata, LoadedFile, TrackIdx, TrackRef};

    use super::{RecordingNames, common_path_prefix_len};
    use crate::{FileHistory, LoadedFiles};

    fn file(filename: &str, title: Option<&str>) -> LoadedFile {
        LoadedFile {
            metadata: FileMetadata {
                filename: filename.to_owned(),
                title: title.map(ToOwned::to_owned),
                ..gt_test_utils::empty_file_metadata()
            },
            tracks: Vec::new(),
            event_marker_styles: FxHashMap::default(),
            orphaned_event_markers: Vec::new(),
            source: gt_types::FileSource::GtdPath(std::path::PathBuf::new()),
            load_warnings: Vec::new(),
        }
    }

    /// The same file with `track_count` tracks, which is what decides whether
    /// a track label carries a track number.
    fn file_with_tracks(filename: &str, title: Option<&str>, track_count: usize) -> LoadedFile {
        LoadedFile {
            tracks: (0..track_count)
                .map(|_| gt_test_utils::fixtures::loaded_track_with_points(Vec::new()))
                .collect(),
            ..file(filename, title)
        }
    }

    fn meta() -> gt_history_types::RecordingMeta {
        gt_history_types::RecordingMeta {
            time_range: None,
            nav_point_count: 0,
            sat_report_count: 0,
            marker_count: 0,
            event_marker_count: 0,
            gtd_size_bytes: 0,
        }
    }

    fn names(template: &str, files: &LoadedFiles) -> Vec<String> {
        let resolved = RecordingNames::resolve(files.view(), template);
        (0..files.len())
            .filter_map(|i| resolved.get(FileIdx::new(i)).map(ToOwned::to_owned))
            .collect()
    }

    #[test]
    fn identity_token_drops_the_auto_prefix() {
        let mut files = LoadedFiles::new();
        files.push(
            file("ride.gtd", None),
            FileHistory::recording("auto:Morning ride".to_owned(), meta(), None),
        );
        assert_eq!(names("{identity}", &files), ["Morning ride"]);
    }

    #[test]
    fn metadata_tokens_render_and_fall_back_to_the_filename() {
        let mut files = LoadedFiles::new();
        files.push(file("ride.gtd", Some("Morning ride")), FileHistory::None);
        assert_eq!(names("{title}", &files), ["Morning ride"]);

        let mut untitled = LoadedFiles::new();
        untitled.push(file("ride.gtd", None), FileHistory::None);
        assert_eq!(names("{title}", &untitled), ["ride.gtd"]);
    }

    #[test]
    fn filename_token_drops_the_shared_directory_prefix() {
        let mut files = LoadedFiles::new();
        files.push(file("/home/user/rec/a.gtd", None), FileHistory::None);
        files.push(file("/home/user/rec/b.gtd", None), FileHistory::None);
        assert_eq!(names("{filename}", &files), ["a.gtd", "b.gtd"]);
    }

    /// A surface holding a recording's session identity resolves the name
    /// through the loaded recordings, and gets nothing once that recording has
    /// unloaded.
    #[test]
    fn a_recording_is_named_by_its_session_identity_while_it_is_loaded() {
        let mut files = LoadedFiles::new();
        files.push(file("a.gtd", Some("Morning ride")), FileHistory::None);
        files.push(file("b.gtd", Some("Evening ride")), FileHistory::None);
        let evening = files
            .view()
            .get(1)
            .map(|entry| entry.id())
            .expect("the fixture file is loaded");
        let names = RecordingNames::resolve(files.view(), "{title}");

        assert_eq!(
            names.display_name_of_loaded_recording(files.view(), evening),
            Some("Evening ride")
        );

        files.remove_file(1);

        assert_eq!(
            names.display_name_of_loaded_recording(files.view(), evening),
            None
        );
    }

    #[test]
    fn empty_slice_returns_zero() {
        assert_eq!(common_path_prefix_len(&[]), 0);
    }

    #[test]
    fn single_name_returns_zero() {
        assert_eq!(
            common_path_prefix_len(&["/home/user/recordings/ride.gtd"]),
            0
        );
    }

    #[test]
    fn no_path_separators_returns_zero() {
        assert_eq!(common_path_prefix_len(&["ride_0.gtd", "ride_1.gtd"]), 0);
    }

    #[test]
    fn shared_directory_prefix_is_stripped() {
        let names = [
            "/home/user/recordings/2024-01-15.gtd",
            "/home/user/recordings/2024-01-16.gtd",
        ];
        assert_eq!(
            common_path_prefix_len(&names),
            "/home/user/recordings/".len()
        );
    }

    #[test]
    fn common_bytes_mid_component_trims_to_last_separator() {
        // "/home/user/recordings/…" vs "/home/user/recent/…" share "/home/user/"
        // even though more bytes match inside the next component.
        let names = ["/home/user/recordings/a.gtd", "/home/user/recent/b.gtd"];
        assert_eq!(common_path_prefix_len(&names), "/home/user/".len());
    }

    #[test]
    fn no_common_directory_prefix_strips_only_root_slash() {
        // The only shared byte is the leading '/', so we strip that.
        let names = ["/alpha/a.gtd", "/beta/b.gtd"];
        assert_eq!(common_path_prefix_len(&names), 1);
    }

    #[test]
    fn truly_no_common_prefix_returns_zero() {
        let names = ["alpha/a.gtd", "beta/b.gtd"];
        assert_eq!(common_path_prefix_len(&names), 0);
    }

    #[test]
    fn windows_backslash_separator() {
        let names = [
            r"C:\Users\alice\recordings\ride_a.gtd",
            r"C:\Users\alice\recordings\ride_b.gtd",
        ];
        assert_eq!(
            common_path_prefix_len(&names),
            r"C:\Users\alice\recordings\".len()
        );
    }

    /// A track is named by its recording, and by its number within it once
    /// the recording holds more than one track. The raw filename stands in
    /// where the template resolved nothing.
    #[rstest::rstest]
    #[case::the_only_track_of_a_recording("{title}", 1, 0, "Morning ride")]
    #[case::one_of_several_tracks("{title}", 3, 1, "Morning ride (track 2)")]
    #[case::an_unresolved_template("{device}", 3, 2, "ride.gtd (track 3)")]
    fn a_track_is_named_by_its_recording_and_its_number(
        #[case] template: &str,
        #[case] track_count: usize,
        #[case] track_index: usize,
        #[case] expected: &str,
    ) {
        let mut files = LoadedFiles::new();
        files.push(
            file_with_tracks("ride.gtd", Some("Morning ride"), track_count),
            FileHistory::None,
        );
        let names = RecordingNames::resolve(files.view(), template);

        let label = names.track_label(
            files.files(),
            TrackRef::new(FileIdx::new(0), TrackIdx::new(track_index)),
        );

        assert_eq!(label.as_deref(), Some(expected));
    }

    /// A track of a file that is not loaded has no label at all.
    #[test]
    fn a_track_of_an_unloaded_recording_has_no_label() {
        let names = RecordingNames::default();

        assert_eq!(
            names.track_label(&[], TrackRef::new(FileIdx::new(0), TrackIdx::new(0))),
            None
        );
    }
}
