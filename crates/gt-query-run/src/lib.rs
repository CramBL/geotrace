//! Running the query language over the loaded data.
//!
//! [`gt_query`] is pure language and evaluation: it parses, checks, and
//! evaluates a query against caller-supplied [`gt_query::MetricProvider`]s.
//! This crate is the layer above it, holding everything a run over loaded
//! `.gtd` files needs and nothing about how it is displayed:
//!
//! - the editor text split into blank-line-separated queries and checked
//!   ([`QueryChunk`], [`check_all`]),
//! - the channel schema gathered from the loaded files ([`schema_from_files`]),
//! - the providers the evaluator reads points, derived series and channels
//!   through ([`TrackProvider`], [`SliceProvider`]),
//! - one prepared, cancellable, synchronous run ([`PreparedRun`]),
//! - the projection of its output into map matches and panel rows
//!   ([`RunResults`]),
//! - and the state machine tying those together ([`QuerySession`]).
//!
//! A caller drives a whole run without a UI, threads, or a GPU:
//!
//! ```ignore
//! let mut session = QuerySession::new();
//! session.set_text("points | where velocity > 30 km/h".to_owned());
//! session.sync_checks(&schema_from_files(files));
//! let prepared = session.start_run(inputs).expect("the query checks");
//! session.finish_run(prepared.execute());
//! let matches = session.matches();
//! ```
//!
//! The app layer adds the parts this crate deliberately leaves out: the egui
//! editor and results panel, and the worker thread that calls
//! [`PreparedRun::execute`] off the UI thread.

mod check;
mod fingerprint;
mod provider;
mod results;
mod run;
mod schema;
mod session;
#[cfg(test)]
mod test_fixtures;

pub use check::{QueryChunk, analysis_context, check_all, check_text, split_queries};
pub use fingerprint::{JammingValues, RunFingerprint, RunInputs, SnapErrorValues};
pub use provider::{SliceProvider, TimeFilteredPoints, TrackProvider, TrackQueryData};
pub use results::{
    ChannelResults, ChannelTrackResult, HiddenPoints, MatchValues, PanelQuery, PointsResults,
    QuerySummary, RunResults, TrackMatchValues,
};
pub use run::{PreparedRun, RunHandle, RunKind, RunOutcome};
pub use schema::schema_from_files;
pub use session::{CheckRefresh, QueryProgress, QuerySession};

/// Microseconds per second, for converting between a channel sample's
/// `timestamp_micros` and the evaluator's seconds.
pub const MICROS_PER_SEC: f64 = 1_000_000.0;
