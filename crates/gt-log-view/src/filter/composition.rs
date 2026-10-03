use super::matches::EntryMatches;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum FilterGroupOperator {
    #[default]
    All,
    Any,
}

impl FilterGroupOperator {
    /// Returns `None` when the group has no participating conditions.
    pub fn compose(self, conditions: &[&EntryMatches]) -> Option<EntryMatches> {
        if conditions.is_empty() {
            return None;
        }
        Some(match self {
            Self::All => EntryMatches::intersection(conditions),
            Self::Any => EntryMatches::union(conditions),
        })
    }
}
