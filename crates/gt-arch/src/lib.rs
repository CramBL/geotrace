#[cfg(test)]
mod tests {
    use cargo_pup_lint_config::{LintBuilder, LintBuilderExt, ModuleLintExt, Severity};

    #[test]
    fn enforce_architecture() {
        let mut builder = LintBuilder::new();

        // gt_geo_math Isolation (Whitelist)
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
                    "gt_types.*".into(),
                    "crate.*".into(),
                    "gt_geo_math.*".into(),
                ]),
                None,
            )
            .build();

        // gt_ui_theme Isolation (Whitelist)
        builder
            .module_lint()
            .lint_named("gt_ui_theme_isolation")
            .matching(|m| m.module("gt_ui_theme.*"))
            .with_severity(Severity::Error)
            .restrict_imports(
                Some(vec![
                    "^$".into(),
                    "std.*".into(),
                    "core.*".into(),
                    "alloc.*".into(),
                    "egui.*".into(),
                    "gt_types.*".into(),
                    "crate.*".into(),
                    "gt_ui_theme.*".into(),
                ]),
                None,
            )
            .build();

        // gt_fmt Isolation (Whitelist)
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
                    "gt_types.*".into(),
                    "uom.*".into(),
                    "crate.*".into(),
                    "gt_fmt.*".into(),
                ]),
                None,
            )
            .build();

        // gt_types Isolation (Whitelist)
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
                    "geo_types.*".into(),
                    "proptest.*".into(),
                    "serde.*".into(),
                    "strum.*".into(),
                    "uom.*".into(),
                    "rstar.*".into(),
                    "crate.*".into(),
                    "gt_types.*".into(),
                    "coordinates.*".into(),
                    "highlight.*".into(),
                    "markers.*".into(),
                    "mercator.*".into(),
                    "metrics.*".into(),
                    "nav_point.*".into(),
                    "satellites.*".into(),
                    "thiserror.*".into(),
                    "time_types.*".into(),
                    "track.*".into(),
                    "tpv.*".into(),
                ]),
                None,
            )
            .build();

        // gt_ui_types Isolation (Whitelist)
        builder
            .module_lint()
            .lint_named("gt_ui_types_isolation")
            .matching(|m| m.module("gt_ui_types.*"))
            .with_severity(Severity::Error)
            .restrict_imports(
                Some(vec![
                    "^$".into(),
                    "std.*".into(),
                    "core.*".into(),
                    "alloc.*".into(),
                    "chrono.*".into(),
                    "gt_types.*".into(),
                    "crate.*".into(),
                    "gt_ui_types.*".into(),
                    // Sub-module shorthands
                    "event_marker_visibility.*".into(),
                    "highlight.*".into(),
                    "query_matches.*".into(),
                    "visibility.*".into(),
                ]),
                None,
            )
            .build();

        // gt_filter Isolation (Whitelist)
        builder
            .module_lint()
            .lint_named("gt_filter_isolation")
            .matching(|m| m.module("gt_filter.*"))
            .with_severity(Severity::Error)
            .restrict_imports(
                Some(vec![
                    "^$".into(),
                    "std.*".into(),
                    "core.*".into(),
                    "alloc.*".into(),
                    "chrono.*".into(),
                    "geo_types.*".into(),
                    "gt_types.*".into(),
                    "uom.*".into(),
                    "crate.*".into(),
                    "gt_filter.*".into(),
                ]),
                None,
            )
            .build();

        // gt_test_utils Isolation (Whitelist)
        builder
            .module_lint()
            .lint_named("gt_test_utils_isolation")
            .matching(|m| m.module("gt_test_utils.*"))
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
                    "geo.*".into(),
                    "geo_types.*".into(),
                    "geotrace_sdk.*".into(),
                    "gt_types.*".into(),
                    "uom.*".into(),
                    "crate.*".into(),
                    "gt_test_utils.*".into(),
                    "fixtures.*".into(),
                    "snapshot_harness.*".into(),
                ]),
                None,
            )
            .build();

        // gt_analysis Isolation (Whitelist)
        //
        // Pure domain analysis algorithms: only the shared types in gt_types,
        // std, and (in tests) chrono/proptest.  No UI, plot, or rendering crate.
        builder
            .module_lint()
            .lint_named("gt_analysis_isolation")
            .matching(|m| m.module("gt_analysis.*"))
            .with_severity(Severity::Error)
            .restrict_imports(
                Some(vec![
                    "^$".into(),
                    "std.*".into(),
                    "core.*".into(),
                    "alloc.*".into(),
                    "chrono.*".into(),
                    "gt_types.*".into(),
                    "proptest.*".into(),
                    "crate.*".into(),
                    "gt_analysis.*".into(),
                    "loss_of_lock.*".into(),
                    "satellite_utilization.*".into(),
                ]),
                None,
            )
            .build();

        // gt_track_builder Isolation (Whitelist)
        builder
            .module_lint()
            .lint_named("gt_track_builder_isolation")
            .matching(|m| m.module("gt_track_builder.*"))
            .with_severity(Severity::Error)
            .restrict_imports(
                Some(vec![
                    "^$".into(),
                    "std.*".into(),
                    "core.*".into(),
                    "alloc.*".into(),
                    "chrono.*".into(),
                    "geo_types.*".into(),
                    "gt_analysis.*".into(),
                    "gt_geo_math.*".into(),
                    "gt_types.*".into(),
                    "proptest.*".into(),
                    "rstar.*".into(),
                    "uom.*".into(),
                    "vec1.*".into(),
                    "crate.*".into(),
                    "gt_track_builder.*".into(),
                    "lod.*".into(),
                    "segment.*".into(),
                    "spatial.*".into(),
                ]),
                None,
            )
            .build();

        // gt_map Isolation (Whitelist)
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
                    "proptest.*".into(),
                    "smallvec.*".into(),
                    "strum.*".into(),
                    "rstar.*".into(),
                    "walkers.*".into(),
                    "uom.*".into(),
                    "gt_filter.*".into(),
                    "gt_track_builder.*".into(),
                    "gt_test_utils.*".into(),
                    "gt_types.*".into(),
                    "gt_ui_theme.*".into(),
                    "gt_ui_types.*".into(),
                    "crate.*".into(),
                    "gt_map.*".into(),
                    "geo_types.*".into(),
                    "event_marker_renderer.*".into(),
                    "generated_marker_renderer.*".into(),
                    "hover_labels.*".into(),
                    "icons.*".into(),
                    "marker_renderer.*".into(),
                    "query_match_renderer.*".into(),
                    "track_renderer.*".into(),
                    "tpv_renderer.*".into(),
                    "transform.*".into(),
                    "viewport.*".into(),
                ]),
                None,
            )
            .build();

        // gt_egui_mipmap Isolation (Whitelist)
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
                    "proptest.*".into(),
                    "crate.*".into(),
                    "gt_egui_mipmap.*".into(),
                ]),
                None,
            )
            .build();

        // gt_plot Isolation (Whitelist)
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
                    "gt_analysis.*".into(),
                    "gt_egui_mipmap.*".into(),
                    "gt_filter.*".into(),
                    "rayon.*".into(),
                    "strum.*".into(),
                    "uom.*".into(),
                    "gt_types.*".into(),
                    "gt_ui_theme.*".into(),
                    "gt_ui_types.*".into(),
                    "crate.*".into(),
                    "gt_plot.*".into(),
                    // Sub-module shorthands used by the compiler
                    "plot_widget.*".into(),
                    "series.*".into(),
                ]),
                None,
            )
            .build();

        // gt_query Isolation (Whitelist)
        //
        // The query language (lexer/parser/checker/evaluator): pure logic on
        // top of the shared types. No UI, plot, or rendering crate.
        builder
            .module_lint()
            .lint_named("gt_query_isolation")
            .matching(|m| m.module("gt_query.*"))
            .with_severity(Severity::Error)
            .restrict_imports(
                Some(vec![
                    "^$".into(),
                    "std.*".into(),
                    "core.*".into(),
                    "alloc.*".into(),
                    "gt_types.*".into(),
                    "logos.*".into(),
                    "strum.*".into(),
                    "uom.*".into(),
                    "insta.*".into(),
                    "proptest.*".into(),
                    "serde.*".into(),
                    "crate.*".into(),
                    "gt_query.*".into(),
                    // Sub-module shorthands used by the compiler
                    "ast.*".into(),
                    "check.*".into(),
                    "eval.*".into(),
                    "fmt.*".into(),
                    "lexer.*".into(),
                    "metric.*".into(),
                    "parser.*".into(),
                    "unit.*".into(),
                ]),
                None,
            )
            .build();

        // Application Root Isolation (Whitelist)
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
                    "egui_notify.*".into(),
                    "egui_phosphor.*".into(),
                    "egui_extras.*".into(),
                    "egui_tiles.*".into(),
                    "eframe.*".into(),
                    "env_logger.*".into(),
                    "log.*".into(),
                    "strum.*".into(),
                    "walkers.*".into(),
                    "chrono.*".into(),
                    "gt_analysis.*".into(),
                    "gt_filter.*".into(),
                    "gt_track_builder.*".into(),
                    "gt_history.*".into(),
                    "gt_fmt.*".into(),
                    "gt_types.*".into(),
                    "gt_map.*".into(),
                    "gt_plot.*".into(),
                    "gt_query.*".into(),
                    "gt_loader.*".into(),
                    "gt_logfile.*".into(),
                    "gt_side_panel.*".into(),
                    "gt_ui_theme.*".into(),
                    "gt_ui_types.*".into(),
                    "geotrace_sdk.*".into(),
                    "rfd.*".into(),
                    "uom.*".into(),
                    "crate.*".into(),
                    "geotrace.*".into(),
                    "geo_types.*".into(),
                    "app.*".into(),
                    "config_manager.*".into(),
                    "history_db.*".into(),
                    "loader.*".into(),
                    "modals.*".into(),
                    "query.*".into(),
                ]),
                None,
            )
            .build();

        // gt_history Isolation (Whitelist)
        // The `gt_history.*` pattern also covers the two backend crates
        // (`gt_history_backend_pure`, `gt_history_backend_sys`). The whitelist
        // therefore includes the system-HDF5 backend's dependencies: `hdf5.*`
        // (the `hdf5-metno` bindings, imported as `hdf5`, plus `hdf5_pure`) and
        // `tempfile` (used to stage GTD bytes for libhdf5's cross-file copy).
        builder
            .module_lint()
            .lint_named("gt_history_isolation")
            .matching(|m| m.module("gt_history.*"))
            .with_severity(Severity::Error)
            .restrict_imports(
                Some(vec![
                    "^$".into(),
                    "std.*".into(),
                    "core.*".into(),
                    "alloc.*".into(),
                    "chrono.*".into(),
                    "dirs.*".into(),
                    "gt_types.*".into(),
                    "hdf5.*".into(),
                    "log.*".into(),
                    "tempfile.*".into(),
                    "thiserror.*".into(),
                    "crate.*".into(),
                    "gt_history.*".into(),
                    "parking_lot.*".into(),
                ]),
                None,
            )
            .build();

        // gt_side_panel Isolation (Whitelist)
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
                    "gt_filter.*".into(),
                    "gt_track_builder.*".into(),
                    "gt_types.*".into(),
                    "gt_fmt.*".into(),
                    "gt_ui_theme.*".into(),
                    "gt_ui_types.*".into(),
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

        // gt_logfile Isolation (Whitelist)
        builder
            .module_lint()
            .lint_named("gt_logfile_isolation")
            .matching(|m| m.module("gt_logfile.*"))
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
                    "gt_test_utils.*".into(),
                    "thiserror.*".into(),
                    "log.*".into(),
                    "crate.*".into(),
                    "gt_logfile.*".into(),
                ]),
                None,
            )
            .build();

        // geotrace_sdk_macros Isolation (Whitelist) - proc-macro support crates only
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

        // geotrace_sdk Isolation (Whitelist) - no workspace-internal crates allowed
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
                    "strum.*".into(),
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

        // geotrace_c Isolation (Whitelist) - FFI layer. Only touches geotrace_sdk and std
        builder
            .module_lint()
            .lint_named("geotrace_c_isolation")
            .matching(|m| m.module("^geotrace_c($|::.+)"))
            .with_severity(Severity::Error)
            .restrict_imports(
                Some(vec![
                    "^$".into(),
                    "std.*".into(),
                    "core.*".into(),
                    "alloc.*".into(),
                    "chrono.*".into(),
                    "geotrace_sdk.*".into(),
                    "crate.*".into(),
                    "super.*".into(),
                    "geotrace_c.*".into(),
                    // Sub-module shorthands used by the compiler
                    "builder.*".into(),
                    "error.*".into(),
                    "macros.*".into(),
                    "nav_file.*".into(),
                ]),
                None,
            )
            .build();

        // gt_loader Isolation (Whitelist)
        builder
            .module_lint()
            .lint_named("gt_loader_isolation")
            .matching(|m| m.module("gt_loader.*"))
            .with_severity(Severity::Error)
            .restrict_imports(
                Some(vec![
                    "^$".into(),
                    "std.*".into(),
                    "core.*".into(),
                    "alloc.*".into(),
                    "gt_track_builder.*".into(),
                    "gt_types.*".into(),
                    "geotrace_sdk.*".into(),
                    "thiserror.*".into(),
                    "crate.*".into(),
                    "gt_loader.*".into(),
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

        // Run cargo-pup itself, so `cargo test -p gt-arch` is the single
        // source of truth for the architecture check (no separate `cargo
        // pup check` invocation needed).
        builder
            .assert_lints(Some("../../Cargo.toml"))
            .expect("cargo pup architecture checks failed");
    }
}
