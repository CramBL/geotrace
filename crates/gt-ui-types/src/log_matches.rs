//! What the loaded logs' filters put on the map: the positions each filter
//! selected, and the colour that filter draws them in.
//!
//! A match takes its position from the recording its log is associated
//! against, a log being a layer over time: an entry with no fix inside the
//! association window has no position and draws nothing.

use gt_types::MercPoint;

/// The colour a group of log matches draws in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogMatchColor {
    /// The colour reserved for the filter being typed.
    LiveFilter,

    /// A layer chip's palette slot. `shared` marks a slot held by more than
    /// one chip, which the map draws with a doubled outline.
    LayerSlot { index: usize, shared: bool },
}

/// The matches of one filter, in file order.
#[derive(Debug, Clone, PartialEq)]
pub struct LogMatchLayer {
    pub color: LogMatchColor,
    pub positions: Vec<MercPoint>,
}

/// Every loaded log's map contribution, in draw order: later layers draw over
/// earlier ones.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct LogMatches {
    layers: Vec<LogMatchLayer>,
}

impl LogMatches {
    pub fn from_layers(layers: Vec<LogMatchLayer>) -> Self {
        Self { layers }
    }

    pub fn layers(&self) -> &[LogMatchLayer] {
        &self.layers
    }

    pub fn is_empty(&self) -> bool {
        self.layers.iter().all(|layer| layer.positions.is_empty())
    }

    /// Positions across every layer, which the display toggle counts. An entry
    /// matched by two filters counts once per filter: each draws its own
    /// hexagon.
    pub fn position_count(&self) -> usize {
        self.layers.iter().map(|layer| layer.positions.len()).sum()
    }
}
