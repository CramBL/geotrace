#[cfg(test)]
mod tests {
    use cargo_pup_lint_config::{LintBuilder, ModuleLintExt, Severity};

    #[test]
    fn enforce_architecture() {
        let mut builder = LintBuilder::new();

        // 1. gt_geo_math Isolation (Whitelist)
        builder
            .module_lint()
            .lint_named("gt_geo_math_isolation")
            .matching(|m| m.module("gt_geo_math.*"))
            .with_severity(Severity::Error)
            .restrict_imports(
                Some(vec![
                    "^$".into(),
                    "std.*".into(),
                    "core.*".into(),
                    "alloc.*".into(),
                    "geo.*".into(),
                    "geo_types.*".into(),
                    "smallvec.*".into(),
                    "crate.*".into(),
                    "gt_geo_math.*".into(),
                ]),
                None,
            )
            .build();

        // 2. gt_fmt Isolation (Whitelist)
        builder
            .module_lint()
            .lint_named("gt_fmt_isolation")
            .matching(|m| m.module("gt_fmt.*"))
            .with_severity(Severity::Error)
            .restrict_imports(
                Some(vec![
                    "^$".into(),
                    "std.*".into(),
                    "core.*".into(),
                    "alloc.*".into(),
                    "chrono.*".into(),
                    "crate.*".into(),
                    "gt_fmt.*".into(),
                ]),
                None,
            )
            .build();

        // 3. gt_types Isolation (Whitelist)
        builder
            .module_lint()
            .lint_named("gt_types_isolation")
            .matching(|m| m.module("gt_types.*"))
            .with_severity(Severity::Error)
            .restrict_imports(
                Some(vec![
                    "^$".into(),
                    "std.*".into(),
                    "core.*".into(),
                    "alloc.*".into(),
                    "bon.*".into(),
                    "chrono.*".into(),
                    "geo.*".into(),
                    "geo_types.*".into(),
                    "proptest.*".into(),
                    "strum.*".into(),
                    "uom.*".into(),
                    "vec1.*".into(),
                    "gt_geo_math.*".into(),
                    "rstar.*".into(),
                    "crate.*".into(),
                    "gt_types.*".into(),
                    "coordinates.*".into(),
                    "filter.*".into(),
                    "highlight.*".into(),
                    "markers.*".into(),
                    "nav_point.*".into(),
                    "satellites.*".into(),
                    "segment.*".into(),
                    "test_data.*".into(),
                    "time_types.*".into(),
                    "track.*".into(),
                    "tpv.*".into(),
                    "visibility.*".into(),
                    "event_marker_visibility.*".into(),
                ]),
                None,
            )
            .build();

        // 3. gt_map Isolation (Whitelist)
        builder
            .module_lint()
            .lint_named("gt_map_isolation")
            .matching(|m| m.module("gt_map.*"))
            .with_severity(Severity::Error)
            .restrict_imports(
                Some(vec![
                    "^$".into(),
                    "std.*".into(),
                    "core.*".into(),
                    "alloc.*".into(),
                    "chrono.*".into(),
                    "egui.*".into(),
                    "egui_extras.*".into(),
                    "egui_kittest.*".into(),
                    "egui_phosphor.*".into(),
                    "strum.*".into(),
                    "rstar.*".into(),
                    "walkers.*".into(),
                    "uom.*".into(),
                    "gt_types.*".into(),
                    "crate.*".into(),
                    "gt_map.*".into(),
                    "geo_types.*".into(),
                    "event_marker_renderer.*".into(),
                    "generated_marker_renderer.*".into(),
                    "marker_renderer.*".into(),
                    "track_renderer.*".into(),
                    "tpv_renderer.*".into(),
                ]),
                None,
            )
            .build();

        // 4. gt_egui_mipmap Isolation (Whitelist)
        builder
            .module_lint()
            .lint_named("gt_egui_mipmap_isolation")
            .matching(|m| m.module("gt_egui_mipmap.*"))
            .with_severity(Severity::Error)
            .restrict_imports(
                Some(vec![
                    "^$".into(),
                    "std.*".into(),
                    "core.*".into(),
                    "alloc.*".into(),
                    "egui_plot.*".into(),
                    "crate.*".into(),
                    "gt_egui_mipmap.*".into(),
                ]),
                None,
            )
            .build();

        // 5. gt_plot Isolation (Whitelist)
        builder
            .module_lint()
            .lint_named("gt_plot_isolation")
            .matching(|m| m.module("gt_plot.*"))
            .with_severity(Severity::Error)
            .restrict_imports(
                Some(vec![
                    "^$".into(),
                    "std.*".into(),
                    "core.*".into(),
                    "alloc.*".into(),
                    "chrono.*".into(),
                    "egui.*".into(),
                    "egui_phosphor.*".into(),
                    "egui_plot.*".into(),
                    "gt_egui_mipmap.*".into(),
                    "rayon.*".into(),
                    "uom.*".into(),
                    "gt_types.*".into(),
                    "crate.*".into(),
                    "gt_plot.*".into(),
                    // Sub-module shorthands used by the compiler
                    "plot_widget.*".into(),
                    "series.*".into(),
                ]),
                None,
            )
            .build();

        // 6. Application Root Isolation (Whitelist)
        builder
            .module_lint()
            .lint_named("geotrace_isolation")
            .matching(|m| m.module("^geotrace($|::.+)"))
            .with_severity(Severity::Error)
            .restrict_imports(
                Some(vec![
                    "^$".into(),
                    "std.*".into(),
                    "core.*".into(),
                    "alloc.*".into(),
                    "egui.*".into(),
                    "egui_phosphor.*".into(),
                    "egui_extras.*".into(),
                    "egui_tiles.*".into(),
                    "eframe.*".into(),
                    "env_logger.*".into(),
                    "log.*".into(),
                    "walkers.*".into(),
                    "chrono.*".into(),
                    "gt_fmt.*".into(),
                    "gt_types.*".into(),
                    "gt_map.*".into(),
                    "gt_plot.*".into(),
                    "gt_io.*".into(),
                    "gt_log_marker.*".into(),
                    "gt_side_panel.*".into(),
                    "geotrace_sdk.*".into(),
                    "rfd.*".into(),
                    "uom.*".into(),
                    "crate.*".into(),
                    "geotrace.*".into(),
                    "geo_types.*".into(),
                    "app.*".into(),
                    "config_manager.*".into(),
                    "loader.*".into(),
                    "modals.*".into(),
                ]),
                None,
            )
            .build();

        // 11. gt_side_panel Isolation (Whitelist)
        builder
            .module_lint()
            .lint_named("gt_side_panel_isolation")
            .matching(|m| m.module("gt_side_panel.*"))
            .with_severity(Severity::Error)
            .restrict_imports(
                Some(vec![
                    "^$".into(),
                    "std.*".into(),
                    "core.*".into(),
                    "alloc.*".into(),
                    "chrono.*".into(),
                    "egui.*".into(),
                    "egui_phosphor.*".into(),
                    "uom.*".into(),
                    "gt_types.*".into(),
                    "gt_fmt.*".into(),
                    "crate.*".into(),
                    "gt_side_panel.*".into(),
                    // Sub-module shorthands
                    "filter.*".into(),
                    "render.*".into(),
                    "tree.*".into(),
                ]),
                None,
            )
            .build();

        // 7. gt_log_marker Isolation (Whitelist)
        builder
            .module_lint()
            .lint_named("gt_log_marker_isolation")
            .matching(|m| m.module("gt_log_marker.*"))
            .with_severity(Severity::Error)
            .restrict_imports(
                Some(vec![
                    "^$".into(),
                    "std.*".into(),
                    "core.*".into(),
                    "alloc.*".into(),
                    "chrono.*".into(),
                    "uom.*".into(),
                    "gt_types.*".into(),
                    "thiserror.*".into(),
                    "log.*".into(),
                    "crate.*".into(),
                    "gt_log_marker.*".into(),
                ]),
                None,
            )
            .build();

        // 8. geotrace_sdk_macros Isolation (Whitelist) — proc-macro support crates only
        builder
            .module_lint()
            .lint_named("geotrace_sdk_macros_isolation")
            .matching(|m| m.module("geotrace_sdk_macros.*"))
            .with_severity(Severity::Error)
            .restrict_imports(
                Some(vec![
                    "^$".into(),
                    "proc_macro.*".into(),
                    "proc_macro2.*".into(),
                    "quote.*".into(),
                    "syn.*".into(),
                    "crate.*".into(),
                    "geotrace_sdk_macros.*".into(),
                ]),
                None,
            )
            .build();

        // 9. geotrace_sdk Isolation (Whitelist) — no workspace-internal crates allowed
        builder
            .module_lint()
            .lint_named("geotrace_sdk_isolation")
            .matching(|m| m.module("^geotrace_sdk($|::.+)"))
            .with_severity(Severity::Error)
            .restrict_imports(
                Some(vec![
                    "^$".into(),
                    "std.*".into(),
                    "core.*".into(),
                    "alloc.*".into(),
                    "bon.*".into(),
                    "chrono.*".into(),
                    "uom.*".into(),
                    "thiserror.*".into(),
                    "log.*".into(),
                    "hdf5_pure.*".into(),
                    "crate.*".into(),
                    "super.*".into(),
                    "geotrace_sdk.*".into(),
                    // Sub-module shorthands used by the compiler
                    "builder.*".into(),
                    "error.*".into(),
                    "read.*".into(),
                    "time_types.*".into(),
                    "types.*".into(),
                    "units.*".into(),
                    "variant_path.*".into(),
                    "write.*".into(),
                ]),
                None,
            )
            .build();

        // 10. gt_io Isolation (Whitelist)
        builder
            .module_lint()
            .lint_named("gt_io_isolation")
            .matching(|m| m.module("gt_io.*"))
            .with_severity(Severity::Error)
            .restrict_imports(
                Some(vec![
                    "^$".into(),
                    "std.*".into(),
                    "core.*".into(),
                    "alloc.*".into(),
                    "gt_types.*".into(),
                    "geotrace_sdk.*".into(),
                    "thiserror.*".into(),
                    "crate.*".into(),
                    "gt_io.*".into(),
                    // Sub-module shorthands used by the compiler
                    "error.*".into(),
                ]),
                None,
            )
            .build();

        // Also write to pup.ron for cargo-pup CLI support
        // This makes the code in this test the source of truth for the architecture rules.
        builder
            .write_to_file("../../pup.ron")
            .expect("Failed to sync pup.ron");
    }
}
