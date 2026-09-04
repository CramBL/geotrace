//! Fetch one day's maps, decompress them, and classify each mirror's
//! response.
//!
//! Requests go out over a [`Transport`] of [`Vec<u8>`]: the archives serve
//! compressed files, and decoding the body as text first would replace every
//! byte that is not valid UTF-8.
//!
//! [`fetch_day_maps`] walks the whole [`MirrorList`] for one product before it
//! moves on to the next, and reports [`DayFetch::Missing`] only once every
//! mirror has returned 404 for every product: a 404 means that one mirror
//! has no file of that product for the day.
//!
//! The Earthdata token reaches the mirrors of
//! [`MirrorLayout::Cddis`](crate::mirrors::MirrorLayout::Cddis) and no others.
//! A mirror needing one while the setting is empty is passed over as
//! [`MirrorOutcome::SkippedWithoutToken`], which the day's failure names.

use std::io::Read as _;
use std::time::Duration;

use chrono::NaiveDate;
use flate2::read::GzDecoder;
use gt_fetch::{BytesResponse, Classified, HttpRequest, SecretToken, Transport};

use crate::maps::GlobalIonosphereMaps;
use crate::mirrors::{
    FileCandidate, FileCompression, Mirror, MirrorAttempt, MirrorBaseUrl, MirrorFailure,
    MirrorList, MirrorOutcome, ProductFetchFailure,
};
use crate::{IonexProduct, calendar, parse, unix_compress};

/// Minimum gap between requests, enforced by the transport the fetch worker
/// connects with.
///
/// The files are static on third-party servers, and a day costs up to one
/// request per mirror per product.
pub const REQUEST_INTERVAL: Duration = Duration::from_secs(2);

/// HTTP status for a product the mirror has no file of for the day.
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
/// The first mirror serving a file that reads wins. A mirror returning 404 for
/// every name it files the product under is passed over, and the next product
/// is reached only once every mirror has done so: a failure leaves it unknown
/// whether the day has a settled file, and an earlier product's estimate
/// cannot settle that.
///
/// A file that arrives but cannot be decompressed or parsed counts as that
/// mirror's failure, and the same product is requested from the next mirror.
pub fn fetch_day_maps(
    transport: &impl Transport<Vec<u8>>,
    mirrors: &MirrorList,
    earthdata_token: Option<&SecretToken>,
    day: NaiveDate,
    today_utc: NaiveDate,
) -> DayFetch {
    let mut skipped: Vec<MirrorAttempt> = Vec::new();
    for &product in calendar::fetchable_products(day, today_utc) {
        for mirror in mirrors.as_slice() {
            let outcome = match fetch_product(transport, mirror, earthdata_token, product, day) {
                ProductFetch::Served { body, compression } => {
                    match read_served_file(&body, compression) {
                        Ok(maps) => {
                            return DayFetch::Fetched {
                                mirror: mirror.base_url.clone(),
                                product,
                                maps: Box::new(maps),
                                skipped,
                            };
                        }
                        Err(detail) => MirrorOutcome::Failed(detail),
                    }
                }
                ProductFetch::Missing => MirrorOutcome::NoFile,
                ProductFetch::SkippedWithoutToken => MirrorOutcome::SkippedWithoutToken,
                ProductFetch::Failed(detail) => MirrorOutcome::Failed(detail),
            };
            skipped.push(MirrorAttempt {
                mirror: mirror.base_url.clone(),
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
    Served {
        body: Vec<u8>,
        compression: FileCompression,
    },
    Missing,
    SkippedWithoutToken,
    Failed(String),
}

/// Request `product` from `mirror` under each name it files that product
/// under, in order, until one is served.
///
/// A 404 moves on to the next name, and a failure ends the mirror: it leaves
/// unknown whether the file is there under any name.
fn fetch_product(
    transport: &impl Transport<Vec<u8>>,
    mirror: &Mirror,
    earthdata_token: Option<&SecretToken>,
    product: IonexProduct,
    day: NaiveDate,
) -> ProductFetch {
    let token = if mirror.layout.needs_earthdata_token() {
        match earthdata_token {
            Some(token) => Some(token),
            None => return ProductFetch::SkippedWithoutToken,
        }
    } else {
        None
    };

    for FileCandidate { url, compression } in mirror.file_candidates(product, day) {
        let request = match token {
            Some(token) => HttpRequest::get_with_bearer_token(url, token.clone()),
            None => HttpRequest::get(url),
        };
        match gt_fetch::send_classified(transport, &request, classify, ServedFile::Failed) {
            ServedFile::Served(body) => return ProductFetch::Served { body, compression },
            // A failure detail can quote the request the client tried, so it
            // passes the token's own redaction before it is reported.
            ServedFile::Failed(detail) => {
                return ProductFetch::Failed(
                    token.map_or_else(|| detail.clone(), |token| token.redact(&detail)),
                );
            }
            ServedFile::Missing => {}
        }
    }
    ProductFetch::Missing
}

/// What one request for one name produced.
enum ServedFile {
    Served(Vec<u8>),
    Missing,
    Failed(String),
}

/// A 5xx is retried once. A 4xx is deterministic and is not.
fn classify(response: BytesResponse) -> Classified<ServedFile> {
    if !response.status_is_valid() {
        return Classified::Outcome(ServedFile::Failed(format!(
            "invalid HTTP status {}",
            response.status
        )));
    }
    if response.is_success() {
        return Classified::Outcome(ServedFile::Served(response.body));
    }
    if response.status == HTTP_NOT_FOUND {
        return Classified::Outcome(ServedFile::Missing);
    }
    if response.is_server_error() {
        return Classified::Transient(format!("HTTP {}", response.status_line()));
    }
    Classified::Outcome(ServedFile::Failed(format!(
        "HTTP {}",
        response.status_line()
    )))
}

/// Decompress a served file the way the name it was requested under packs it,
/// and parse it.
///
/// Both steps are deterministic for a given body, so the same file cannot
/// succeed on a retry.
pub fn read_served_file(
    compressed: &[u8],
    compression: FileCompression,
) -> Result<GlobalIonosphereMaps, String> {
    let text = match compression {
        FileCompression::Gzip => {
            let mut text = String::new();
            GzDecoder::new(compressed)
                .read_to_string(&mut text)
                .map_err(|err| format!("decompressing the file: {err}"))?;
            text
        }
        FileCompression::UnixCompress => {
            let bytes = unix_compress::decompress(compressed)
                .map_err(|err| format!("decompressing the file: {err}"))?;
            String::from_utf8(bytes).map_err(|err| format!("decompressing the file: {err}"))?
        }
    };
    parse::global_ionosphere_maps(&text).map_err(|err| format!("reading the file: {err}"))
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::io::Write as _;

    use gt_fetch::{TransportError, TransportSource};
    use rstest::rstest;

    use crate::mirrors::MirrorLayout;

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

    /// Replays a scripted sequence and records the requests it was sent.
    struct CannedTransport {
        script: RefCell<Vec<Result<BytesResponse, TransportError>>>,
        requests: RefCell<Vec<HttpRequest>>,
    }

    impl CannedTransport {
        fn new(script: Vec<Result<BytesResponse, TransportError>>) -> Self {
            Self {
                script: RefCell::new(script),
                requests: RefCell::new(Vec::new()),
            }
        }

        fn urls(&self) -> Vec<String> {
            self.requests
                .borrow()
                .iter()
                .map(|request| request.url().to_owned())
                .collect()
        }

        fn requests(&self) -> Vec<HttpRequest> {
            self.requests.borrow().clone()
        }
    }

    impl Transport<Vec<u8>> for CannedTransport {
        fn send(&self, request: &HttpRequest) -> Result<BytesResponse, TransportError> {
            self.requests.borrow_mut().push(request.clone());
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

    /// The legacy name's file, generated by
    /// `just qa::generate-unix-compress-fixtures`.
    const LEGACY_FILE: &[u8] = include_bytes!("../tests/fixtures/unix_compress/JPLG0920.24I.Z");

    fn token() -> SecretToken {
        SecretToken::new("earthdata-token").expect("a token")
    }

    fn jpl(base_url: &str) -> Mirror {
        Mirror::new(MirrorBaseUrl::new(base_url), MirrorLayout::Jpl)
    }

    fn one_mirror(mirror: Mirror) -> MirrorList {
        MirrorList::single(mirror)
    }

    fn publishing_host() -> MirrorList {
        one_mirror(jpl(crate::DEFAULT_BASE_URL))
    }

    fn two_mirrors() -> MirrorList {
        MirrorList::new(vec![jpl(FIRST_MIRROR), jpl(SECOND_MIRROR)]).expect("two named hosts")
    }

    fn fetch_with_token(
        mirrors: &MirrorList,
        earthdata_token: Option<&SecretToken>,
        script: Vec<Result<BytesResponse, TransportError>>,
    ) -> (DayFetch, CannedTransport) {
        let transport = CannedTransport::new(script);
        let outcome = fetch_day_maps(&transport, mirrors, earthdata_token, day(), today());
        (outcome, transport)
    }

    fn fetch_from(
        mirrors: &MirrorList,
        script: Vec<Result<BytesResponse, TransportError>>,
    ) -> (DayFetch, Vec<String>) {
        let (outcome, transport) = fetch_with_token(mirrors, None, script);
        (outcome, transport.urls())
    }

    fn fetch(script: Vec<Result<BytesResponse, TransportError>>) -> (DayFetch, Vec<String>) {
        fetch_from(&publishing_host(), script)
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
            &publishing_host(),
            None,
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
        let outcome = fetch_day_maps(&transport, &publishing_host(), None, day(), today());
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
    /// fails listing every mirror that was tried: a failure leaves it unknown
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

    /// The archive needing a token is never requested while none is set, and
    /// the day's failure states the missing token.
    #[test]
    fn a_mirror_needing_a_token_is_passed_over_while_none_is_set() {
        let (outcome, transport) = fetch_with_token(
            &one_mirror(Mirror::publishing(MirrorLayout::Cddis)),
            None,
            Vec::new(),
        );

        match outcome {
            DayFetch::Failed(failure) => {
                assert_eq!(
                    failure.to_string(),
                    format!(
                        "final: {}: no Earthdata token set",
                        crate::cddis::DEFAULT_BASE_URL
                    )
                );
            }
            other => panic!("{other:?}"),
        }
        assert!(transport.urls().is_empty(), "nothing was requested");
    }

    /// The token authenticates the archive that needs one, and the mirrors
    /// that do not are requested without it.
    #[test]
    fn the_token_reaches_the_archive_that_needs_one_and_no_other_mirror() {
        let mirrors = MirrorList::new(vec![
            jpl(FIRST_MIRROR),
            Mirror::publishing(MirrorLayout::Cddis),
        ])
        .expect("two named hosts");

        let (outcome, transport) = fetch_with_token(
            &mirrors,
            Some(&token()),
            vec![
                response(404, Vec::new()),
                response(200, gzipped(&published_file())),
            ],
        );

        assert!(matches!(outcome, DayFetch::Fetched { .. }), "{outcome:?}");
        assert_eq!(
            transport.requests(),
            [
                HttpRequest::get(format!("{FIRST_MIRROR}/IONEX_final/y2024/JPLG1310.24I.gz")),
                HttpRequest::get_with_bearer_token(
                    "https://cddis.nasa.gov/archive/gnss/products/ionex/2024/131\
                     /JPL0OPSFIN_20241310000_01D_02H_GIM.INX.gz",
                    token(),
                ),
            ]
        );
    }

    /// A day is requested under its long name and then its legacy one, and the
    /// file under the legacy name is LZW compressed.
    #[test]
    fn a_legacy_file_is_read_through_the_compress_decoder() {
        let (outcome, transport) = fetch_with_token(
            &one_mirror(Mirror::publishing(MirrorLayout::Cddis)),
            Some(&token()),
            vec![
                response(404, Vec::new()),
                response(200, LEGACY_FILE.to_vec()),
            ],
        );

        match outcome {
            DayFetch::Fetched { maps, product, .. } => {
                assert_eq!(product, IonexProduct::Final);
                assert_eq!(maps.maps().len(), 13);
            }
            other => panic!("{other:?}"),
        }
        assert_eq!(
            transport.urls().last().map(String::as_str),
            Some("https://cddis.nasa.gov/archive/gnss/products/ionex/2024/131/jplg1310.24i.Z")
        );
    }

    /// A failure quoting the request is reported through the token's own
    /// redaction, so nothing the user reads holds the token.
    #[test]
    fn a_failure_from_the_authenticated_archive_holds_no_token() {
        let (outcome, _transport) = fetch_with_token(
            &one_mirror(Mirror::publishing(MirrorLayout::Cddis)),
            Some(&token()),
            // Twice: a transport failure is retried once.
            vec![
                Err(TransportError {
                    detail: "error sending request with header Bearer earthdata-token".to_owned(),
                }),
                Err(TransportError {
                    detail: "error sending request with header Bearer earthdata-token".to_owned(),
                }),
            ],
        );

        match outcome {
            DayFetch::Failed(failure) => {
                let reported = failure.to_string();
                assert!(!reported.contains("earthdata-token"), "{reported}");
                assert!(reported.contains(gt_fetch::REDACTED_SECRET), "{reported}");
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
