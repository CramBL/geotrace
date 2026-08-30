use std::fmt;

/// A structured data quality warning produced when loading a recording file.
#[derive(Debug, Clone)]
pub struct LoadWarning {
    /// Number of instances of this issue in the file.
    pub count: u32,
    /// Short description of the issue (e.g. "satellite(s) with PRN 0").
    pub issue: String,
    /// Explanation of why the issue matters and how to resolve it.
    pub description: String,
}

/// How many entries a warning names before it only counts the rest.
const MAX_LISTED_ENTRIES: usize = 5;

/// Renders the first [`MAX_LISTED_ENTRIES`] of `entries`, followed by how many
/// more there are.
pub fn first_few_listed<T: fmt::Display>(entries: &[T]) -> String {
    let listed = entries
        .iter()
        .take(MAX_LISTED_ENTRIES)
        .map(T::to_string)
        .collect::<Vec<_>>()
        .join(", ");
    match entries.len().saturating_sub(MAX_LISTED_ENTRIES) {
        0 => listed,
        rest => format!("{listed}, and {rest} more"),
    }
}

/// The wording of a warning about what a layer of the loading pipeline altered:
/// `issue` completes the "<count> …" line, `consequence` follows the listed
/// entries in the description.
///
/// The SDK's own warnings describe the file itself, and these sit beside them:
/// a file that repeats a satellite raises one warning about the file and one
/// about what the app made of it.
#[derive(Clone, Copy)]
pub struct AlterationWording {
    pub issue: &'static str,
    pub consequence: &'static str,
}

impl AlterationWording {
    pub fn load_warning<T: fmt::Display>(self, entries: &[T]) -> Option<LoadWarning> {
        (!entries.is_empty()).then(|| LoadWarning {
            count: u32::try_from(entries.len()).unwrap_or(u32::MAX),
            issue: self.issue.to_owned(),
            description: format!("{}. {}", first_few_listed(entries), self.consequence),
        })
    }
}
