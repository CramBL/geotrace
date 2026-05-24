use crate::trip::LoadedFile;

#[derive(Debug, Clone)]
pub struct TripDataVisibility {
    pub files: Vec<FileVisibility>,
}

#[derive(Debug, Clone)]
pub struct FileVisibility {
    pub enabled: bool,
    pub trips: Vec<TripVisibility>,
}

#[derive(Debug, Clone, Copy)]
pub struct TripVisibility {
    pub enabled: bool,
    pub track_visible: bool,
    pub tpv_visible: bool,
    pub satellites_visible: bool,
    pub custom_markers_visible: bool,
    pub generated_markers_visible: bool,
}

impl TripVisibility {
    pub fn all_visible() -> Self {
        Self {
            enabled: true,
            track_visible: true,
            tpv_visible: true,
            satellites_visible: true,
            custom_markers_visible: true,
            generated_markers_visible: true,
        }
    }
}

impl TripDataVisibility {
    pub fn from_loaded(files: &[LoadedFile]) -> Self {
        Self {
            files: files
                .iter()
                .map(|f| FileVisibility {
                    enabled: true,
                    trips: f
                        .trips
                        .iter()
                        .map(|_| TripVisibility::all_visible())
                        .collect(),
                })
                .collect(),
        }
    }

    /// Enable or disable every file and trip at once.
    pub fn set_all_enabled(&mut self, enabled: bool) {
        for file in &mut self.files {
            file.enabled = enabled;
            for trip in &mut file.trips {
                trip.enabled = enabled;
            }
        }
    }

    /// Show only the given file; hide all others. Trip visibility within files
    /// is preserved so that re-enabling a file restores its previous state.
    pub fn show_only_file(&mut self, fi: usize) {
        for (i, file) in self.files.iter_mut().enumerate() {
            file.enabled = i == fi;
        }
    }

    /// Show only the given trip (and its parent file); hide everything else.
    pub fn show_only_trip(&mut self, fi: usize, ti: usize) {
        for (i, file) in self.files.iter_mut().enumerate() {
            if i == fi {
                file.enabled = true;
                for (j, trip) in file.trips.iter_mut().enumerate() {
                    trip.enabled = j == ti;
                }
            } else {
                file.enabled = false;
            }
        }
    }
}
