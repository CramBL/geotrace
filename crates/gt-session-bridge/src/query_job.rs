use gt_query_run::{QuerySummary, RunResults};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct QueryJobId {
    pub session: Uuid,
    pub id: Uuid,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct QueryJobRequest {
    pub job: QueryJobId,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct QueryJobStatus {
    pub job: QueryJobId,
    #[serde(flatten)]
    pub state: QueryJobState,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum QueryJobState {
    Busy,
    Cancelled,
    Cancelling,
    Completed {
        queries: Vec<QueryCounts>,
    },
    Failed,
    Preparing,
    Running {
        tracks_prepared: usize,
        track_total: usize,
    },
    Stale,
}

impl QueryJobState {
    pub fn of_results(results: &RunResults) -> Self {
        if results.stale() {
            return Self::Stale;
        }
        let queries = match results {
            RunResults::Channel(channel) => vec![QueryCounts::of(&channel.summary)],
            RunResults::Points(points) => points
                .queries
                .iter()
                .map(|query| QueryCounts::of(&query.summary))
                .collect(),
        };
        Self::Completed { queries }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct QueryCounts {
    pub matches: usize,
    pub tracks_with_matches: usize,
    pub skipped: usize,
}

impl QueryCounts {
    fn of(summary: &QuerySummary) -> Self {
        Self {
            matches: summary.match_count,
            tracks_with_matches: summary.tracks_with_matches,
            skipped: summary.skipped,
        }
    }
}
