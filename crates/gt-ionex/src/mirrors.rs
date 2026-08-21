//! The hosts a day's maps are fetched from, in the order they are tried.
//!
//! A mirror serves one of two layouts. [`MirrorLayout::Jpl`] is the publisher's
//! own directory tree, `IONEX_final/yYYYY/JPLG<ddd>0.<yy>I.gz` and
//! `IONEX_rapid/JPLR<ddd>0.<yy>I.gz`, which is what a self-hosted or
//! institutional copy of that archive holds. [`MirrorLayout::Cddis`] is NASA's
//! archive, addressed by [`crate::cddis`] and served to callers holding an
//! Earthdata token.

use std::fmt;

use chrono::NaiveDate;
use serde::{Deserialize, Serialize};

use crate::{DEFAULT_BASE_URL, IonexProduct, cddis, text};

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

/// Which archive a mirror serves, which decides what a day's file is named
/// and whether the request carries a token.
#[derive(
    Debug,
    Clone,
    Copy,
    Default,
    PartialEq,
    Eq,
    Hash,
    Serialize,
    Deserialize,
    strum::Display,
    strum::EnumIter,
)]
#[serde(rename_all = "snake_case")]
pub enum MirrorLayout {
    #[default]
    #[strum(serialize = "JPL")]
    Jpl,
    #[strum(serialize = "CDDIS")]
    Cddis,
}

impl MirrorLayout {
    /// Whether a request to this layout carries the user's Earthdata token.
    /// A mirror needing one is skipped while the token setting is empty.
    pub const fn needs_earthdata_token(self) -> bool {
        match self {
            Self::Jpl => false,
            Self::Cddis => true,
        }
    }

    /// The host this layout is published on, which a new mirror row starts at.
    pub const fn publishing_host(self) -> &'static str {
        match self {
            Self::Jpl => DEFAULT_BASE_URL,
            Self::Cddis => cddis::DEFAULT_BASE_URL,
        }
    }
}

/// One host a day's file is requested from, and the layout it serves.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(from = "MirrorWire", into = "MirrorWire")]
pub struct Mirror {
    pub base_url: MirrorBaseUrl,
    pub layout: MirrorLayout,
}

impl Mirror {
    pub fn new(base_url: MirrorBaseUrl, layout: MirrorLayout) -> Self {
        Self { base_url, layout }
    }

    /// The mirror serving `layout` on the host that publishes it.
    pub fn publishing(layout: MirrorLayout) -> Self {
        Self::new(MirrorBaseUrl::new(layout.publishing_host()), layout)
    }

    /// The files `product` may sit under on this mirror for `day`, in the
    /// order they are requested.
    pub fn file_candidates(&self, product: IonexProduct, day: NaiveDate) -> Vec<FileCandidate> {
        match self.layout {
            MirrorLayout::Jpl => vec![FileCandidate::gzipped(
                product.file_url(self.base_url.as_ref(), day),
            )],
            MirrorLayout::Cddis => cddis::file_candidates(self.base_url.as_ref(), product, day),
        }
    }
}

/// A mirror as a settings file holds it.
///
/// Before the layouts existed an entry was the base URL alone, which loads as
/// a mirror serving JPL's layout.
#[derive(Serialize, Deserialize)]
#[serde(untagged)]
enum MirrorWire {
    Entry {
        url: MirrorBaseUrl,
        layout: MirrorLayout,
    },
    JplLayoutUrl(MirrorBaseUrl),
}

impl From<MirrorWire> for Mirror {
    fn from(wire: MirrorWire) -> Self {
        match wire {
            MirrorWire::Entry { url, layout } => Self::new(url, layout),
            MirrorWire::JplLayoutUrl(url) => Self::new(url, MirrorLayout::Jpl),
        }
    }
}

impl From<Mirror> for MirrorWire {
    fn from(mirror: Mirror) -> Self {
        Self::Entry {
            url: mirror.base_url,
            layout: mirror.layout,
        }
    }
}

/// One file a mirror may hold a day's maps in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileCandidate {
    pub url: String,
    pub compression: FileCompression,
}

impl FileCandidate {
    pub fn gzipped(url: String) -> Self {
        Self {
            url,
            compression: FileCompression::Gzip,
        }
    }

    pub fn compressed(url: String) -> Self {
        Self {
            url,
            compression: FileCompression::UnixCompress,
        }
    }
}

/// How a served file is packed, taken from the name it was requested under.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileCompression {
    Gzip,
    /// The Unix `compress` format of the legacy `.Z` names, which
    /// [`crate::unix_compress`] reads.
    UnixCompress,
}

/// The mirrors a day is requested from, in the order they are tried.
///
/// Never empty: a list with no host to fetch from is not a configuration, so
/// [`MirrorList::remove`] keeps the last entry and [`MirrorList::new`] returns
/// `None` for an empty one.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct MirrorList(Vec<Mirror>);

impl Default for MirrorList {
    fn default() -> Self {
        Self(vec![
            Mirror::publishing(MirrorLayout::Jpl),
            Mirror::publishing(MirrorLayout::Cddis),
        ])
    }
}

impl MirrorList {
    pub fn single(mirror: Mirror) -> Self {
        Self(vec![mirror])
    }

    pub fn new(mirrors: Vec<Mirror>) -> Option<Self> {
        (!mirrors.is_empty()).then_some(Self(mirrors))
    }

    pub fn as_slice(&self) -> &[Mirror] {
        &self.0
    }

    /// Add `mirror` after the ones already listed.
    pub fn add(&mut self, mirror: Mirror) {
        self.0.push(mirror);
    }

    /// Point the entry at `index` at another host or layout.
    pub fn replace(&mut self, index: usize, mirror: Mirror) {
        if let Some(entry) = self.0.get_mut(index) {
            *entry = mirror;
        }
    }

    /// The removed mirror, or `None` for the only remaining one.
    pub fn remove(&mut self, index: usize) -> Option<Mirror> {
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
    ///
    /// A mirror passed over for want of a token is a failure: nothing was
    /// learned about the day from it, and the reason belongs where the user
    /// reads why a day did not arrive.
    pub fn failure(&self) -> Option<MirrorFailure> {
        let detail = match &self.outcome {
            MirrorOutcome::NoFile => return None,
            MirrorOutcome::SkippedWithoutToken => text::MIRROR_SKIPPED_WITHOUT_TOKEN.to_owned(),
            MirrorOutcome::Failed(detail) => detail.clone(),
        };
        Some(MirrorFailure {
            mirror: self.mirror.clone(),
            detail,
        })
    }
}

/// Why a mirror did not serve a day's file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MirrorOutcome {
    /// The mirror holds no file of this product for the day.
    NoFile,
    /// The mirror serves the maps to registered callers only and no Earthdata
    /// token is set, so nothing was requested from it.
    SkippedWithoutToken,
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
/// attempts failed or was passed over.
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
    use strum::IntoEnumIterator as _;

    use super::*;

    fn mirror(host: &str) -> Mirror {
        Mirror::new(
            MirrorBaseUrl::new(format!("https://{host}.example")),
            MirrorLayout::Jpl,
        )
    }

    fn list(hosts: &[&str]) -> MirrorList {
        MirrorList::new(hosts.iter().map(|host| mirror(host)).collect()).expect("a named host")
    }

    fn hosts(mirrors: &MirrorList) -> Vec<String> {
        mirrors
            .as_slice()
            .iter()
            .map(|mirror| mirror.base_url.to_string())
            .collect()
    }

    /// The publisher first, and the authenticated archive behind it: a mirror
    /// needing a token is never the one a default install fetches from.
    #[test]
    fn the_default_list_holds_the_publishing_host_before_the_authenticated_one() {
        assert_eq!(
            MirrorList::default().as_slice(),
            [
                Mirror::new(MirrorBaseUrl::new(DEFAULT_BASE_URL), MirrorLayout::Jpl),
                Mirror::new(
                    MirrorBaseUrl::new(cddis::DEFAULT_BASE_URL),
                    MirrorLayout::Cddis
                ),
            ]
        );
    }

    #[test]
    fn every_layout_names_the_host_that_publishes_it() {
        for layout in MirrorLayout::iter() {
            let publishing = Mirror::publishing(layout);
            assert_eq!(publishing.layout, layout);
            assert!(
                publishing.base_url.as_ref().starts_with("https://"),
                "{layout}"
            );
        }
    }

    #[test]
    fn only_the_authenticated_archive_needs_a_token() {
        assert!(!MirrorLayout::Jpl.needs_earthdata_token());
        assert!(MirrorLayout::Cddis.needs_earthdata_token());
    }

    /// A mirror of the publisher's layout holds one file per product, and the
    /// authenticated archive is addressed under several names for the same one.
    #[rstest]
    #[case::the_publishers_layout(
        MirrorLayout::Jpl,
        DEFAULT_BASE_URL,
        &["https://sideshow.jpl.nasa.gov/pub/iono_daily/IONEX_final/y2024/JPLG1310.24I.gz"]
    )]
    #[case::the_authenticated_archive(
        MirrorLayout::Cddis,
        cddis::DEFAULT_BASE_URL,
        &[
            "https://cddis.nasa.gov/archive/gnss/products/ionex/2024/131/JPL0OPSFIN_20241310000_01D_02H_GIM.INX.gz",
            "https://cddis.nasa.gov/archive/gnss/products/ionex/2024/131/jplg1310.24i.Z",
        ]
    )]
    fn a_mirror_addresses_a_day_the_way_its_layout_files_it(
        #[case] layout: MirrorLayout,
        #[case] base_url: &str,
        #[case] expected: &[&str],
    ) {
        let mirror = Mirror::new(MirrorBaseUrl::new(base_url), layout);
        let day = NaiveDate::from_ymd_opt(2024, 5, 10).unwrap_or_default();

        assert_eq!(
            mirror
                .file_candidates(IonexProduct::Final, day)
                .iter()
                .map(|candidate| candidate.url.clone())
                .collect::<Vec<_>>(),
            expected
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
                    mirror: MirrorBaseUrl::new("https://first.example"),
                    detail: "HTTP 503 Service Unavailable".to_owned(),
                },
                MirrorFailure {
                    mirror: MirrorBaseUrl::new("https://second.example"),
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
    #[case::a_failure(
        MirrorOutcome::Failed("HTTP 500".to_owned()),
        Some("https://first.example: HTTP 500")
    )]
    #[case::a_mirror_passed_over_for_want_of_a_token(
        MirrorOutcome::SkippedWithoutToken,
        Some("https://first.example: no Earthdata token set")
    )]
    fn an_attempt_that_learned_nothing_about_the_day_reads_as_a_failure(
        #[case] outcome: MirrorOutcome,
        #[case] expected: Option<&str>,
    ) {
        let attempt = MirrorAttempt {
            mirror: MirrorBaseUrl::new("https://first.example"),
            product: IonexProduct::Final,
            outcome,
        };
        assert_eq!(
            attempt.failure().map(|failure| failure.to_string()),
            expected.map(str::to_owned)
        );
    }
}
