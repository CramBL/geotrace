use super::*;

#[test]
fn unused_params_flow_into_the_summary() {
    let provider = TestProvider::new(1).with(QueryMetric::Velocity, vec![Some(1.0)]);
    let output = test_util::run_one(
        "points | with mask 15 deg | where velocity > 0 km/h",
        &provider,
    );
    assert_eq!(output.summary.unused_params, vec![ParamName::Mask]);
}

#[test]
fn with_params_resolve_to_base_units() {
    let query = test_util::checked(
        "points | with mask 15 deg, snr_drop 10, slip_window 5 min | where slip_all > 2 per min",
    );
    let params = query.params();
    assert_eq!(params.mask_deg, Some(15.0));
    assert_eq!(params.snr_drop_db_hz, Some(10.0));
    assert_eq!(params.slip_window_s, Some(300.0));
    assert!(query.unused_params().is_empty());
}
