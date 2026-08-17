//! The hosts a day's maps are fetched from, in the order they are tried.
//!
//! Every mirror serves JPL's directory layout under its base URL:
//! `IONEX_final/yYYYY/JPLG<ddd>0.<yy>I.gz` and
//! `IONEX_rapid/JPLR<ddd>0.<yy>I.gz`, which is what a self-hosted or
//! institutional copy of the archive holds. Multi-producer file naming and
//! compression are left for later: CODE Bern's `CODG` files are LZW-compressed
//! `.Z`, a format the gzip decoder does not read.

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::{DEFAULT_BASE_URL, IonexProduct};

/// Base URL of one host serving the archive, without a trailing slash.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct MirrorBaseUrl(String);

impl MirrorBaseUrl {
    pub fn new(url: impl Into<String>) -> Self {
        Self(url.into())
    }
}

impl AsRef<str> for MirrorBaseUrl {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for MirrorBaseUrl {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// The mirrors a day is requested from, in the order they are tried.
///
/// Never empty: a list with no host to fetch from is not a configuration, so
/// [`MirrorList::remove`] keeps the last entry and [`MirrorList::new`] returns
/// `None` for an empty one.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct MirrorList(Vec<MirrorBaseUrl>);

impl Default for MirrorList {
    fn default() -> Self {
        Self::single(MirrorBaseUrl::new(DEFAULT_BASE_URL))
    }
}

impl MirrorList {
    pub fn single(mirror: MirrorBaseUrl) -> Self {
        Self(vec![mirror])
    }

    pub fn new(mirrors: Vec<MirrorBaseUrl>) -> Option<Self> {
        (!mirrors.is_empty()).then_some(Self(mirrors))
    }

    pub fn as_slice(&self) -> &[MirrorBaseUrl] {
        &self.0
    }

    /// Add `mirror` after the ones already listed.
    pub fn add(&mut self, mirror: MirrorBaseUrl) {
        self.0.push(mirror);
    }

    /// Point the entry at `index` at another host.
    pub fn replace(&mut self, index: usize, mirror: MirrorBaseUrl) {
        if let Some(entry) = self.0.get_mut(index) {
            *entry = mirror;
        }
    }

    /// The removed mirror, or `None` for the only remaining one.
    pub fn remove(&mut self, index: usize) -> Option<MirrorBaseUrl> {
        (self.0.len() > 1 && index < self.0.len()).then(|| self.0.remove(index))
    }

    /// Try the mirror at `index` one place earlier.
    pub fn move_up(&mut self, index: usize) {
        if let Some(earlier) = index.checked_sub(1)
            && index < self.0.len()
        {
            self.0.swap(index, earlier);
        }
    }

    /// Try the mirror at `index` one place later.
    pub fn move_down(&mut self, index: usize) {
        let later = index.saturating_add(1);
        if later < self.0.len() {
            self.0.swap(index, later);
        }
    }
}

/// What one mirror returned for one product's file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MirrorAttempt {
    pub mirror: MirrorBaseUrl,
    pub product: IonexProduct,
    pub outcome: MirrorOutcome,
}

impl MirrorAttempt {
    /// The attempt as a failure, or `None` when the mirror holds no such file.
    pub fn failure(&self) -> Option<MirrorFailure> {
        match &self.outcome {
            MirrorOutcome::NoFile => None,
            MirrorOutcome::Failed(detail) => Some(MirrorFailure {
                mirror: self.mirror.clone(),
                detail: detail.clone(),
            }),
        }
    }
}

/// Why a mirror did not serve a day's file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MirrorOutcome {
    /// The mirror holds no file of this product for the day.
    NoFile,
    /// The request did not complete, or the file the mirror served could not
    /// be read.
    Failed(String),
}

/// One mirror's failure to serve a day's file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MirrorFailure {
    pub mirror: MirrorBaseUrl,
    pub detail: String,
}

impl fmt::Display for MirrorFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.mirror, self.detail)
    }
}

/// No mirror served one product's file for a day, and at least one of the
/// attempts failed outright.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProductFetchFailure {
    pub product: IonexProduct,
    pub mirrors: Vec<MirrorFailure>,
}

impl fmt::Display for ProductFetchFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: ", self.product)?;
        for (position, failure) in self.mirrors.iter().enumerate() {
            if position > 0 {
                f.write_str(", ")?;
            }
            write!(f, "{failure}")?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use rstest::rstest;

    use super::*;

    fn mirror(host: &str) -> MirrorBaseUrl {
        MirrorBaseUrl::new(format!("https://{host}.example"))
    }

    fn list(hosts: &[&str]) -> MirrorList {
        MirrorList::new(hosts.iter().map(|host| mirror(host)).collect()).expect("a named host")
    }

    fn hosts(mirrors: &MirrorList) -> Vec<String> {
        mirrors
            .as_slice()
            .iter()
            .map(MirrorBaseUrl::to_string)
            .collect()
    }

    #[test]
    fn the_default_list_holds_the_publishing_host() {
        assert_eq!(
            MirrorList::default().as_slice(),
            [MirrorBaseUrl::new(DEFAULT_BASE_URL)]
        );
    }

    #[test]
    fn a_list_without_a_host_to_fetch_from_is_not_a_list() {
        assert_eq!(MirrorList::new(Vec::new()), None);
    }

    /// The list keeps its last entry, so the fetch always has a host to try.
    #[test]
    fn the_only_remaining_mirror_is_never_removed() {
        let mut mirrors = list(&["first"]);
        assert_eq!(mirrors.remove(0), None);
        assert_eq!(hosts(&mirrors), ["https://first.example"]);
    }

    #[rstest]
    #[case::the_first(0, &["https://second.example", "https://third.example"])]
    #[case::the_last(2, &["https://first.example", "https://second.example"])]
    #[case::past_the_end(3, &["https://first.example", "https://second.example", "https://third.example"])]
    fn removing_a_mirror_keeps_the_order_of_the_rest(
        #[case] index: usize,
        #[case] expected: &[&str],
    ) {
        let mut mirrors = list(&["first", "second", "third"]);
        mirrors.remove(index);
        assert_eq!(hosts(&mirrors), expected);
    }

    /// A move at either end of the list leaves the order as it was.
    #[rstest]
    #[case::up_from_the_middle(MirrorList::move_up, 1, &["https://second.example", "https://first.example", "https://third.example"])]
    #[case::up_from_the_first(MirrorList::move_up, 0, &["https://first.example", "https://second.example", "https://third.example"])]
    #[case::down_from_the_middle(MirrorList::move_down, 1, &["https://first.example", "https://third.example", "https://second.example"])]
    #[case::down_from_the_last(MirrorList::move_down, 2, &["https://first.example", "https://second.example", "https://third.example"])]
    fn moving_a_mirror_swaps_it_with_its_neighbor(
        #[case] move_mirror: fn(&mut MirrorList, usize),
        #[case] index: usize,
        #[case] expected: &[&str],
    ) {
        let mut mirrors = list(&["first", "second", "third"]);
        move_mirror(&mut mirrors, index);
        assert_eq!(hosts(&mirrors), expected);
    }

    #[test]
    fn an_edited_mirror_keeps_its_place_in_the_order() {
        let mut mirrors = list(&["first", "second"]);
        mirrors.replace(0, mirror("edited"));
        assert_eq!(
            hosts(&mirrors),
            ["https://edited.example", "https://second.example"]
        );
    }

    /// The failure names every mirror that was tried for one product, in the
    /// order they were tried, under that product.
    #[test]
    fn a_product_failure_reads_as_the_product_and_each_mirrors_reason() {
        let failure = ProductFetchFailure {
            product: IonexProduct::Final,
            mirrors: vec![
                MirrorFailure {
                    mirror: mirror("first"),
                    detail: "HTTP 503 Service Unavailable".to_owned(),
                },
                MirrorFailure {
                    mirror: mirror("second"),
                    detail: "connection refused".to_owned(),
                },
            ],
        };

        assert_eq!(
            failure.to_string(),
            "final: https://first.example: HTTP 503 Service Unavailable, \
             https://second.example: connection refused"
        );
    }

    #[rstest]
    #[case::a_mirror_without_the_file(MirrorOutcome::NoFile, None)]
    #[case::a_failure(MirrorOutcome::Failed("HTTP 500".to_owned()), Some("https://first.example: HTTP 500".to_owned()))]
    fn only_a_failed_attempt_reads_as_a_failure(
        #[case] outcome: MirrorOutcome,
        #[case] expected: Option<String>,
    ) {
        let attempt = MirrorAttempt {
            mirror: mirror("first"),
            product: IonexProduct::Final,
            outcome,
        };
        assert_eq!(
            attempt.failure().map(|failure| failure.to_string()),
            expected
        );
    }
}
