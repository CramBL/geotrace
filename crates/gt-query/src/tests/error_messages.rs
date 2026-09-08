use super::*;

/// A diagnostic states the problem in `message` and the fix in `help`,
/// which the editor shows as a separate "Hint:" line.
#[rstest]
#[case(
    "points | where velocity > 30",
    "velocity needs a unit, e.g. 30 km/h",
    None
)]
#[case(
    "points | where velocity > 30 deg",
    "expected a speed unit (km/h, m/s, kn), found deg",
    None
)]
#[case(
    "points | where velocity == 30 km/h",
    "use a range, e.g. 29 km/h < velocity and velocity < 31 km/h",
    None
)]
#[case(
    "points | where jamming > 0.1",
    "jamming needs a unit, e.g. 50 %",
    None
)]
#[case(
    "points | window 10 | where velocity > 30 km/h",
    "velocity is per point",
    Some("wrap it in an aggregate like avg(velocity)")
)]
#[case(
    "points | where avg(velocity) > 30 km/h",
    "avg needs a window",
    Some("add a window before the where, e.g. window 10")
)]
#[case(
    "points | where util_gps < 50 %",
    "util_gps needs an elevation mask",
    Some("add: | with mask 15 deg")
)]
#[case(
    "points | where slip_all > 2 per min",
    "slip_all needs mask, snr_drop, and slip_window",
    Some("add: | with mask 15 deg, snr_drop 10, slip_window 5 min")
)]
#[case(
    "points | where eph > 20 m | window 3",
    "window must come before where",
    Some("windows always see consecutive points")
)]
fn pinned_error_messages_and_help(
    #[case] src: &str,
    #[case] expected_message: &str,
    #[case] expected_help: Option<&str>,
) {
    // The error may come from either the parse or the check stage.
    let error = parse(src)
        .and_then(|q| test_util::chk(&q).map(|_| ()))
        .expect_err(src);
    assert_eq!(error.message, expected_message, "for {src}");
    assert_eq!(error.help.as_deref(), expected_help, "for {src}");
}

/// `g`, `km/h/s`, and the `kmh` alias are accepted where their quantity
/// fits, and rejected (with a message) where it does not - `g` is an
/// acceleration, not a speed.
#[rstest]
#[case("points | window 3 | where avg(accel) >= 0.3 g", None)]
#[case("points | window 3 | where avg(accel) >= 5 km/h/s", None)]
#[case("points | where velocity > 30 kmh", None)]
#[case(
    "points | where velocity > 30 g",
    Some("expected a speed unit (km/h, m/s, kn), found g")
)]
fn acceleration_units_and_kmh_alias(#[case] src: &str, #[case] error: Option<&str>) {
    match error {
        None => {
            test_util::chk(&parse(src).expect(src)).expect(src);
        }
        Some(message) => {
            assert_eq!(
                test_util::chk(&parse(src).unwrap()).unwrap_err().message,
                message
            );
        }
    }
}

#[test]
fn error_catalog() {
    // One snapshot over every distinct diagnostic, so any wording or span
    // change shows up as a reviewable diff.
    let sources = [
        "",
        "where velocity > 0",
        "points | window",
        "points | window 0",
        "points | window 2.5",
        "points | window 10 km/h",
        "points | window 0 s",
        "points | window 3 | window 4",
        "points | draw | where velocity > 0 km/h",
        "points | draw | draw",
        "points | keep | hide",
        "points | where velocity > 0 km/h | keep | table time",
        "points | where velocity > 30 mph",
        "points | where velocity > 30 km/s",
        "points | where accel > 1 g/s",
        "points | where speed > 30 km/h",
        "points | where avg > 3",
        "points | where blah(velocity) > 3",
        "points | where velocity + 3 s > 30 km/h",
        "points | where not velocity",
        "points | where -heading < 10 deg",
        "points | window 3 | where avg(avg(velocity)) > 0 km/h",
        "points | window 3 | where avg(heading) < 10 deg",
        "points | window 3 | where spread(velocity) > 3 km/h and avg(velocity)",
        "points | where time > 100",
        "points | where sats_fix > 6 m",
        "points | where eph > velocity",
        "points | where velocity / eph > 1",
        "points | with mask 15 | where util_all < 50 %",
        "points | with snr_drop 10 db | where velocity > 0 km/h",
        "points | with slip_window 5 | where velocity > 0 km/h",
        "points | with mask 1 deg, mask 2 deg | where util_all < 50 %",
        "points | with speed 3 | where velocity > 0 km/h",
        "points | Draw",
        "points | table",
        "points | table 5",
        "points | table velocity, | draw",
        "points | where (velocity > 0 km/h",
        "points | where velocity > 2 per day",
        "points draw",
        "points |",
        "points | 5",
        "points | where > 3",
        "points | with mask deg",
        "points | with mask 1 deg | with mask 2 deg",
        "points | table time | table time",
        "points | window 5 | with mask 1 deg",
        "points | draw | window 5",
        "points | window 2 | where avg(velocity)",
        "points | where velocity",
        "points | window 2 | where avg(velocity > 0 km/h)",
        "points | where abs(velocity > 0 km/h)",
        "points | where (velocity > 0 km/h) < (eph > 1 m)",
        "points | window 2 | where first(velocity) == last(velocity)",
        "points | with snr_drop 10 s | where velocity > 0 km/h",
    ];
    let catalog: Vec<(&str, String)> = sources
        .iter()
        .map(|src| {
            let outcome = match parse(src) {
                Err(e) => diag_line(&e),
                Ok(q) => match test_util::chk(&q) {
                    Err(e) => diag_line(&e),
                    Ok(_) => "(no error)".to_owned(),
                },
            };
            (*src, outcome)
        })
        .collect();
    insta::assert_debug_snapshot!("error_catalog", catalog);
}

fn diag_line(diagnostic: &Diagnostic) -> String {
    let mut line = format!(
        "{}..{}: {}",
        diagnostic.span.start, diagnostic.span.end, diagnostic.message
    );
    if let Some(help) = &diagnostic.help {
        line.push_str(" | help: ");
        line.push_str(help);
    }
    line
}
