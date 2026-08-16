//! The GeoTrace query language: a small declarative pipeline for ad-hoc
//! analysis of loaded navigation data.
//!
//! ```text
//! points
//! | window 10
//! | where spread(heading) <= 10 deg
//!     and avg(accel) >= 0.3 m/s2
//!     and avg(velocity) > 30 km/h
//! | draw
//! | table time, velocity, heading, accel
//! ```
//!
//! The flow is [`parse`] → [`check`] → [`run`]. Parsing and checking report a
//! [`Diagnostic`] with a byte span for the editor to underline; a checked
//! query evaluates over [`MetricProvider`]s supplied by the caller and
//! returns matches as point-index ranges per track, plus a run summary.
//!
//! This crate is pure language and evaluation - no data loading, no UI, no
//! rendering.

pub mod ast;
mod check;
pub mod construct;
mod dimension;
mod eval;
mod fmt;
pub mod lexer;
mod metric;
mod parser;
mod pipeline;
mod position;
mod unit;

pub use ast::{ParamName, Query, Span};
pub use check::{ChannelConflict, ChannelInfo, ChannelSchema, CheckedQuery, Params, Window, check};
pub use construct::{Construct, ConstructKind, catalog};
pub use dimension::Dimension;
pub use eval::{
    ChannelSamples, ChannelTimeline, MetricProvider, RunOutput, RunSummary, TrackInput,
    TrackMatches, derived_accel, run, run_cancellable,
};
pub use metric::{Quantity, QueryMetric};
pub use parser::parse;
pub use pipeline::{DrawContribution, PipelineOutput, QueryOutput, run_pipeline};
pub use position::{
    ChannelCompletions, ChannelSuggestion, CompletionTrigger, Completions, channel_at,
    channel_completions_at, completions_at, construct_at,
};
pub use unit::Unit;

/// A parse or type error: what went wrong, where, and optionally how to fix
/// it. Rendered by the editor as an underline plus message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diagnostic {
    pub span: Span,
    pub message: String,
    pub help: Option<String>,
}

impl Diagnostic {
    /// An error with no suggestion.
    pub(crate) fn new(span: Span, message: impl Into<String>) -> Self {
        Self {
            span,
            message: message.into(),
            help: None,
        }
    }

    /// An error whose fix goes in `help` (shown as a separate "Hint:" line)
    /// rather than tacked onto the message.
    pub(crate) fn with_hint(
        span: Span,
        message: impl Into<String>,
        help: impl Into<String>,
    ) -> Self {
        Self {
            span,
            message: message.into(),
            help: Some(help.into()),
        }
    }
}

#[cfg(test)]
mod tests;
