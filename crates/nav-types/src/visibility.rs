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
}
