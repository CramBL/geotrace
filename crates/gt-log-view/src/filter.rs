//! The filter engine: what the viewer's live filter and chips select out of a
//! log, and the parallel scan that finds it.
//!
//! A filter matches an entry's message, never its timestamp, and never a
//! structural line: those are not entries. Plain filters split into
//! whitespace-separated terms that must all occur in the message. A regex
//! filter matches the message as one pattern.

mod clock_ticks;
mod matches;
mod pattern;
mod query;
mod slots;
mod stack;

pub use clock_ticks::{ClockTicks, DayDivider, TimestampTickLevel};
pub use matches::EntryMatches;
pub use pattern::{FilterPattern, InvalidFilterPattern};
pub use slots::{LAYER_COLOR_SLOT_COUNT, LayerColorSlot, LayerColorSlots};
pub use stack::{FilterChip, FilterChipId, FilterChipMode, FilterStack, VisibleEntries};
