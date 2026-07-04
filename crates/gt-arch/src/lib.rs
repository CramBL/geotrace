#![cfg(test)]
use cargo_pup_lint_config::{LintBuilder, LintBuilderExt, ModuleLintExt, Severity};

const UI_AND_APP_IMPORTS: &[&str] = &[
    "^eframe($|::.*)",
    "^egui($|::.*)",
    "^egui_extras($|::.*)",
    "^egui_kittest($|::.*)",
    "^egui_notify($|::.*)",
    "^egui_phosphor($|::.*)",
    "^egui_plot($|::.*)",
    "^egui_tiles($|::.*)",
    "^gt_map($|::.*)",
    "^gt_plot($|::.*)",
    "^gt_side_panel($|::.*)",
    "^gt_ui_theme($|::.*)",
    "^gt_ui_types($|::.*)",
    "^walkers($|::.*)",
    "^geotrace($|::.*)",
];

const HISTORY_BACKEND_IMPORTS: &[&str] = &[
    "^gt_history_backend_pure($|::.*)",
    "^gt_history_backend_sys($|::.*)",
    "^hdf5($|::.*)",
    "^hdf5_pure($|::.*)",
];

fn deny_imports(
    builder: &mut LintBuilder,
    name: &str,
    module_pattern: &str,
    denied_patterns: &[&str],
) {
    builder
        .module_lint()
        .lint_named(name)
        .matching(|m| m.module(module_pattern))
        .with_severity(Severity::Error)
        .restrict_imports(
            None,
            Some(
                denied_patterns
                    .iter()
                    .map(|pattern| (*pattern).to_owned())
                    .collect(),
            ),
        )
        .build();
}

#[test]
fn enforce_architecture() {
    let mut builder = LintBuilder::new();

    deny_imports(
        &mut builder,
        "sdk_does_not_import_app",
        "^geotrace_sdk($|::.*)",
        &["^gt_.*", "^geotrace($|::.*)"],
    );

    deny_imports(
        &mut builder,
        "sdk_macros_do_not_import_app",
        "^geotrace_sdk_macros($|::.*)",
        &["^gt_.*", "^geotrace($|::.*)"],
    );

    deny_imports(
        &mut builder,
        "c_sdk_wrapper_stays_on_sdk_boundary",
        "^geotrace_c($|::.*)",
        &["^gt_.*", "^geotrace($|::.*)"],
    );

    deny_imports(
        &mut builder,
        "feature_crates_do_not_import_root_app",
        "^gt_.*",
        &["^geotrace($|::.*)"],
    );

    deny_imports(
        &mut builder,
        "domain_crates_do_not_import_ui",
        "^(gt_types|gt_history_types|gt_analysis|gt_query|gt_geo_math|gt_filter|gt_track_builder)($|::.*)",
        UI_AND_APP_IMPORTS,
    );

    deny_imports(
        &mut builder,
        "data_crates_do_not_import_ui",
        "^(gt_loader|gt_loaded_files|gt_logfile|gt_history)($|::.*)",
        UI_AND_APP_IMPORTS,
    );

    deny_imports(
        &mut builder,
        "ui_support_crates_do_not_import_features",
        "^(gt_ui_theme|gt_ui_types|gt_fmt|gt_egui_mipmap)($|::.*)",
        &[
            "^gt_map($|::.*)",
            "^gt_plot($|::.*)",
            "^gt_side_panel($|::.*)",
            "^gt_loader($|::.*)",
            "^gt_history($|::.*)",
            "^geotrace($|::.*)",
        ],
    );

    deny_imports(
        &mut builder,
        "gt_map_does_not_import_other_ui_features",
        "^gt_map($|::.*)",
        &["^gt_plot($|::.*)", "^gt_side_panel($|::.*)"],
    );

    deny_imports(
        &mut builder,
        "gt_plot_does_not_import_other_ui_features",
        "^gt_plot($|::.*)",
        &["^gt_map($|::.*)", "^gt_side_panel($|::.*)"],
    );

    deny_imports(
        &mut builder,
        "gt_side_panel_does_not_import_other_ui_features",
        "^gt_side_panel($|::.*)",
        &["^gt_map($|::.*)", "^gt_plot($|::.*)"],
    );

    deny_imports(
        &mut builder,
        "history_backends_do_not_import_ui_or_app",
        "^(gt_history_backend_pure|gt_history_backend_sys)($|::.*)",
        UI_AND_APP_IMPORTS,
    );

    deny_imports(
        &mut builder,
        "app_and_feature_crates_do_not_import_history_backends",
        "^(geotrace|gt_loader|gt_loaded_files|gt_logfile|gt_map|gt_plot|gt_side_panel)($|::.*)",
        HISTORY_BACKEND_IMPORTS,
    );

    builder
        .assert_lints(Some("../../Cargo.toml"))
        .expect("cargo pup architecture checks failed");
}
