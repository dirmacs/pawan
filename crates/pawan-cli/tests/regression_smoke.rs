#[test]
fn tui_regression_tests_cover_past_breakages() {
    let render_tests = include_str!("../src/tui/render/mod.rs");
    let headless_tests = include_str!("tui_headless_tests.rs");

    for required in [
        "test_slash_model_enter_release_does_not_select_first_model",
        "test_slash_model_enter_repeat_does_not_select_first_model",
        "test_exact_slash_commands_dispatch_from_input_smoke",
        "test_load_available_models_preserves_live_catalog",
        "test_slash_model_preserves_live_catalog",
    ] {
        assert!(
            render_tests.contains(required),
            "missing release-regression coverage: {required}"
        );
    }

    for required in [
        "slash_help_displays_registered_commands_in_real_pty",
        "slash_tools_displays_core_and_rmux_tools_in_real_pty",
        "slash_model_enter_opens_picker_in_real_pty",
        "slash_model_picker_can_filter_live_nvidia_catalog_in_real_pty",
    ] {
        assert!(
            headless_tests.contains(required),
            "missing headless TUI smoke coverage: {required}"
        );
    }
}

#[test]
fn scheduled_regression_workflow_runs_critical_smokes() {
    let workflow = include_str!("../../../.github/workflows/regression-smoke.yml");

    for required in [
        "schedule:",
        "cargo test -p pawan --lib live_catalog",
        "E2E=1 cargo test -p pawan --lib model_catalog::tests::test_fetch_live_models_live",
        "cargo test -p pawan --test tui_headless_tests -- --nocapture",
        "slash_model_picker_can_filter_live_nvidia_catalog_in_real_pty",
        "test_slash_model_enter_release_does_not_select_first_model",
        "test_slash_model_enter_repeat_does_not_select_first_model",
        "test_exact_slash_commands_dispatch_from_input_smoke",
        "cargo test -p pawan --test regression_smoke",
    ] {
        assert!(
            workflow.contains(required),
            "regression workflow does not run required smoke: {required}"
        );
    }
}
