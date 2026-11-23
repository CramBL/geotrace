use std::num::NonZeroUsize;

use gt_types::{Generation, TimeRange, TrackIdx, TrackRef};

use crate::{IndexAlignedFiles, LoadedFileEntry, LoadedFileId, LoadedFilesView};

pub const MAX_RECORDINGS_PER_PAGE: usize = 100;

#[derive(Debug, Clone, Copy)]
pub struct ListRecordings {
    /// Resume after this ID even if its recording has been unloaded.
    pub after: Option<LoadedFileId>,
    pub limit: NonZeroUsize,
}

impl ListRecordings {
    /// List recordings in load order, up to [`MAX_RECORDINGS_PER_PAGE`].
    pub fn execute(self, files: LoadedFilesView<'_>, name_template: &str) -> RecordingPage {
        let ids = &files.loaded_files.storage.get().ids;
        let start = self
            .after
            .map_or(0, |after| ids.partition_point(|id| *id <= after));
        let limit = self.limit.get().min(MAX_RECORDINGS_PER_PAGE);
        let end = start.saturating_add(limit).min(ids.len());
        let recordings: Vec<_> = (start..end)
            .filter_map(|index| files.get(index))
            .map(|entry| RecordingSummary::new(entry, name_template))
            .collect();
        let next_after = if end < ids.len() {
            recordings.last().map(|summary| summary.id)
        } else {
            None
        };
        RecordingPage {
            recordings,
            next_after,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordingPage {
    pub recordings: Vec<RecordingSummary>,
    pub next_after: Option<LoadedFileId>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordingSummary {
    pub id: LoadedFileId,
    pub display_name: String,
    pub filename: String,
    pub title: Option<String>,
    pub device: Option<String>,
    /// The time range of the loaded tracks. An empty recording has no range.
    pub time_range: Option<TimeRange>,
    pub track_count: usize,
}

impl RecordingSummary {
    fn new(entry: LoadedFileEntry<'_>, name_template: &str) -> Self {
        let file = entry.file();
        let filename = entry.basename();
        let mut name_fields = entry.name_fields(filename);
        let source_filename = file.metadata.filename.as_str();
        if filename != source_filename
            && let Some(identity) = entry.identity()
            && let (display, true) = crate::display_identity(identity)
            && display == crate::display_identity(source_filename).0
        {
            name_fields.identity = Some(filename);
        }
        Self {
            id: entry.id(),
            display_name: gt_fmt::render_name_template(name_template, &name_fields),
            filename: filename.to_owned(),
            title: file.metadata.title.clone(),
            device: file.metadata.device.clone(),
            time_range: file.metadata.time_range,
            track_count: file.tracks.len(),
        }
    }
}

/// A track address valid for one generation of the loaded recordings.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SessionTrackRef {
    recording: LoadedFileId,
    index: TrackIdx,
    generation: Generation<IndexAlignedFiles>,
}

impl LoadedFilesView<'_> {
    pub fn session_track_ref(&self, track: TrackRef) -> Option<SessionTrackRef> {
        let entry = self.entry_for(track.fi)?;
        track.index.get(&entry.file().tracks)?;
        Some(SessionTrackRef {
            recording: entry.id(),
            index: track.index,
            generation: self.generation(),
        })
    }

    /// Resolve a reference from the current generation to a loaded track.
    pub fn resolve_session_track(&self, track: SessionTrackRef) -> Option<TrackRef> {
        if track.generation != self.generation() {
            return None;
        }
        let entry = self.entry_for_id(track.recording)?;
        track.index.get(&entry.file().tracks)?;
        Some(TrackRef::new(entry.fi, track.index))
    }
}

#[cfg(test)]
mod tests {
    use std::num::NonZeroUsize;

    use chrono::{DateTime, Duration};
    use gt_types::{FileIdx, TimeRange, TrackIdx, TrackRef};
    use rstest::rstest;

    use super::{ListRecordings, MAX_RECORDINGS_PER_PAGE};
    use crate::{FileHistory, LoadedFiles, test_util};

    #[test]
    fn recording_summary_uses_the_configured_template_and_precise_loaded_times() {
        let start = DateTime::UNIX_EPOCH - Duration::microseconds(121);
        let end = DateTime::UNIX_EPOCH + Duration::microseconds(345);
        let points = [start, end]
            .into_iter()
            .flat_map(|instant| gt_test_utils::fixtures::nav_points_from(instant, 1, 1))
            .collect();
        let mut file =
            gt_test_utils::loaded_file_with_tracks(vec![gt_test_utils::loaded_track_with_points(
                points,
            )]);
        file.metadata.filename = r"C:\private\recordings\morning.gtd".to_owned();
        file.metadata.title = Some("Morning".to_owned());
        file.metadata.device = Some("Receiver".to_owned());
        let mut files = LoadedFiles::new();
        files.push(
            file,
            FileHistory::recording(
                "auto:Field test".to_owned(),
                test_util::empty_recording_meta(),
                None,
            ),
        );

        let page = ListRecordings {
            after: None,
            limit: NonZeroUsize::MIN,
        }
        .execute(files.view(), "{identity} {title} {device} {filename}");

        let [recording] = page.recordings.as_slice() else {
            panic!("expected one recording");
        };
        assert_eq!(
            recording.display_name,
            "Field test Morning Receiver morning.gtd"
        );
        assert_eq!(recording.filename, "morning.gtd");
        assert_eq!(recording.title.as_deref(), Some("Morning"));
        assert_eq!(recording.device.as_deref(), Some("Receiver"));
        assert_eq!(recording.time_range, Some(TimeRange::new(start, end)));
        assert_eq!(recording.track_count, 1);
        assert!(page.next_after.is_none());
    }

    #[test]
    fn a_recording_without_tracks_has_no_loaded_time_range() {
        let mut files = LoadedFiles::new();
        let request = ListRecordings {
            after: None,
            limit: NonZeroUsize::MIN,
        };
        let empty = request.execute(files.view(), "{filename}");
        assert!(empty.recordings.is_empty());
        assert!(empty.next_after.is_none());

        files.push(test_util::empty_file(), FileHistory::None);

        let listed = request.execute(files.view(), "{filename}");
        let recording = listed.recordings.first().expect("one recording");
        assert_eq!(recording.time_range, None);
        assert_eq!(recording.track_count, 0);
    }

    #[test]
    fn a_bounded_page_resumes_after_an_unloaded_cursor() {
        let mut files = LoadedFiles::new();
        for _ in 0..=MAX_RECORDINGS_PER_PAGE {
            files.push(test_util::empty_file(), FileHistory::None);
        }
        let view = files.view();
        let last = view
            .get(MAX_RECORDINGS_PER_PAGE)
            .expect("one more file")
            .id();
        let first = ListRecordings {
            after: None,
            limit: NonZeroUsize::MAX,
        }
        .execute(view, "{filename}");
        assert_eq!(first.recordings.len(), MAX_RECORDINGS_PER_PAGE);
        let cursor = first.next_after.expect("more recordings");
        let cursor_index = MAX_RECORDINGS_PER_PAGE - 1;
        assert_eq!(
            first.recordings.last().map(|recording| recording.id),
            Some(cursor)
        );
        assert!(files.remove_file(cursor_index).is_some());

        let view = files.view();
        assert!(view.entry_for_id(cursor).is_none());
        let remaining = ListRecordings {
            after: Some(cursor),
            limit: NonZeroUsize::MIN,
        }
        .execute(view, "{filename}");

        assert_eq!(remaining.recordings.len(), 1);
        assert_eq!(
            remaining.recordings.first().map(|recording| recording.id),
            Some(last)
        );
        assert!(remaining.next_after.is_none());
    }

    #[rstest]
    #[case::reprocess(|files: &mut LoadedFiles| {
        files.replace_file(1, gt_test_utils::segmented_recording(3), FileHistory::None);
    }, FileIdx::new(1))]
    #[case::remove_earlier_track(|files: &mut LoadedFiles| {
        if let Some(file) = files.get_mut(1) {
            file.tracks.remove(0);
        }
    }, FileIdx::new(1))]
    #[case::remove_earlier_recording(|files: &mut LoadedFiles| {
        assert!(files.remove_file(0).is_some());
    }, FileIdx::new(0))]
    fn a_session_track_ref_expires_when_loaded_tracks_change(
        #[case] change: fn(&mut LoadedFiles),
        #[case] new_file_index: FileIdx,
    ) {
        let mut files = LoadedFiles::new();
        files.push(test_util::empty_file(), FileHistory::None);
        files.push(gt_test_utils::segmented_recording(3), FileHistory::None);
        let view = files.view();
        let file_index = FileIdx::new(1);
        let track_index = TrackIdx::new(1);
        let address = TrackRef::new(file_index, track_index);
        assert!(
            view.session_track_ref(TrackRef::new(file_index, TrackIdx::new(3)))
                .is_none()
        );
        let stable_id = view.entry_for(file_index).expect("file is loaded").id();
        let reference = view.session_track_ref(address).expect("track is loaded");
        assert_eq!(view.resolve_session_track(reference), Some(address));

        change(&mut files);

        let current = files.view();
        assert_eq!(
            current.entry_for_id(stable_id).map(|entry| entry.fi),
            Some(new_file_index)
        );
        assert!(current.resolve_session_track(reference).is_none());
        let current_address = TrackRef::new(new_file_index, track_index);
        let current_reference = current
            .session_track_ref(current_address)
            .expect("track is loaded");
        assert_eq!(
            current.resolve_session_track(current_reference),
            Some(current_address)
        );
    }
}
