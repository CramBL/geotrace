//! The log match under the cursor, on either side of the cross-highlight
//! between the map and the log viewer.

use gt_types::MercPoint;

use crate::log_matches::{LoadedLogId, LogMatchColor};

/// What the cursor is on, in both directions: the hexagon it is over on the
/// map, and the viewer row it is over.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct LogMatchHover {
    /// The hexagon under the cursor. The map writes it while it draws, and the
    /// viewer - which draws after the map - highlights the rows of the entries
    /// it stands for.
    pub glyph: Option<HoveredLogGlyph>,

    /// Where the viewer row under the cursor was recorded. The viewer writes
    /// it while it draws, and the map reads it one frame later, ringing that
    /// position.
    pub row_position: Option<MercPoint>,
}

/// The hexagon under the cursor on the map.
#[derive(Debug, Clone, PartialEq)]
pub struct HoveredLogGlyph {
    pub log: LoadedLogId,
    pub color: LogMatchColor,

    /// Indices into `ParsedLog::entries` of the hovered log, ascending.
    pub entry_indices: Vec<usize>,
}

impl HoveredLogGlyph {
    pub fn covers(&self, log: LoadedLogId, entry_index: usize) -> bool {
        self.log == log && self.entry_indices.binary_search(&entry_index).is_ok()
    }
}
