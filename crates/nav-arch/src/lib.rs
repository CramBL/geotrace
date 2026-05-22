#[cfg(test)]
mod tests {
    use cargo_pup_lint_config::{LintBuilder, ModuleLintExt, Severity};

    #[test]
    fn enforce_architecture() {
        let mut builder = LintBuilder::new();

        // 1. nav_geo_math Isolation (Whitelist)
        builder
            .module_lint()
            .lint_named("nav_geo_math_isolation")
            .matching(|m| m.module("nav_geo_math.*"))
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
                    "nav_geo_math.*".into(),
                ]),
                None,
            )
            .build();

        // 2. nav_fmt Isolation (Whitelist)
        builder
            .module_lint()
            .lint_named("nav_fmt_isolation")
            .matching(|m| m.module("nav_fmt.*"))
            .with_severity(Severity::Error)
            .restrict_imports(
                Some(vec![
                    "^$".into(),
                    "std.*".into(),
                    "core.*".into(),
                    "alloc.*".into(),
                    "chrono.*".into(),
                    "crate.*".into(),
                    "nav_fmt.*".into(),
                ]),
                None,
            )
            .build();

        // 3. nav_types Isolation (Whitelist)
        builder
            .module_lint()
            .lint_named("nav_types_isolation")
            .matching(|m| m.module("nav_types.*"))
            .with_severity(Severity::Error)
            .restrict_imports(
                Some(vec![
                    "^$".into(),
                    "std.*".into(),
                    "core.*".into(),
                    "alloc.*".into(),
                    "chrono.*".into(),
                    "geo.*".into(),
                    "geo_types.*".into(),
                    "uom.*".into(),
                    "nav_geo_math.*".into(),
                    "crate.*".into(),
                    "nav_types.*".into(),
                    "filter.*".into(),
                    "highlight.*".into(),
                    "markers.*".into(),
                    "nav_point.*".into(),
                    "satellites.*".into(),
                    "segment.*".into(),
                    "test_data.*".into(),
                    "trip.*".into(),
                    "tpv.*".into(),
                    "visibility.*".into(),
                ]),
                None,
            )
            .build();

        // 3. nav_map Isolation (Whitelist)
        builder
            .module_lint()
            .lint_named("nav_map_isolation")
            .matching(|m| m.module("nav_map.*"))
            .with_severity(Severity::Error)
            .restrict_imports(
                Some(vec![
                    "^$".into(),
                    "std.*".into(),
                    "core.*".into(),
                    "alloc.*".into(),
                    "egui.*".into(),
                    "walkers.*".into(),
                    "uom.*".into(),
                    "nav_types.*".into(),
                    "crate.*".into(),
                    "nav_map.*".into(),
                    "geo_types.*".into(),
                    "generated_marker_renderer.*".into(),
                    "marker_renderer.*".into(),
                    "track_renderer.*".into(),
                    "tpv_renderer.*".into(),
                ]),
                None,
            )
            .build();

        // 5. Application Root Isolation (Whitelist)
        builder
            .module_lint()
            .lint_named("naview_isolation")
            .matching(|m| m.module("^naview($|::.+)"))
            .with_severity(Severity::Error)
            .restrict_imports(
                Some(vec![
                    "^$".into(),
                    "std.*".into(),
                    "core.*".into(),
                    "alloc.*".into(),
                    "egui.*".into(),
                    "egui_phosphor.*".into(),
                    "eframe.*".into(),
                    "env_logger.*".into(),
                    "log.*".into(),
                    "walkers.*".into(),
                    "chrono.*".into(),
                    "nav_fmt.*".into(),
                    "nav_types.*".into(),
                    "nav_map.*".into(),
                    "nav_io.*".into(),
                    "nav_log_marker.*".into(),
                    "naview_sdk.*".into(),
                    "rfd.*".into(),
                    "uom.*".into(),
                    "crate.*".into(),
                    "naview.*".into(),
                    "geo_types.*".into(),
                    "app.*".into(),
                    "filter_panel.*".into(),
                    "modals.*".into(),
                    "side_panel.*".into(),
                    "trip_data_panel.*".into(),
                ]),
                None,
            )
            .build();

        // 6. nav_log_marker Isolation (Whitelist)
        builder
            .module_lint()
            .lint_named("nav_log_marker_isolation")
            .matching(|m| m.module("nav_log_marker.*"))
            .with_severity(Severity::Error)
            .restrict_imports(
                Some(vec![
                    "^$".into(),
                    "std.*".into(),
                    "core.*".into(),
                    "alloc.*".into(),
                    "chrono.*".into(),
                    "uom.*".into(),
                    "nav_types.*".into(),
                    "thiserror.*".into(),
                    "log.*".into(),
                    "crate.*".into(),
                    "nav_log_marker.*".into(),
                ]),
                None,
            )
            .build();

        // 8. naview_sdk Isolation (Whitelist) — no workspace-internal crates allowed
        builder
            .module_lint()
            .lint_named("naview_sdk_isolation")
            .matching(|m| m.module("naview_sdk.*"))
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
                    "naview_sdk.*".into(),
                    // Sub-module shorthands used by the compiler
                    "builder.*".into(),
                    "error.*".into(),
                    "read.*".into(),
                    "types.*".into(),
                    "write.*".into(),
                ]),
                None,
            )
            .build();

        // 9. nav_io Isolation (Whitelist)
        builder
            .module_lint()
            .lint_named("nav_io_isolation")
            .matching(|m| m.module("nav_io.*"))
            .with_severity(Severity::Error)
            .restrict_imports(
                Some(vec![
                    "^$".into(),
                    "std.*".into(),
                    "core.*".into(),
                    "alloc.*".into(),
                    "nav_types.*".into(),
                    "naview_sdk.*".into(),
                    "thiserror.*".into(),
                    "crate.*".into(),
                    "nav_io.*".into(),
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
