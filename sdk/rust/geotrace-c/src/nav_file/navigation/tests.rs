#![expect(
    clippy::panic_in_result_fn,
    reason = "test assertions and error propagation"
)]

use std::error::Error;

use geotrace_sdk::NavFile;
use geotrace_sdk_test_util as test_util;
use hdf5_pure::{AttrValue, FileBuilder};
use rstest::rstest;

use super::{GtdNavFile, GtdNavPointInfoV2, GtdStatus};
use crate::navigation;

#[rstest]
#[case::v1("1")]
#[case::v2("2")]
fn legacy_files_expose_unreported_navigation_metadata(
    #[case] version: &str,
) -> Result<(), Box<dyn Error>> {
    let mut file = FileBuilder::new();
    file.set_attr("geotrace_version", AttrValue::String(version.to_owned()));
    test_util::add_nav_points_group_of_one_fix(&mut file);
    let bytes = file.finish()?;
    let handle = GtdNavFile::from_nav_file(NavFile::read(bytes.as_slice())?);
    let mut out = GtdNavPointInfoV2 {
        fix: navigation::gtd_nav_fix_v2_init(),
        usage_report_count: 0,
        observation_report_count: 0,
    };
    // SAFETY: the handle and caller-owned output remain live for the call.
    let status = unsafe { super::gtd_nav_file_get_nav_point_v2(&handle, 0, &mut out) };
    assert_eq!(status, GtdStatus::GTD_OK);
    assert_eq!(
        (
            out.fix.state,
            out.fix.method,
            out.fix.dimension,
            out.fix.position_provenance
        ),
        (0, 0, 0, 0)
    );
    assert_eq!(
        (
            out.fix.used_count.present,
            out.fix.hdop.present,
            out.fix.vdop.present,
            out.fix.pdop.present
        ),
        (0, 0, 0, 0)
    );
    Ok(())
}
