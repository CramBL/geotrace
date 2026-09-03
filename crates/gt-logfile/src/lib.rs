//! Reading journald/syslog-style logs recorded alongside a GPS session.
//!
//! Parsing is lenient: the head of the log decides its timestamp format, and
//! every non-empty line is kept, as an entry the line timestamped itself, as a
//! recognized structural line, or as an entry timestamped from its neighbours.
//! The log is kept as one shared text buffer with a compact index over its
//! lines, in the order the file wrote them.

mod associate;
mod format;
mod parse;
mod pool;
mod recognise;
mod session;
mod structure;
mod summary;
mod text;

pub use associate::{EntryPlacement, associate_entries, associate_position};
pub use format::{LogFormat, detect_format, infer_year};
pub use parse::{LogEntry, LogParseError, ParsedLog, TextSlice, TimestampKind, parse_log};
pub use pool::log_worker_pool;
pub use recognise::{
    HostnameColumn, LogLevelKind, RecognisedLevel, RecognisedMessage, RecognisedService,
};
pub use session::{AnchoredBounds, BootSession, OrderAnomaly};
pub use structure::{StructuralLine, StructuralLineKind};
pub use summary::{EntryCountMismatch, ServiceCount, ServiceIssueCounts, SummaryBlock};
pub use text::LogText;
