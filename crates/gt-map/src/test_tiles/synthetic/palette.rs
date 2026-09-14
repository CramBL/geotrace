//! Muted enough that a track's own colours stay legible over it. The two
//! backgrounds alternate so neighbouring tiles show their shared edge.

use egui::Color32;

pub const BACKGROUND_EVEN: Color32 = Color32::from_rgb(0xE4, 0xE2, 0xDB);
pub const BACKGROUND_ODD: Color32 = Color32::from_rgb(0xD6, 0xD4, 0xCB);
pub const GRID_LINE: Color32 = Color32::from_rgb(0xC2, 0xBF, 0xB4);
pub const BORDER: Color32 = Color32::from_rgb(0x93, 0x8F, 0x84);
pub const LABEL: Color32 = Color32::from_rgb(0x4B, 0x48, 0x41);
