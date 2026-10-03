use super::matches::{self, EntryMatches};

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum FilterGroupOperator {
    #[default]
    All,
    Any,
}

impl FilterGroupOperator {
    /// Returns `None` when the group has no participating conditions.
    pub fn compose(self, conditions: &[&EntryMatches]) -> Option<Vec<usize>> {
        if conditions.is_empty() {
            return None;
        }
        Some(match self {
            Self::All => matches::intersecting_entry_indices(conditions),
            Self::Any => matches::union_entry_indices(conditions),
        })
    }
}
