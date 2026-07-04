use gt_types::{FileIdx, LoadedFile, TrackIdx, TrackRef};

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
    pub track_visible: bool,
    pub tpv_visible: bool,
    pub satellites_visible: bool,
    pub custom_markers_visible: bool,
    pub generated_markers_visible: bool,
    pub event_markers_visible: bool,
}

impl TrackVisibility {
    pub fn all_visible() -> Self {
        Self {
            enabled: true,
            track_visible: true,
            tpv_visible: true,
            satellites_visible: true,
            custom_markers_visible: true,
            generated_markers_visible: true,
            event_markers_visible: true,
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
