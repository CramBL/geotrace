//! Fetch one day's maps, decompress them, and classify what the host
//! answered.
//!
//! Requests go out over a [`Transport`] of [`Vec<u8>`]: the archive serves
//! gzipped files, and decoding the body as text first would replace every byte
//! that is not valid UTF-8.
//!
//! A 404 means the host has no file of that product for the day, which is why
//! [`fetch_day_maps`] walks [`calendar::fetchable_products`] in order and only
//! reports [`DayFetch::Missing`] once every product has answered 404.

use std::io::Read as _;
use std::time::Duration;

use chrono::NaiveDate;
use flate2::read::GzDecoder;
use gt_fetch::{BytesResponse, Classified, HttpRequest, Transport};

use crate::maps::GlobalIonosphereMaps;
use crate::{IonexProduct, calendar, parse};

/// Minimum gap between requests to the host, enforced by the transport the
/// fetch worker connects with.
///
/// The files are static on a third-party server, and a day costs up to one
/// request per product.
pub const REQUEST_INTERVAL: Duration = Duration::from_secs(2);

/// HTTP status for a product the host has no file of for the day.
const HTTP_NOT_FOUND: u16 = 404;

/// What one day's fetch produced.
#[derive(Debug)]
pub enum DayFetch {
    /// The maps, and which product they came from.
    Fetched {
        product: IonexProduct,
        maps: Box<GlobalIonosphereMaps>,
    },
    /// No product has a file for the day.
    Missing,
    /// A product answered with a file that could not be read, or the request
    /// failed and retrying did not help.
    Failed(String),
}

/// Fetch `day` from `base_url`, trying each product
/// [`calendar::fetchable_products`] offers until one answers with a file.
///
/// A file that arrives but cannot be decompressed or parsed fails the day
/// outright: falling through to another product would answer with a second
/// estimate of a day whose first file the host does hold.
pub fn fetch_day_maps(
    transport: &impl Transport<Vec<u8>>,
    base_url: &str,
    day: NaiveDate,
    today_utc: NaiveDate,
) -> DayFetch {
    for &product in calendar::fetchable_products(day, today_utc) {
        match fetch_product(transport, base_url, product, day) {
            ProductFetch::Served(body) => {
                return match read_maps(&body) {
                    Ok(maps) => DayFetch::Fetched {
                        product,
                        maps: Box::new(maps),
                    },
                    Err(detail) => DayFetch::Failed(format!("{product}: {detail}")),
                };
            }
            ProductFetch::Missing => {}
            ProductFetch::Failed(detail) => {
                return DayFetch::Failed(format!("{product}: {detail}"));
            }
        }
    }
    DayFetch::Missing
}

/// What one product's request produced, before the body is read.
enum ProductFetch {
    Served(Vec<u8>),
    Missing,
    Failed(String),
}

fn fetch_product(
    transport: &impl Transport<Vec<u8>>,
    base_url: &str,
    product: IonexProduct,
    day: NaiveDate,
) -> ProductFetch {
    let request = HttpRequest::get(product.file_url(base_url, day));
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

    /// Replays a scripted sequence and records the URLs it was asked for.
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

    fn fetch(script: Vec<Result<BytesResponse, TransportError>>) -> (DayFetch, Vec<String>) {
        let transport = CannedTransport::new(script);
        let outcome = fetch_day_maps(&transport, crate::DEFAULT_BASE_URL, day(), today());
        (outcome, transport.urls())
    }

    /// The settled product answers, so the earlier estimate is never asked
    /// for.
    #[test]
    fn a_served_final_file_is_parsed_without_asking_for_the_rapid_one() {
        let (outcome, urls) = fetch(vec![response(200, gzipped(&published_file()))]);

        match outcome {
            DayFetch::Fetched { product, maps } => {
                assert_eq!(product, IonexProduct::Final);
                assert_eq!(maps.maps().len(), 1);
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

    #[test]
    fn a_day_neither_product_has_a_file_for_is_missing() {
        let (outcome, urls) = fetch(vec![response(404, Vec::new()), response(404, Vec::new())]);
        assert!(matches!(outcome, DayFetch::Missing), "{outcome:?}");
        assert_eq!(urls.len(), 2);
    }

    /// A body that is not gzip, and gzip holding something that is not IONEX,
    /// both fail the day, and the next product is left unasked.
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
            DayFetch::Failed(detail) => {
                assert_eq!(detail, "final: HTTP 503 Service Unavailable");
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
            crate::DEFAULT_BASE_URL,
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
        let outcome = fetch_day_maps(&transport, crate::DEFAULT_BASE_URL, day(), today());
        assert!(matches!(outcome, DayFetch::Failed(_)), "{outcome:?}");
    }
}
