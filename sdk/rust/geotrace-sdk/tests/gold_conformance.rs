use std::env;

use geotrace_sdk::NavFile;
use geotrace_sdk_test_util as test_util;
use rstest::rstest;

#[rstest]
#[case::legacy("gold")]
#[case::navigation("navigation_evidence")]
#[case::host_clock("host_clock")]
fn all_sdks_decode_to_the_same_nav_file(#[case] family: &str) {
    let dir = test_util::fixture_path("gold_dataset");
    let canonical = NavFile::open(dir.join(format!("{family}.gtd"))).unwrap();
    for language in ["c", "cpp", "py"] {
        let name = format!("{family}_{language}.gtd");
        let path = dir.join(&name);
        if !path.exists() {
            assert!(
                env::var_os("GEOTRACE_REQUIRE_GOLD_OUTPUTS").is_none(),
                "missing required output: {name}"
            );
            eprintln!("skipping {name}: not generated (run `just test-gold-all`)");
            continue;
        }
        let other = NavFile::open(&path).unwrap();
        assert!(
            canonical.equals_ignoring_build_provenance(&other),
            "{name} decodes to a different NavFile than {family}.gtd: cross-SDK drift. \
             Regenerate the gold fixtures with `just test-gold-all`."
        );
    }
}
