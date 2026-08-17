//! Fetch one day's maps, decompress them, and classify each mirror's
//! response.
//!
//! Requests go out over a [`Transport`] of [`Vec<u8>`]: the archives serve
//! gzipped files, and decoding the body as text first would replace every byte
//! that is not valid UTF-8.
//!
//! [`fetch_day_maps`] walks the whole [`MirrorList`] for one product before it
//! moves on to the next, and reports [`DayFetch::Missing`] only once every
//! mirror has returned 404 for every product: a 404 means that one mirror
//! holds no file of that product for the day.

use std::io::Read as _;
use std::time::Duration;

use chrono::NaiveDate;
use flate2::read::GzDecoder;
use gt_fetch::{BytesResponse, Classified, HttpRequest, Transport};

use crate::maps::GlobalIonosphereMaps;
use crate::mirrors::{
    MirrorAttempt, MirrorBaseUrl, MirrorFailure, MirrorList, MirrorOutcome, ProductFetchFailure,
};
use crate::{IonexProduct, calendar, parse};

/// Minimum gap between requests, enforced by the transport the fetch worker
/// connects with.
///
/// The files are static on third-party servers, and a day costs up to one
/// request per mirror per product.
pub const REQUEST_INTERVAL: Duration = Duration::from_secs(2);

/// HTTP status for a product the mirror holds no file of for the day.
const HTTP_NOT_FOUND: u16 = 404;

/// What one day's fetch produced.
#[derive(Debug)]
pub enum DayFetch {
    /// The maps, which product they came from, and which mirror served them.
    Fetched {
        mirror: MirrorBaseUrl,
        product: IonexProduct,
        maps: Box<GlobalIonosphereMaps>,
        /// The mirrors tried before the one that served, and what each
        /// returned.
        skipped: Vec<MirrorAttempt>,
    },
    /// No mirror holds a file of any product for the day.
    Missing,
    /// No mirror served one product's file, and at least one of them failed.
    Failed(ProductFetchFailure),
}

/// Fetch `day` from `mirrors`, requesting every product
/// [`calendar::fetchable_products`] offers from each of them in order.
///
/// The first mirror serving a file that reads wins. A mirror returning 404 is
/// passed over, and the next product is reached only once every mirror has
/// returned 404 for the current one: a failure leaves it unknown whether the
/// day has a settled file, and an earlier product's estimate cannot settle
/// that.
///
/// A file that arrives but cannot be decompressed or parsed counts as that
/// mirror's failure, and the same product is requested from the next mirror.
pub fn fetch_day_maps(
    transport: &impl Transport<Vec<u8>>,
    mirrors: &MirrorList,
    day: NaiveDate,
    today_utc: NaiveDate,
) -> DayFetch {
    let mut skipped: Vec<MirrorAttempt> = Vec::new();
    for &product in calendar::fetchable_products(day, today_utc) {
        for mirror in mirrors.as_slice() {
            let outcome = match fetch_product(transport, mirror, product, day) {
                ProductFetch::Served(body) => match read_maps(&body) {
                    Ok(maps) => {
                        return DayFetch::Fetched {
                            mirror: mirror.clone(),
                            product,
                            maps: Box::new(maps),
                            skipped,
                        };
                    }
                    Err(detail) => MirrorOutcome::Failed(detail),
                },
                ProductFetch::Missing => MirrorOutcome::NoFile,
                ProductFetch::Failed(detail) => MirrorOutcome::Failed(detail),
            };
            skipped.push(MirrorAttempt {
                mirror: mirror.clone(),
                product,
                outcome,
            });
        }
        let failures: Vec<MirrorFailure> = skipped
            .iter()
            .filter(|attempt| attempt.product == product)
            .filter_map(MirrorAttempt::failure)
            .collect();
        if !failures.is_empty() {
            return DayFetch::Failed(ProductFetchFailure {
                product,
                mirrors: failures,
            });
        }
    }
    DayFetch::Missing
}

/// What one product's request to one mirror produced, before the body is read.
enum ProductFetch {
    Served(Vec<u8>),
    Missing,
    Failed(String),
}

fn fetch_product(
    transport: &impl Transport<Vec<u8>>,
    mirror: &MirrorBaseUrl,
    product: IonexProduct,
    day: NaiveDate,
) -> ProductFetch {
    let request = HttpRequest::get(product.file_url(mirror.as_ref(), day));
    gt_fetch::send_classified(transport, &request, classify, ProductFetch::Failed)
}

/// A 5xx is retried once. A 4xx is deterministic and is not.
fn classify(response: BytesResponse) -> Classified<ProductFetch> {
    if !response.status_is_valid() {
        return Classified::Outcome(ProductFetch::Failed(format!(
            "invalid HTTP status {}",
            response.status
        )));
    }
    if response.is_success() {
        return Classified::Outcome(ProductFetch::Served(response.body));
    }
    if response.status == HTTP_NOT_FOUND {
        return Classified::Outcome(ProductFetch::Missing);
    }
    if response.is_server_error() {
        return Classified::Transient(format!("HTTP {}", response.status_line()));
    }
    Classified::Outcome(ProductFetch::Failed(format!(
        "HTTP {}",
        response.status_line()
    )))
}

/// Decompress a served file and parse it.
///
/// Both steps are deterministic for a given body, so the same file cannot
/// succeed on a retry.
fn read_maps(compressed: &[u8]) -> Result<GlobalIonosphereMaps, String> {
    let mut text = String::new();
    GzDecoder::new(compressed)
        .read_to_string(&mut text)
        .map_err(|err| format!("decompressing the file: {err}"))?;
    parse::global_ionosphere_maps(&text).map_err(|err| format!("reading the file: {err}"))
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::io::Write as _;

    use gt_fetch::{TransportError, TransportSource};
    use rstest::rstest;

    use super::*;

    fn day() -> NaiveDate {
        NaiveDate::from_ymd_opt(2024, 5, 10).unwrap_or_default()
    }

    fn today() -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 8, 17).unwrap_or_default()
    }

    /// Column the record label starts at.
    const LABEL_COLUMN: usize = 60;

    /// One record: values padded out to the label columns.
    fn record(values: &str, label: &str) -> String {
        format!("{values:<LABEL_COLUMN$}{label}\n")
    }

    /// A file of one map on a grid of two latitudes and two longitudes, the
    /// smallest shape a published product's structure reduces to.
    fn published_file() -> String {
        let epoch = "  2024     5    10     0     0     0";
        [
            record(
                "     1.0            IONOSPHERE MAPS     GPS",
                "IONEX VERSION / TYPE",
            ),
            record(epoch, "EPOCH OF FIRST MAP"),
            record(epoch, "EPOCH OF LAST MAP"),
            record("  7200", "INTERVAL"),
            record("     1", "# OF MAPS IN FILE"),
            record("   450.0 450.0   0.0", "HGT1 / HGT2 / DHGT"),
            record("    87.5  85.0  -2.5", "LAT1 / LAT2 / DLAT"),
            record("  -180.0-175.0   5.0", "LON1 / LON2 / DLON"),
            record("    -1", "EXPONENT"),
            record("", "END OF HEADER"),
            record("     1", "START OF TEC MAP"),
            record(epoch, "EPOCH OF CURRENT MAP"),
            record("    87.5-180.0-175.0   5.0 450.0", "LAT/LON1/LON2/DLON/H"),
            record("  100  200", ""),
            record("    85.0-180.0-175.0   5.0 450.0", "LAT/LON1/LON2/DLON/H"),
            record("  300  400", ""),
            record("     1", "END OF TEC MAP"),
            record("", "END OF FILE"),
        ]
        .concat()
    }

    fn gzipped(text: &str) -> Vec<u8> {
        let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
        encoder.write_all(text.as_bytes()).expect("compress");
        encoder.finish().expect("finish")
    }

    fn response(status: u16, body: Vec<u8>) -> Result<BytesResponse, TransportError> {
        Ok(BytesResponse { status, body })
    }

    /// Replays a scripted sequence and records the URLs requested from it.
    struct CannedTransport {
        script: RefCell<Vec<Result<BytesResponse, TransportError>>>,
        urls: RefCell<Vec<String>>,
    }

    impl CannedTransport {
        fn new(script: Vec<Result<BytesResponse, TransportError>>) -> Self {
            Self {
                script: RefCell::new(script),
                urls: RefCell::new(Vec::new()),
            }
        }

        fn urls(&self) -> Vec<String> {
            self.urls.borrow().clone()
        }
    }

    impl Transport<Vec<u8>> for CannedTransport {
        fn send(&self, request: &HttpRequest) -> Result<BytesResponse, TransportError> {
            self.urls.borrow_mut().push(request.url().to_owned());
            let mut script = self.script.borrow_mut();
            if script.is_empty() {
                return Err(TransportError {
                    detail: "the test under-declared its requests".to_owned(),
                });
            }
            script.remove(0)
        }
    }

    const FIRST_MIRROR: &str = "https://first.example";
    const SECOND_MIRROR: &str = "https://second.example";

    fn two_mirrors() -> MirrorList {
        MirrorList::new(vec![
            MirrorBaseUrl::new(FIRST_MIRROR),
            MirrorBaseUrl::new(SECOND_MIRROR),
        ])
        .expect("two named hosts")
    }

    fn fetch_from(
        mirrors: &MirrorList,
        script: Vec<Result<BytesResponse, TransportError>>,
    ) -> (DayFetch, Vec<String>) {
        let transport = CannedTransport::new(script);
        let outcome = fetch_day_maps(&transport, mirrors, day(), today());
        (outcome, transport.urls())
    }

    fn fetch(script: Vec<Result<BytesResponse, TransportError>>) -> (DayFetch, Vec<String>) {
        fetch_from(&MirrorList::default(), script)
    }

    /// The settled product serves a file, so the earlier estimate is never
    /// requested.
    #[test]
    fn a_served_final_file_is_parsed_without_requesting_the_rapid_one() {
        let (outcome, urls) = fetch(vec![response(200, gzipped(&published_file()))]);

        match outcome {
            DayFetch::Fetched {
                mirror,
                product,
                maps,
                skipped,
            } => {
                assert_eq!(mirror, MirrorBaseUrl::new(crate::DEFAULT_BASE_URL));
                assert_eq!(product, IonexProduct::Final);
                assert_eq!(maps.maps().len(), 1);
                assert!(skipped.is_empty());
            }
            other => panic!("{other:?}"),
        }
        assert_eq!(
            urls,
            ["https://sideshow.jpl.nasa.gov/pub/iono_daily/IONEX_final/y2024/JPLG1310.24I.gz"]
        );
    }

    /// A day too recent for the settled product falls back to the earlier
    /// estimate.
    #[test]
    fn a_missing_final_file_falls_back_to_the_rapid_one() {
        let (outcome, urls) = fetch(vec![
            response(404, Vec::new()),
            response(200, gzipped(&published_file())),
        ]);

        assert!(
            matches!(
                outcome,
                DayFetch::Fetched {
                    product: IonexProduct::Rapid,
                    ..
                }
            ),
            "{outcome:?}"
        );
        assert_eq!(
            urls,
            [
                "https://sideshow.jpl.nasa.gov/pub/iono_daily/IONEX_final/y2024/JPLG1310.24I.gz",
                "https://sideshow.jpl.nasa.gov/pub/iono_daily/IONEX_rapid/JPLR1310.24I.gz",
            ]
        );
    }

    /// A body that is not gzip, and gzip holding something that is not IONEX,
    /// both fail the day, and the next product is never requested.
    #[rstest]
    #[case::a_body_that_is_not_gzip(b"<html>captive portal</html>".to_vec())]
    #[case::gzip_holding_something_else(gzipped("not an ionex file"))]
    fn a_file_that_cannot_be_read_fails_the_day(#[case] body: Vec<u8>) {
        let (outcome, urls) = fetch(vec![response(200, body)]);
        assert!(matches!(outcome, DayFetch::Failed(_)), "{outcome:?}");
        assert_eq!(urls.len(), 1, "the broken file is not retried");
    }

    #[test]
    fn a_5xx_is_retried_once_and_then_fails_the_day() {
        let (outcome, urls) = fetch(vec![response(503, Vec::new()), response(503, Vec::new())]);
        match outcome {
            DayFetch::Failed(failure) => {
                assert_eq!(
                    failure.to_string(),
                    format!(
                        "final: {}: HTTP 503 Service Unavailable",
                        crate::DEFAULT_BASE_URL
                    )
                );
            }
            other => panic!("{other:?}"),
        }
        assert_eq!(urls.len(), 2);
    }

    #[rstest]
    #[case::forbidden(403)]
    #[case::too_many_requests(429)]
    fn a_4xx_that_is_not_a_missing_file_fails_the_day(#[case] status: u16) {
        let (outcome, urls) = fetch(vec![response(status, Vec::new())]);
        assert!(matches!(outcome, DayFetch::Failed(_)), "{outcome:?}");
        assert_eq!(urls.len(), 1);
    }

    /// A day outside coverage costs no request at all.
    #[rstest]
    #[case::before_coverage(NaiveDate::from_ymd_opt(2000, 1, 1))]
    #[case::in_the_future(NaiveDate::from_ymd_opt(2030, 1, 1))]
    fn a_day_outside_coverage_is_never_requested(#[case] day: Option<NaiveDate>) {
        let transport = CannedTransport::new(Vec::new());
        let outcome = fetch_day_maps(
            &transport,
            &MirrorList::default(),
            day.unwrap_or_default(),
            today(),
        );
        assert!(matches!(outcome, DayFetch::Missing), "{outcome:?}");
        assert!(transport.urls().is_empty());
    }

    /// Offline, the host is never reached, so nothing is known about the day:
    /// that is a failure, not a missing file.
    #[test]
    fn an_offline_fetch_fails() {
        let transport = TransportSource::Offline
            .connect(None)
            .expect("the offline source connects");
        let outcome = fetch_day_maps(&transport, &MirrorList::default(), day(), today());
        assert!(matches!(outcome, DayFetch::Failed(_)), "{outcome:?}");
    }

    /// The mirror that did not serve the file is passed over, and what it
    /// returned is kept with the day the next mirror served.
    #[rstest]
    #[case::a_mirror_without_the_file(vec![response(404, Vec::new())], MirrorOutcome::NoFile)]
    #[case::a_mirror_that_fails(
        vec![response(503, Vec::new()), response(503, Vec::new())],
        MirrorOutcome::Failed("HTTP 503 Service Unavailable".to_owned())
    )]
    fn the_next_mirror_serves_a_day_the_one_before_it_did_not(
        #[case] first_mirror: Vec<Result<BytesResponse, TransportError>>,
        #[case] expected: MirrorOutcome,
    ) {
        let mut script = first_mirror;
        script.push(response(200, gzipped(&published_file())));

        let (outcome, urls) = fetch_from(&two_mirrors(), script);

        match outcome {
            DayFetch::Fetched {
                mirror,
                product,
                skipped,
                ..
            } => {
                assert_eq!(mirror, MirrorBaseUrl::new(SECOND_MIRROR));
                assert_eq!(product, IonexProduct::Final);
                assert_eq!(
                    skipped,
                    [MirrorAttempt {
                        mirror: MirrorBaseUrl::new(FIRST_MIRROR),
                        product: IonexProduct::Final,
                        outcome: expected,
                    }]
                );
            }
            other => panic!("{other:?}"),
        }
        assert_eq!(
            urls.last().map(String::as_str),
            Some("https://second.example/IONEX_final/y2024/JPLG1310.24I.gz")
        );
    }

    /// The earlier estimate is only requested once every mirror has returned
    /// 404 for the settled file, and the list is walked from the top again for
    /// it.
    #[test]
    fn the_rapid_product_is_reached_once_every_mirror_lacks_the_final_file() {
        let (outcome, urls) = fetch_from(
            &two_mirrors(),
            vec![
                response(404, Vec::new()),
                response(404, Vec::new()),
                response(200, gzipped(&published_file())),
            ],
        );

        match outcome {
            DayFetch::Fetched {
                mirror,
                product,
                skipped,
                ..
            } => {
                assert_eq!(mirror, MirrorBaseUrl::new(FIRST_MIRROR));
                assert_eq!(product, IonexProduct::Rapid);
                assert_eq!(skipped.len(), 2, "both mirrors lack the final file");
            }
            other => panic!("{other:?}"),
        }
        assert_eq!(
            urls,
            [
                "https://first.example/IONEX_final/y2024/JPLG1310.24I.gz",
                "https://second.example/IONEX_final/y2024/JPLG1310.24I.gz",
                "https://first.example/IONEX_rapid/JPLR1310.24I.gz",
            ]
        );
    }

    /// The earlier estimate is not requested once a mirror fails, and the day
    /// fails naming every mirror that was tried: a failure leaves it unknown
    /// whether the day has a settled file.
    #[test]
    fn a_failed_mirror_fails_the_day_before_the_earlier_product_is_requested() {
        let (outcome, urls) = fetch_from(
            &two_mirrors(),
            vec![
                response(503, Vec::new()),
                response(503, Vec::new()),
                response(500, Vec::new()),
                response(500, Vec::new()),
            ],
        );

        match outcome {
            DayFetch::Failed(failure) => {
                assert_eq!(
                    failure.to_string(),
                    "final: https://first.example: HTTP 503 Service Unavailable, \
                     https://second.example: HTTP 500 Internal Server Error"
                );
            }
            other => panic!("{other:?}"),
        }
        assert!(
            urls.iter().all(|url| url.contains("IONEX_final")),
            "{urls:?}"
        );
    }

    /// One mirror holding no file and another failing still fails the day: a
    /// mirror whose request failed is not a mirror that returned 404.
    #[test]
    fn a_day_one_mirror_lacks_and_another_fails_on_is_a_failure() {
        let (outcome, _urls) = fetch_from(
            &two_mirrors(),
            vec![response(404, Vec::new()), response(403, Vec::new())],
        );

        match outcome {
            DayFetch::Failed(failure) => {
                assert_eq!(
                    failure.to_string(),
                    "final: https://second.example: HTTP 403 Forbidden"
                );
            }
            other => panic!("{other:?}"),
        }
    }

    /// Every mirror returning 404 for both products is a day nobody
    /// published, not a failure.
    #[test]
    fn a_day_no_mirror_has_a_file_for_is_missing() {
        let (outcome, urls) = fetch_from(
            &two_mirrors(),
            vec![
                response(404, Vec::new()),
                response(404, Vec::new()),
                response(404, Vec::new()),
                response(404, Vec::new()),
            ],
        );

        assert!(matches!(outcome, DayFetch::Missing), "{outcome:?}");
        assert_eq!(urls.len(), 4);
    }
}
