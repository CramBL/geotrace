use std::fmt;

use gt_query_run::{QueryChunk, RunResults};

/// What became of the last run the scenario asked for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunAttempt {
    /// The run evaluated and its results are current.
    Completed,
    /// The session refused to start it: a failing or empty editor, or a channel
    /// source mixed with other queries.
    Refused,
}

impl fmt::Display for RunAttempt {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Completed => f.write_str("completed"),
            Self::Refused => f.write_str("refused"),
        }
    }
}

/// What the query window says about the editor text and the last run: how the
/// text split into queries, how each one checked, and the per-query summaries.
///
/// The counterpart to [`crate::MapPicture`]: this is the panel's account, that
/// one is the map's. Borrowed straight from the session, so it renders the
/// session's own data rather than a copy of it.
pub struct PanelView<'a> {
    pub chunks: &'a [QueryChunk],
    pub run: Option<RunAttempt>,
    pub results: Option<&'a RunResults>,
}

impl fmt::Display for PanelView<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "chunks: {}", self.chunks.len())?;
        for chunk in self.chunks {
            let range = &chunk.range;
            match &chunk.result {
                Ok(_) => writeln!(f, "  {}..{} ok", range.start, range.end)?,
                Err(diagnostic) => writeln!(
                    f,
                    "  {}..{} error: {}",
                    range.start, range.end, diagnostic.message
                )?,
            }
        }
        match self.run {
            Some(run) => writeln!(f, "run: {run}")?,
            None => writeln!(f, "run: none")?,
        }
        match self.results {
            Some(RunResults::Points(points)) => {
                for query in &points.queries {
                    writeln!(f, "  {}", query.summary)?;
                }
            }
            Some(RunResults::Channel(channel)) => writeln!(f, "  {}", channel.summary)?,
            None => {}
        }
        if self.results.is_some_and(RunResults::stale) {
            writeln!(f, "stale")?;
        }
        Ok(())
    }
}
