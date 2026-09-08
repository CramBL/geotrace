use super::*;

#[test]
fn display_mode_defaults_to_draw() {
    // No display stage, and a table without one, both mean draw.
    assert_eq!(
        test_util::checked("points | where velocity > 0 km/h").mode(),
        DisplayMode::Draw
    );
    assert_eq!(
        test_util::checked("points | where velocity > 0 km/h | table time").mode(),
        DisplayMode::Draw
    );
    assert_eq!(
        test_util::checked("points | where velocity > 0 km/h | keep").mode(),
        DisplayMode::Keep
    );
    assert_eq!(
        test_util::checked("points | where velocity > 0 km/h | hide | table time").mode(),
        DisplayMode::Hide
    );
}
