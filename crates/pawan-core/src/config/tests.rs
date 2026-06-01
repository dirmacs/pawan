use super::*;
use std::collections::HashMap;
use std::path::PathBuf;

#[test]
fn test_provider_mlx_parsing() {
    // "mlx" string parses to LlmProvider::Mlx via serde rename_all = "lowercase"
    let toml = r#"
provider = "mlx"
model = "mlx-community/Qwen3.5-9B-4bit"
"#;
    let config: PawanConfig = toml::from_str(toml).expect("should parse without error");
    assert_eq!(config.provider, LlmProvider::Mlx);
    assert_eq!(config.model, "mlx-community/Qwen3.5-9B-4bit");
}

#[test]
fn test_provider_mlx_lm_alias() {
    // "mlx-lm" is an alias for mlx via apply_env_overrides (env var path)
    let mut config = PawanConfig::default();
    std::env::set_var("PAWAN_PROVIDER", "mlx-lm");
    config.apply_env_overrides();
    std::env::remove_var("PAWAN_PROVIDER");
    assert_eq!(config.provider, LlmProvider::Mlx);
}

#[test]
fn test_mlx_base_url_override() {
    // When provider=mlx and base_url is set, base_url is preserved in config
    let toml = r#"
provider = "mlx"
model = "test-model"
base_url = "http://192.168.1.100:8080/v1"
"#;
    let config: PawanConfig = toml::from_str(toml).expect("should parse without error");
    assert_eq!(config.provider, LlmProvider::Mlx);
    assert_eq!(
        config.base_url.as_deref(),
        Some("http://192.168.1.100:8080/v1")
    );
}

// --- ModelRouting tests ---

#[test]
fn test_route_code_signals() {
    let routing = ModelRouting {
        code: Some("code-model".into()),
        orchestrate: Some("orch-model".into()),
        execute: Some("exec-model".into()),
    };
    assert_eq!(routing.route("implement a linked list"), Some("code-model"));
    assert_eq!(routing.route("refactor the parser"), Some("code-model"));
    assert_eq!(routing.route("add test for config"), Some("code-model"));
    assert_eq!(routing.route("Write a new struct"), Some("code-model"));
}

#[test]
fn test_route_orchestration_signals() {
    let routing = ModelRouting {
        code: Some("code-model".into()),
        orchestrate: Some("orch-model".into()),
        execute: Some("exec-model".into()),
    };
    assert_eq!(routing.route("analyze the error logs"), Some("orch-model"));
    assert_eq!(routing.route("review this PR"), Some("orch-model"));
    assert_eq!(
        routing.route("explain how the agent works"),
        Some("orch-model")
    );
    assert_eq!(routing.route("search for uses of foo"), Some("orch-model"));
}

#[test]
fn test_route_execution_signals() {
    let routing = ModelRouting {
        code: Some("code-model".into()),
        orchestrate: Some("orch-model".into()),
        execute: Some("exec-model".into()),
    };
    assert_eq!(routing.route("run cargo test"), Some("exec-model"));
    assert_eq!(
        routing.route("execute the deploy script"),
        Some("exec-model")
    );
    assert_eq!(routing.route("build the project"), Some("exec-model"));
    assert_eq!(routing.route("commit these changes"), Some("exec-model"));
}

#[test]
fn test_route_no_match_returns_none() {
    let routing = ModelRouting {
        code: Some("code-model".into()),
        orchestrate: Some("orch-model".into()),
        execute: Some("exec-model".into()),
    };
    assert_eq!(routing.route("hello world"), None);
}

#[test]
fn test_route_empty_routing_returns_none() {
    let routing = ModelRouting::default();
    assert_eq!(routing.route("implement something"), None);
    assert_eq!(routing.route("search for bugs"), None);
}

#[test]
fn test_route_case_insensitive() {
    let routing = ModelRouting {
        code: Some("code-model".into()),
        orchestrate: None,
        execute: None,
    };
    assert_eq!(routing.route("IMPLEMENT a FUNCTION"), Some("code-model"));
}

#[test]
fn test_route_partial_routing() {
    // Only code model configured, orch/exec queries return None
    let routing = ModelRouting {
        code: Some("code-model".into()),
        orchestrate: None,
        execute: None,
    };
    assert_eq!(routing.route("implement x"), Some("code-model"));
    assert_eq!(routing.route("search for y"), None);
    assert_eq!(routing.route("run tests"), None);
}

// --- apply_env_overrides tests ---

#[test]
fn test_env_override_model() {
    let mut config = PawanConfig::default();
    std::env::set_var("PAWAN_MODEL", "custom/model-123");
    config.apply_env_overrides();
    std::env::remove_var("PAWAN_MODEL");
    assert_eq!(config.model, "custom/model-123");
}

#[test]
fn test_env_override_temperature() {
    let mut config = PawanConfig::default();
    std::env::set_var("PAWAN_TEMPERATURE", "0.9");
    config.apply_env_overrides();
    std::env::remove_var("PAWAN_TEMPERATURE");
    assert!((config.temperature - 0.9).abs() < f32::EPSILON);
}

#[test]
fn test_env_override_invalid_temperature_ignored() {
    let mut config = PawanConfig::default();
    let original = config.temperature;
    std::env::set_var("PAWAN_TEMPERATURE", "not_a_number");
    config.apply_env_overrides();
    std::env::remove_var("PAWAN_TEMPERATURE");
    assert!((config.temperature - original).abs() < f32::EPSILON);
}

#[test]
fn test_env_override_max_tokens() {
    let mut config = PawanConfig::default();
    std::env::set_var("PAWAN_MAX_TOKENS", "16384");
    config.apply_env_overrides();
    std::env::remove_var("PAWAN_MAX_TOKENS");
    assert_eq!(config.max_tokens, 16384);
}

#[test]
fn test_env_override_fallback_models() {
    std::env::remove_var("PAWAN_FALLBACK_MODELS"); // Clean up before test
    let mut config = PawanConfig::default();
    std::env::set_var("PAWAN_FALLBACK_MODELS", "model-a, model-b, model-c");
    config.apply_env_overrides();
    std::env::remove_var("PAWAN_FALLBACK_MODELS");
    assert_eq!(
        config.fallback_models,
        vec!["model-a", "model-b", "model-c"]
    );
}

#[test]
fn test_env_override_fallback_models_filters_empty() {
    std::env::remove_var("PAWAN_FALLBACK_MODELS"); // Clean up before test
    let mut config = PawanConfig::default();
    std::env::set_var("PAWAN_FALLBACK_MODELS", "model-a,,, model-b,");
    config.apply_env_overrides();
    std::env::remove_var("PAWAN_FALLBACK_MODELS");
    assert_eq!(config.fallback_models, vec!["model-a", "model-b"]);
}

#[test]
fn test_env_override_provider_variants() {
    for (env_val, expected) in [
        ("nvidia", LlmProvider::Nvidia),
        ("nim", LlmProvider::Nvidia),
        ("ollama", LlmProvider::Ollama),
        ("openai", LlmProvider::OpenAI),
        ("mlx", LlmProvider::Mlx),
    ] {
        let mut config = PawanConfig::default();
        std::env::set_var("PAWAN_PROVIDER", env_val);
        config.apply_env_overrides();
        std::env::remove_var("PAWAN_PROVIDER");
        assert_eq!(
            config.provider, expected,
            "PAWAN_PROVIDER={} should map to {:?}",
            env_val, expected
        );
    }
}

// --- use_thinking_mode tests ---

#[test]
fn test_thinking_mode_supported_models() {
    for model in [
        "deepseek-ai/deepseek-r1",
        "google/gemma-4-31b-it",
        "z-ai/glm5",
        "qwen/qwen3.5-122b",
        "mistralai/mistral-small-4-119b",
    ] {
        let config = PawanConfig {
            model: model.into(),
            reasoning_mode: true,
            ..Default::default()
        };
        assert!(
            config.use_thinking_mode(),
            "thinking mode should be on for {}",
            model
        );
    }
}

#[test]
fn test_thinking_mode_disabled_when_reasoning_off() {
    let config = PawanConfig {
        model: "deepseek-ai/deepseek-r1".into(),
        reasoning_mode: false,
        ..Default::default()
    };
    assert!(!config.use_thinking_mode());
}

#[test]
fn test_thinking_mode_unsupported_models() {
    for model in [
        "meta/llama-3.1-70b",
        "minimaxai/minimax-m2.5",
        "stepfun-ai/step-3.5-flash",
    ] {
        let config = PawanConfig {
            model: model.into(),
            reasoning_mode: true,
            ..Default::default()
        };
        assert!(
            !config.use_thinking_mode(),
            "thinking mode should be off for {}",
            model
        );
    }
}

// --- get_system_prompt tests ---

#[test]
fn test_system_prompt_default() {
    let config = PawanConfig::default();
    let prompt = config.get_system_prompt();
    assert!(
        prompt.contains("Pawan"),
        "default prompt should mention Pawan"
    );
    assert!(
        prompt.contains("coding"),
        "default prompt should mention coding"
    );
}

#[test]
fn test_system_prompt_custom_override() {
    let config = PawanConfig {
        system_prompt: Some("Custom system prompt.".into()),
        ..Default::default()
    };
    let prompt = config.get_system_prompt();
    assert!(prompt.starts_with("Custom system prompt."));
}

// --- Config TOML parsing tests ---

#[test]
fn test_config_with_cloud_fallback() {
    let toml = r#"
model = "qwen/qwen3.5-122b-a10b"
[cloud]
provider = "nvidia"
model = "minimaxai/minimax-m2.5"
"#;
    let config: PawanConfig = toml::from_str(toml).expect("should parse");
    assert_eq!(config.model, "qwen/qwen3.5-122b-a10b");
    let cloud = config.cloud.unwrap();
    assert_eq!(cloud.model, "minimaxai/minimax-m2.5");
}

#[test]
fn test_config_with_healing() {
    let toml = r#"
model = "test"
[healing]
fix_errors = true
fix_warnings = false
fix_tests = true
"#;
    let config: PawanConfig = toml::from_str(toml).expect("should parse");
    assert!(config.healing.fix_errors);
    assert!(!config.healing.fix_warnings);
    assert!(config.healing.fix_tests);
}

#[test]
fn test_config_defaults_sensible() {
    let config = PawanConfig::default();
    assert_eq!(config.provider, LlmProvider::Nvidia);
    assert!(config.temperature > 0.0 && config.temperature <= 1.0);
    assert!(config.max_tokens > 0);
    assert!(config.max_tool_iterations > 0);
}

#[test]
fn test_context_file_search_order() {
    // Verify the search list includes all expected files
    // (We test the behavior via get_system_prompt since load_context_file is private
    // and changing cwd is unsafe in parallel tests)
    let config = PawanConfig::default();
    let prompt = config.get_system_prompt();
    // In the pawan repo, PAWAN.md exists, so it should be in the prompt
    if std::path::Path::new("PAWAN.md").exists() {
        assert!(
            prompt.contains("Project Context"),
            "Should inject project context when PAWAN.md exists"
        );
        assert!(
            prompt.contains("from PAWAN.md"),
            "Should identify source as PAWAN.md"
        );
    }
}

#[test]
fn test_system_prompt_injection_format() {
    // Verify the injection format includes the source filename
    let config = PawanConfig {
        system_prompt: Some("Base prompt.".into()),
        ..Default::default()
    };
    let prompt = config.get_system_prompt();
    // If any context file is found, it should show "from <filename>"
    if prompt.contains("Project Context") {
        assert!(
            prompt.contains("from "),
            "Injection should include source filename"
        );
    }
}

// --- resolve_skills_repo tests ---

#[test]
fn test_resolve_skills_repo_env_var_takes_priority() {
    // PAWAN_SKILLS_REPO pointing at a real tempdir must win over the
    // config.skills_repo field (priority 1 in the resolution chain).
    let env_dir = tempfile::TempDir::new().expect("tempdir");
    let cfg_dir = tempfile::TempDir::new().expect("tempdir");

    let config = PawanConfig {
        skills_repo: Some(cfg_dir.path().to_path_buf()),
        ..Default::default()
    };

    std::env::set_var("PAWAN_SKILLS_REPO", env_dir.path());
    let resolved = config.resolve_skills_repo();
    std::env::remove_var("PAWAN_SKILLS_REPO");

    let resolved = resolved.expect("env var path should resolve to Some");
    assert_eq!(
        resolved.canonicalize().unwrap(),
        env_dir.path().canonicalize().unwrap(),
        "env var should take priority over config.skills_repo"
    );
}

#[test]
fn test_resolve_skills_repo_env_var_nonexistent_falls_through() {
    // PAWAN_SKILLS_REPO pointing at a nonexistent path must be ignored
    // (warning logged) and the function continues to the next priority.
    // Here config.skills_repo is also nonexistent, and we cannot control
    // ~/.config/pawan/skills from a test, so we only assert that the
    // function does NOT panic and returns either None or the default dir
    // — crucially it does NOT return the bogus env var path.
    let bogus = PathBuf::from("/tmp/pawan-nonexistent-skills-repo-for-test-xyz123");
    assert!(!bogus.exists(), "precondition: bogus path must not exist");

    let config = PawanConfig {
        skills_repo: Some(PathBuf::from("/tmp/pawan-also-nonexistent-abc789")),
        ..Default::default()
    };

    std::env::set_var("PAWAN_SKILLS_REPO", &bogus);
    let resolved = config.resolve_skills_repo();
    std::env::remove_var("PAWAN_SKILLS_REPO");

    // Must never return the bogus path
    if let Some(ref p) = resolved {
        assert_ne!(p, &bogus, "nonexistent env var path must not be returned");
        assert!(
            p.is_dir(),
            "any returned path must be an existing directory"
        );
    }
}

// --- auto_discover_mcp_servers tests ---

#[test]
fn test_auto_discover_mcp_is_idempotent() {
    // Two consecutive calls: first may discover some servers, second must
    // return an empty Vec (because all are already registered). The mcp
    // hashmap length must be identical between the two calls.
    let mut config = PawanConfig::default();

    let first = config.auto_discover_mcp_servers();
    let len_after_first = config.mcp.len();

    let second = config.auto_discover_mcp_servers();
    let len_after_second = config.mcp.len();

    assert!(
        second.is_empty(),
        "second call must discover nothing (got {:?})",
        second
    );
    assert_eq!(
        len_after_first, len_after_second,
        "mcp map length must not change between calls (first discovered {:?})",
        first
    );
}

#[test]
fn test_auto_discover_mcp_preserves_existing_entries() {
    // Pre-populate config.mcp with a custom "eruka" entry. Even if
    // which::which("eruka-mcp") would find a binary on the test machine,
    // the existing entry MUST NOT be overwritten.
    let mut config = PawanConfig::default();
    let custom = McpServerEntry {
        command: "custom-eruka".to_string(),
        args: vec!["--custom-flag".to_string()],
        env: HashMap::new(),
        enabled: true,
    };
    config.mcp.insert("eruka".to_string(), custom);

    let discovered = config.auto_discover_mcp_servers();

    // "eruka" must not appear in the discovered list
    assert!(
        !discovered.contains(&"eruka".to_string()),
        "pre-existing 'eruka' entry must not be rediscovered, got {:?}",
        discovered
    );

    // Custom entry must be intact
    let entry = config
        .mcp
        .get("eruka")
        .expect("eruka entry must still exist");
    assert_eq!(
        entry.command, "custom-eruka",
        "custom command must be preserved"
    );
    assert_eq!(entry.args, vec!["--custom-flag".to_string()]);
}

// --- discover_skills_from_repo tests ---

#[test]
fn test_discover_skills_from_repo_returns_parsed_skills() {
    // Build a skills repo with one valid SKILL.md and verify that
    // discover_skills_from_repo parses it via thulp_skill_files::SkillFile.
    let repo = tempfile::TempDir::new().expect("tempdir");

    // Each skill lives in its own subdirectory containing a SKILL.md
    let skill_dir = repo.path().join("example-skill");
    std::fs::create_dir(&skill_dir).expect("mkdir example-skill");
    let skill_md = skill_dir.join("SKILL.md");
    std::fs::write(
        &skill_md,
        "---\nname: example-skill\ndescription: A test skill used in pawan unit tests\n---\n# Instructions\n\nDo the thing.\n",
    )
    .expect("write SKILL.md");

    // Also drop an empty subdirectory with no SKILL.md — should be skipped
    let empty_dir = repo.path().join("not-a-skill");
    std::fs::create_dir(&empty_dir).expect("mkdir not-a-skill");

    let config = PawanConfig {
        skills_repo: Some(repo.path().to_path_buf()),
        ..Default::default()
    };

    // Ensure env var does not interfere
    std::env::remove_var("PAWAN_SKILLS_REPO");

    let skills = config.discover_skills_from_repo();
    assert_eq!(
        skills.len(),
        1,
        "expected exactly 1 skill, got {:?}",
        skills
    );

    let (name, desc, path) = &skills[0];
    assert_eq!(name, "example-skill");
    assert_eq!(desc, "A test skill used in pawan unit tests");
    assert_eq!(path, &skill_md);
}

// ─── PawanConfig::load() edge cases (task #24) ──────────────────────

#[test]
fn test_load_with_explicit_pawan_toml_path() {
    // Happy path: explicit path to a valid pawan.toml
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let path = tmp.path().join("pawan.toml");
    std::fs::write(
        &path,
        r#"
provider = "nvidia"
model = "meta/llama-3.1-405b-instruct"
"#,
    )
    .expect("write pawan.toml");

    let config = PawanConfig::load(Some(&path)).expect("load should succeed");
    assert_eq!(config.model, "meta/llama-3.1-405b-instruct");
}

#[test]
fn test_load_with_invalid_toml_returns_error() {
    // Malformed TOML should return a Config error, not panic
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let path = tmp.path().join("pawan.toml");
    std::fs::write(&path, "this is not [[valid] toml @@").expect("write bad toml");

    let result = PawanConfig::load(Some(&path));
    assert!(result.is_err(), "malformed TOML must return Err");
    let err_msg = format!("{}", result.unwrap_err());
    assert!(
        err_msg.to_lowercase().contains("parse") || err_msg.to_lowercase().contains("failed"),
        "error should mention parse/failed, got: {}",
        err_msg
    );
}

#[test]
fn test_load_with_nonexistent_path_returns_error() {
    // An explicit path to a file that doesn't exist must return Err,
    // not silently fall through to defaults (defaults only apply when
    // path=None and no auto-discovered config exists).
    let bogus = PathBuf::from("/tmp/definitely-does-not-exist-abc123-xyz.toml");
    let result = PawanConfig::load(Some(&bogus));
    assert!(
        result.is_err(),
        "non-existent explicit path must return Err"
    );
}

#[test]
fn test_load_ares_toml_with_pawan_section() {
    // ares.toml loading must extract the [pawan] section specifically
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let path = tmp.path().join("ares.toml");
    std::fs::write(
        &path,
        r#"
# ares config (unrelated to pawan)
[server]
port = 3000

[pawan]
provider = "ollama"
model = "qwen3-coder:30b"
"#,
    )
    .expect("write ares.toml");

    let config = PawanConfig::load(Some(&path)).expect("ares.toml load should succeed");
    assert_eq!(config.provider, LlmProvider::Ollama);
    assert_eq!(config.model, "qwen3-coder:30b");
}

#[test]
fn test_load_ares_toml_without_pawan_section_returns_defaults() {
    // ares.toml with no [pawan] section must fall back to defaults,
    // not error out. This is a common setup where pawan reads an
    // ares.toml that contains no pawan-specific section.
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let path = tmp.path().join("ares.toml");
    std::fs::write(
        &path,
        r#"
[server]
port = 3000
workers = 4
"#,
    )
    .expect("write ares.toml without pawan section");

    let config = PawanConfig::load(Some(&path)).expect("load should succeed");
    // Should match defaults
    let defaults = PawanConfig::default();
    assert_eq!(config.provider, defaults.provider);
    assert_eq!(config.model, defaults.model);
}

#[test]
fn test_load_empty_toml_file_returns_defaults() {
    // A completely empty pawan.toml is valid TOML and must parse as
    // all-defaults via serde(default). This is a common first-run case.
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let path = tmp.path().join("pawan.toml");
    std::fs::write(&path, "").expect("write empty toml");

    let config = PawanConfig::load(Some(&path)).expect("empty toml should load");
    let defaults = PawanConfig::default();
    assert_eq!(config.provider, defaults.provider);
}
#[test]
fn test_default_config_version() {
    assert_eq!(default_config_version(), 1);
}

#[test]
fn test_default_tool_idle_timeout() {
    assert_eq!(default_tool_idle_timeout(), 300);
}

#[test]
fn test_config_version_field_exists() {
    let config = PawanConfig::default();
    assert_eq!(config.config_version, 1);
}

#[test]
fn test_tool_idle_timeout_field_exists() {
    let config = PawanConfig::default();
    assert_eq!(config.tool_call_idle_timeout_secs, 300);
}

#[test]
fn test_migration_result_fields() {
    let result = MigrationResult {
        migrated: true,
        from_version: 0,
        to_version: 1,
        backup_path: Some(std::path::PathBuf::from("/tmp/backup.toml")),
    };
    assert!(result.migrated);
    assert_eq!(result.from_version, 0);
    assert_eq!(result.to_version, 1);
    assert!(result.backup_path.is_some());
}

#[test]
fn test_migrate_to_latest_no_migration_needed() {
    let mut config = PawanConfig {
        config_version: 1, // Already at latest version
        ..Default::default()
    };

    let result = migrate_to_latest(&mut config, None);

    assert!(
        !result.migrated,
        "Should not migrate if already at latest version"
    );
    assert_eq!(result.from_version, 1);
    assert_eq!(result.to_version, 1);
}

#[test]
fn test_migrate_to_latest_performs_migration() {
    let mut config = PawanConfig {
        config_version: 0, // Old version
        ..Default::default()
    };

    let result = migrate_to_latest(&mut config, None);

    assert!(result.migrated, "Should migrate from old version");
    assert_eq!(result.from_version, 0);
    assert_eq!(result.to_version, 1);
    assert_eq!(config.config_version, 1, "Config version should be updated");
}

#[test]
fn test_migrate_to_v1_adds_default_fields() {
    let mut config = PawanConfig {
        config_version: 0,
        ..Default::default()
    };

    let result = migration::migrate_to_v1(&mut config);

    assert!(result.is_ok(), "Migration should succeed");
    assert_eq!(result.unwrap(), 1, "Should return new version");
    assert_eq!(config.config_version, 1, "Config version should be updated");
}

#[test]
fn test_migration_result_no_migration() {
    let result = MigrationResult::no_migration(1);

    assert!(!result.migrated, "Should indicate no migration");
    assert_eq!(result.from_version, 1);
    assert_eq!(result.to_version, 1);
    assert!(result.backup_path.is_none(), "Should not have backup path");
}

#[test]
fn test_migration_result_with_backup() {
    let backup_path = std::path::PathBuf::from("/tmp/backup.toml");
    let result = MigrationResult::new(0, 1, Some(backup_path.clone()));

    assert!(result.migrated, "Should indicate migration occurred");
    assert_eq!(result.from_version, 0);
    assert_eq!(result.to_version, 1);
    assert_eq!(
        result.backup_path,
        Some(backup_path),
        "Should have backup path"
    );
}
