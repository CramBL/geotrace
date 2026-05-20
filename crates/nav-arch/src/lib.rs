#[cfg(test)]
mod tests {
    use cargo_pup_lint_config::{LintBuilder, LintBuilderExt, ModuleLintExt, Severity};

    #[test]
    fn enforce_architecture() {
        let mut builder = LintBuilder::new();

        // 1. nav_types Isolation (Whitelist)
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
                    "crate.*".into(),
                    "nav_types.*".into(),
                    "markers.*".into(),
                    "tpv.*".into(),
                    "test_data.*".into(),
                    "satellites.*".into(),
                    "nav_point.*".into(),
                ]),
                None,
            )
            .build();

        // 2. nav_map Isolation (Whitelist)
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
                    "marker_renderer.*".into(),
                    "tpv_renderer.*".into(),
                ]),
                None,
            )
            .build();

        // 3. Application Root Isolation (Whitelist)
        builder
            .module_lint()
            .lint_named("naview_isolation")
            .matching(|m| m.module("naview.*"))
            .with_severity(Severity::Error)
            .restrict_imports(
                Some(vec![
                    "^$".into(),
                    "std.*".into(),
                    "core.*".into(),
                    "alloc.*".into(),
                    "egui.*".into(),
                    "eframe.*".into(),
                    "env_logger.*".into(),
                    "log.*".into(),
                    "walkers.*".into(),
                    "nav_types.*".into(),
                    "nav_map.*".into(),
                    "crate.*".into(),
                    "naview.*".into(),
                    "geo_types.*".into(),
                    "app.*".into(),
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
