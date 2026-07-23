use gt_types::{DataCategory, DataCategorySet, FileIdx, LoadedFile, TrackIdx, TrackRef};

#[derive(Debug, Clone)]
pub struct TrackDataVisibility {
    pub files: Vec<FileVisibility>,
}

#[derive(Debug, Clone)]
pub struct FileVisibility {
    pub enabled: bool,
    pub tracks: Vec<TrackVisibility>,
}

#[derive(Debug, Clone, Copy)]
pub struct TrackVisibility {
    pub enabled: bool,
    /// Per-category tree toggles. `enabled` is separate: it gates the whole
    /// track, while this only hides individual element categories.
    categories: DataCategorySet,
}

impl TrackVisibility {
    /// This track's tree toggle for the given element category - the
    /// single mapping every renderer and count consults, so the flag a
    /// category answers to cannot drift between consumers.
    pub fn category_visible(self, category: DataCategory) -> bool {
        self.categories.contains(category)
    }

    /// Show or hide the given element category for this track.
    pub fn set_category_visible(&mut self, category: DataCategory, visible: bool) {
        self.categories.set(category, visible);
    }

    pub fn all_visible() -> Self {
        Self {
            enabled: true,
            categories: DataCategorySet::all(),
        }
    }
}

impl TrackDataVisibility {
    pub fn from_loaded(files: &[LoadedFile]) -> Self {
        Self {
            files: files
                .iter()
                .map(|f| FileVisibility {
                    enabled: true,
                    tracks: f
                        .tracks
                        .iter()
                        .map(|_| TrackVisibility::all_visible())
                        .collect(),
                })
                .collect(),
        }
    }

    /// Enable or disable every file and track at once.
    pub fn set_all_enabled(&mut self, enabled: bool) {
        for file in &mut self.files {
            file.enabled = enabled;
            for track in &mut file.tracks {
                track.enabled = enabled;
            }
        }
    }

    /// Whether the given track (and its file) is enabled.
    pub fn track_enabled(&self, track_ref: TrackRef) -> bool {
        track_ref
            .fi
            .get(&self.files)
            .is_some_and(|f| f.enabled && track_ref.index.get(&f.tracks).is_some_and(|t| t.enabled))
    }

    /// Whether the given track's line is shown on the map: its file and the
    /// track are enabled, and the track-line toggle is on. The predicate the
    /// snapped-track rendering and the snap queue's visibility priority use.
    pub fn track_shown(&self, track_ref: TrackRef) -> bool {
        track_ref.fi.get(&self.files).is_some_and(|f| {
            f.enabled
                && track_ref
                    .index
                    .get(&f.tracks)
                    .is_some_and(|t| t.enabled && t.category_visible(DataCategory::Track))
        })
    }

    /// Show only the given file. Hide all others. Trip visibility within files
    /// is preserved so that re-enabling a file restores its previous state.
    pub fn show_only_file(&mut self, fi: FileIdx) {
        for (i, file) in self.files.iter_mut().enumerate() {
            file.enabled = FileIdx::new(i) == fi;
        }
    }

    /// Show only the given track (and its parent file). Hide everything else.
    pub fn show_only_track(&mut self, track: TrackRef) {
        for (i, file) in self.files.iter_mut().enumerate() {
            if FileIdx::new(i) == track.fi {
                file.enabled = true;
                for (j, t) in file.tracks.iter_mut().enumerate() {
                    t.enabled = TrackIdx::new(j) == track.index;
                }
            } else {
                file.enabled = false;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[rstest::rstest]
    #[case(DataCategory::Track)]
    #[case(DataCategory::Tpv)]
    #[case(DataCategory::SatelliteReport)]
    #[case(DataCategory::CustomMarker)]
    #[case(DataCategory::GeneratedMarker)]
    #[case(DataCategory::EventMarker)]
    fn category_visible_reads_exactly_its_flag(#[case] category: DataCategory) {
        let mut tv = TrackVisibility::all_visible();
        assert!(tv.category_visible(category));
        tv.set_category_visible(category, false);
        assert!(!tv.category_visible(category));
        // Exactly one category flag changed: every other category still on.
        let others = [
            DataCategory::Track,
            DataCategory::Tpv,
            DataCategory::SatelliteReport,
            DataCategory::CustomMarker,
            DataCategory::GeneratedMarker,
            DataCategory::EventMarker,
        ]
        .into_iter()
        .filter(|&c| c != category)
        .all(|c| tv.category_visible(c));
        assert!(others);
    }

    /// `track_shown` requires the file, the track, and the track-line
    /// toggle; any one of them off hides the track. Out-of-range refs are
    /// simply not shown.
    #[rstest::rstest]
    #[case::all_on(true, true, true, true)]
    #[case::file_disabled(false, true, true, false)]
    #[case::track_disabled(true, false, true, false)]
    #[case::trackline_hidden(true, true, false, false)]
    fn track_shown_needs_file_track_and_line(
        #[case] file_enabled: bool,
        #[case] track_enabled: bool,
        #[case] track_visible: bool,
        #[case] expected: bool,
    ) {
        let mut tv = TrackVisibility::all_visible();
        tv.enabled = track_enabled;
        tv.set_category_visible(DataCategory::Track, track_visible);
        let vis = TrackDataVisibility {
            files: vec![FileVisibility {
                enabled: file_enabled,
                tracks: vec![tv],
            }],
        };
        let track_ref = TrackRef::new(FileIdx::new(0), TrackIdx::new(0));
        assert_eq!(vis.track_shown(track_ref), expected);
        assert!(
            !vis.track_shown(TrackRef::new(FileIdx::new(1), TrackIdx::new(0))),
            "an out-of-range ref is never shown"
        );
    }
}
