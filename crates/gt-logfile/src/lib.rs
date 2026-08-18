//! Reading journald/syslog-style logs recorded alongside a GPS session.
//!
//! Parsing is lenient: the head of the log decides its timestamp format, and
//! every line that does not carry that format is counted and skipped. The log
//! is kept as one shared text buffer with a compact index over its lines.

mod associate;
mod format;
mod parse;

pub use associate::associate_position;
pub use format::{LogFormat, detect_format, infer_year};
pub use parse::{LogEntry, LogParseError, ParsedLog, parse_log};
