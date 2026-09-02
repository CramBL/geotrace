pub(crate) use gt_test_utils::TestHarness;
use gt_test_utils::TestHarnessBuilder;

/// The harness builder for gt-map snapshot tests: [TestHarness::builder]
/// plus the GPU icon pipeline.
pub(crate) fn builder<'a>() -> TestHarnessBuilder<'a> {
    TestHarness::builder()
        .render_state_hook(crate::icon_mesh::gpu::install_embedded_library_without_dithering)
}
