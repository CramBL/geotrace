//! What an archive takes up on disk and how much of the calendar it holds.

use std::path::Path;

use chrono::NaiveDate;

/// The oldest and newest day an archive holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ArchivedDaySpan {
    pub oldest: NaiveDate,
    pub newest: NaiveDate,
}

impl ArchivedDaySpan {
    /// The span covering both, for a total across archives.
    pub fn joined(self, other: Self) -> Self {
        Self {
            oldest: self.oldest.min(other.oldest),
            newest: self.newest.max(other.newest),
        }
    }
}

/// What one archive holds: its size on disk, and the days it is keyed by.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ArchiveUsage {
    /// Size of the archive file, or [`None`] where its metadata could not be
    /// read.
    pub bytes: Option<u64>,
    pub days: usize,
    /// [`None`] while the archive holds no day.
    pub span: Option<ArchivedDaySpan>,
}

impl ArchiveUsage {
    /// Measure the file at `path` against the `days` it is keyed by.
    pub fn measure(path: &Path, days: impl IntoIterator<Item = NaiveDate>) -> Self {
        let mut usage = Self {
            bytes: std::fs::metadata(path).map(|file| file.len()).ok(),
            ..Self::default()
        };
        for day in days {
            usage.days += 1;
            usage.span = Some(match usage.span {
                Some(span) => span.joined(ArchivedDaySpan {
                    oldest: day,
                    newest: day,
                }),
                None => ArchivedDaySpan {
                    oldest: day,
                    newest: day,
                },
            });
        }
        usage
    }

    /// The archives added up. A size that could not be read leaves the total
    /// unknown.
    pub fn total(usages: impl IntoIterator<Item = Self>) -> Self {
        let mut total = Self {
            bytes: Some(0),
            ..Self::default()
        };
        for usage in usages {
            total.bytes = total.bytes.zip(usage.bytes).map(|(sum, add)| sum + add);
            total.days += usage.days;
            total.span = match (total.span, usage.span) {
                (Some(span), Some(other)) => Some(span.joined(other)),
                (span, other) => span.or(other),
            };
        }
        total
    }

    pub const fn is_empty(&self) -> bool {
        self.days == 0
    }
}
