//! The session model of the log layer: the logs loaded alongside the
//! recordings, and the one recording each of them is anchored to.
//!
//! A log is a layer over time: the session keeps its full text, and every entry
//! carries the position of the fix nearest in time in the one recording the
//! user pointed the log at. Association reads the fixes of that recording
//! alone, whatever else is loaded.
//!
//! Which parts of a log the viewer shows and the map draws is decided by its
//! filter stack: the live filter the user is typing, and the chips added from
//! it.

pub use anchor::RecordingKey;
pub use association::{AssociationCandidate, AssociationCandidates};
pub use attachment::{
    LogAttachmentRef, SessionLogAttachments, stored_year_resolution, year_resolution_of_stored,
};
pub use attachment_settings::{
    AttachmentSettingsCompletion, AttachmentSettingsRevision, AttachmentSettingsState,
    AttachmentSettingsStatus, AttachmentSettingsWrite,
};
pub use filter::{
    ClockTicks, DayDivider, EntryMatches, FilterChip, FilterChipId, FilterEffect, FilterGroup,
    FilterGroupId, FilterGroupOperator, FilterPattern, FilterScope, FilterStack,
    InvalidFilterPattern, LAYER_COLOR_SLOT_COUNT, LayerColorSlot, LayerColorSlots, LiveFilterDraft,
    PreparedFilterCondition, TableFilterApplication, TableFilterStatus, TimestampTick,
    VisibleEntries,
};
pub use loaded_log::{
    LoadedLog, LoadedLogEditor, LoadedLogs, LogContextOrigin, LogPushOutcome, LogSaveOutcome,
    PositionSourceState,
};

pub use log_save::{
    InitialLogSaveOutcome, InitialLogSaveStatus, LogCapabilities, LogOperationError,
    LogSaveCompletion, LogSaveReceipt, LogSaveRequest, PositionSourceSelection,
};

pub use source_jump::{InvalidLogJump, LogJumpRequest, LogJumpTarget, UnresolvedLogJump};

pub use source_search::{SourceOccurrence, SourceSearch, SourceSearchScope, SourceSearchStatus};

mod anchor;
mod association;
mod attachment;
mod attachment_settings;
mod filter;
mod loaded_log;
mod log_save;
mod source_jump;
mod source_search;
mod test_util;
