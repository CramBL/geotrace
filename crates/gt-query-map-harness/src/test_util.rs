//! The generators, the renderer and the reference oracle the property tests
//! compare the harness against.
//!
//! The integration test binaries reach it as `gt_query_map_harness::test_util`,
//! through the `test-util` feature gt-query-map-harness's dev-dependency on
//! itself enables.

pub mod generate;
pub mod oracle;
