//! Integration tests for Pawan with mocked Ollama server
//!
//! These tests use wiremock to simulate Ollama API responses,
//! allowing us to test the full agent workflow without a real LLM.
#![allow(dead_code)]

use pawan::agent::{PawanAgent, Role};
use pawan::agent::session::Session;
use pawan::config::{HealingConfig, PawanConfig};
use pawan::healing::{CompilerFixer, DiagnosticKind};
use pawan::tools::ToolRegistry;
use serde_json::json;
use std::path::PathBuf;
use tempfile::TempDir;

// ============================================================================
// Helper Functions
// ============================================================================

/// Create a config for testing
fn config_with_mock_url(_mock_url: &str) -> PawanConfig {
    let mut config = PawanConfig::default();
    config.model = "nemotron-mini".to_string();
    config
}

// ============================================================================
// Tool Registry Tests
// ============================================================================

#[test]
fn test_tool_registry_creation() {
    let temp_dir = TempDir::new().unwrap();
    let registry = ToolRegistry::with_defaults(temp_dir.path().to_path_buf());

    // Verify all expected tools are registered
    assert!(registry.has_tool("read_file"));
    assert!(registry.has_tool("write_file"));
    assert!(registry.has_tool("list_directory"));
    assert!(registry.has_tool("edit_file"));
    assert!(registry.has_tool("glob_search"));
    assert!(registry.has_tool("grep_search"));
    assert!(registry.has_tool("bash"));
    assert!(registry.has_tool("git_status"));
    assert!(registry.has_tool("git_diff"));
    assert!(registry.has_tool("git_add"));
    assert!(registry.has_tool("git_commit"));
}

#[test]
fn test_tool_registry_definitions() {
    let temp_dir = TempDir::new().unwrap();
    let registry = ToolRegistry::with_defaults(temp_dir.path().to_path_buf());

    let definitions = registry.get_all_definitions();

    // 29 total tools (7 core + 15 standard + 7 extended)
    assert_eq!(definitions.len(), 29);

    // get_definitions should only return core + standard (22 tools, not extended)
    let visible = registry.get_definitions();
    assert_eq!(visible.len(), 22);

    // After activating an extended tool, it becomes visible
    registry.activate("mise");
    let visible_after = registry.get_definitions();
    assert_eq!(visible_after.len(), 23);

    // Each definition should have name, description, and parameters
    for def in &definitions {
        assert!(!def.name.is_empty());
        assert!(!def.description.is_empty());
        assert!(def.parameters.is_object());
    }
}

#[test]
fn test_tiered_tool_visibility() {
    let temp_dir = TempDir::new().unwrap();
    let registry = ToolRegistry::with_defaults(temp_dir.path().to_path_buf());

    let visible = registry.get_definitions();
    let all = registry.get_all_definitions();
    let visible_names: Vec<&str> = visible.iter().map(|d| d.name.as_str()).collect();
    let all_names: Vec<&str> = all.iter().map(|d| d.name.as_str()).collect();

    // Core tools must always be visible
    assert!(visible_names.contains(&"bash"), "bash should be core");
    assert!(visible_names.contains(&"read_file"), "read_file should be core");
    assert!(visible_names.contains(&"write_file"), "write_file should be core");
    assert!(visible_names.contains(&"edit_file"), "edit_file should be core");
    assert!(visible_names.contains(&"ast_grep"), "ast_grep should be core");
    assert!(visible_names.contains(&"glob_search"), "glob_search should be core");
    assert!(visible_names.contains(&"grep_search"), "grep_search should be core");

    // Extended tools should NOT be visible by default
    assert!(!visible_names.contains(&"rg"), "rg should be hidden (extended)");
    assert!(!visible_names.contains(&"fd"), "fd should be hidden (extended)");
    assert!(!visible_names.contains(&"mise"), "mise should be hidden (extended)");
    assert!(!visible_names.contains(&"lsp"), "lsp should be hidden (extended)");

    // But they should exist in all_definitions
    assert!(all_names.contains(&"rg"));
    assert!(all_names.contains(&"fd"));
    assert!(all_names.contains(&"mise"));
    assert!(all_names.contains(&"lsp"));

    // Extended tools are still executable even when hidden
    assert!(registry.has_tool("rg"));
    assert!(registry.has_tool("mise"));
    assert!(registry.has_tool("lsp"));
}

#[test]
fn test_tool_activation_multiple() {
    let temp_dir = TempDir::new().unwrap();
    let registry = ToolRegistry::with_defaults(temp_dir.path().to_path_buf());

    let before = registry.get_definitions().len();

    // Activate multiple extended tools
    registry.activate("rg");
    registry.activate("fd");
    registry.activate("lsp");

    let after = registry.get_definitions().len();
    assert_eq!(after, before + 3, "Three extended tools should now be visible");

    // Activating same tool twice is idempotent
    registry.activate("rg");
    assert_eq!(registry.get_definitions().len(), after);

    // Activating nonexistent tool is a no-op
    registry.activate("nonexistent_tool");
    assert_eq!(registry.get_definitions().len(), after);
}

#[test]
fn test_tool_tier_core_count() {
    let temp_dir = TempDir::new().unwrap();
    let registry = ToolRegistry::with_defaults(temp_dir.path().to_path_buf());

    // With no activations, visible = core + standard
    let visible = registry.get_definitions();
    let all = registry.get_all_definitions();

    // Extended = all - visible
    let extended_count = all.len() - visible.len();
    assert_eq!(extended_count, 7, "Should have 7 extended tools");
}

#[tokio::test]
async fn test_read_file_tool() {
    let temp_dir = TempDir::new().unwrap();
    let test_file = temp_dir.path().join("test.txt");
    std::fs::write(&test_file, "Hello, World!").unwrap();

    let registry = ToolRegistry::with_defaults(temp_dir.path().to_path_buf());

    let result = registry
        .execute(
            "read_file",
            json!({
                "path": "test.txt"
            }),
        )
        .await
        .unwrap();

    assert!(result["content"]
        .as_str()
        .unwrap()
        .contains("Hello, World!"));
}

#[tokio::test]
async fn test_write_file_tool() {
    let temp_dir = TempDir::new().unwrap();
    let registry = ToolRegistry::with_defaults(temp_dir.path().to_path_buf());

    let result = registry
        .execute(
            "write_file",
            json!({
                "path": "new_file.txt",
                "content": "New content"
            }),
        )
        .await
        .unwrap();

    assert!(result["success"].as_bool().unwrap());

    // Verify file was written
    let content = std::fs::read_to_string(temp_dir.path().join("new_file.txt")).unwrap();
    assert_eq!(content, "New content");
}

#[tokio::test]
async fn test_edit_file_tool() {
    let temp_dir = TempDir::new().unwrap();
    let test_file = temp_dir.path().join("edit_me.txt");
    std::fs::write(&test_file, "Hello World").unwrap();

    let registry = ToolRegistry::with_defaults(temp_dir.path().to_path_buf());

    let result = registry
        .execute(
            "edit_file",
            json!({
                "path": "edit_me.txt",
                "old_string": "World",
                "new_string": "Rust"
            }),
        )
        .await
        .unwrap();

    assert!(result["success"].as_bool().unwrap());

    let content = std::fs::read_to_string(&test_file).unwrap();
    assert_eq!(content, "Hello Rust");
}

#[tokio::test]
async fn test_glob_search_tool() {
    let temp_dir = TempDir::new().unwrap();
    std::fs::write(temp_dir.path().join("file1.rs"), "rust code").unwrap();
    std::fs::write(temp_dir.path().join("file2.rs"), "more rust").unwrap();
    std::fs::write(temp_dir.path().join("file3.txt"), "text").unwrap();

    let registry = ToolRegistry::with_defaults(temp_dir.path().to_path_buf());

    let result = registry
        .execute(
            "glob_search",
            json!({
                "pattern": "*.rs"
            }),
        )
        .await
        .unwrap();

    assert_eq!(result["count"].as_u64().unwrap(), 2);
}

#[tokio::test]
async fn test_grep_search_tool() {
    let temp_dir = TempDir::new().unwrap();
    std::fs::write(
        temp_dir.path().join("code.rs"),
        "fn main() {\n    println!(\"hello\");\n}",
    )
    .unwrap();

    let registry = ToolRegistry::with_defaults(temp_dir.path().to_path_buf());

    let result = registry
        .execute(
            "grep_search",
            json!({
                "pattern": "println",
                "include": "*.rs"
            }),
        )
        .await
        .unwrap();

    assert!(result["count"].as_u64().unwrap() >= 1);
    assert!(result["results"].as_str().unwrap().contains("println"));
}

// ============================================================================
// Healing Module Tests
// ============================================================================

#[test]
fn test_parse_text_diagnostics() {
    let output = r#"error[E0425]: cannot find value `x` in this scope
   --> src/main.rs:10:5
    |
10  |     x
    |     ^ not found in this scope

warning: unused variable: `y`
   --> src/main.rs:5:9
    |
5   |     let y = 42;
    |         ^ help: if this is intentional, prefix it with an underscore: `_y`
"#;

    let fixer = CompilerFixer::new(PathBuf::from("."));
    let diagnostics = fixer.parse_diagnostics(output);

    // Should find both error and warning
    let errors: Vec<_> = diagnostics
        .iter()
        .filter(|d| d.kind == DiagnosticKind::Error)
        .collect();
    let _warnings: Vec<_> = diagnostics
        .iter()
        .filter(|d| d.kind == DiagnosticKind::Warning)
        .collect();

    assert!(!errors.is_empty(), "Should find at least one error");
    assert!(
        errors[0].message.contains("cannot find value"),
        "Error message should match"
    );
}

#[test]
fn test_healing_config_defaults() {
    let config = HealingConfig::default();

    assert!(!config.auto_commit);
    assert!(config.fix_errors);
    assert!(config.fix_warnings);
    assert!(config.fix_tests);
    assert!(!config.generate_docs);
    assert_eq!(config.max_attempts, 3);
}

// ============================================================================
// Config Tests
// ============================================================================

#[test]
fn test_config_defaults() {
    let config = PawanConfig::default();

    assert_eq!(config.model, "qwen/qwen3.5-122b-a10b");
    assert!(!config.dry_run);
    assert!(config.auto_backup);
    assert!(config.reasoning_mode);
    assert_eq!(config.max_tool_iterations, 50);
}

#[test]
fn test_config_system_prompt_with_reasoning() {
    let mut config = PawanConfig::default();
    config.reasoning_mode = true;

    let prompt = config.get_system_prompt();

    // System prompt contains Pawan identity
    assert!(prompt.contains("Pawan"));
    // Default model (mistral-small-4) supports thinking
    assert!(config.use_thinking_mode());
    // Non-thinking models don't activate it
    config.model = "stepfun-ai/step-3.5-flash".to_string();
    assert!(!config.use_thinking_mode());
    // DeepSeek also activates thinking
    config.model = "deepseek-ai/deepseek-v3.2".to_string();
    assert!(config.use_thinking_mode());
}

#[test]
fn test_config_system_prompt_without_reasoning() {
    let mut config = PawanConfig::default();
    config.reasoning_mode = false;

    let prompt = config.get_system_prompt();

    // System prompt still contains Pawan identity
    assert!(prompt.contains("Pawan"));
    // With reasoning_mode false, thinking mode should be disabled
    assert!(!config.use_thinking_mode());
}

#[test]
fn test_config_load_from_toml() {
    let temp_dir = TempDir::new().unwrap();
    let config_path = temp_dir.path().join("pawan.toml");

    std::fs::write(
        &config_path,
        r#"
model = "llama3.2"
dry_run = true
temperature = 0.8
max_tool_iterations = 100

[healing]
fix_errors = true
fix_warnings = false
auto_commit = true

[tui]
syntax_highlighting = false
"#,
    )
    .unwrap();

    let config = PawanConfig::load(Some(&config_path)).unwrap();

    assert_eq!(config.model, "llama3.2");
    assert!(config.dry_run);
    assert_eq!(config.temperature, 0.8);
    assert_eq!(config.max_tool_iterations, 100);
    assert!(config.healing.fix_errors);
    assert!(!config.healing.fix_warnings);
    assert!(config.healing.auto_commit);
    assert!(!config.tui.syntax_highlighting);
}

#[test]
fn test_config_targets() {
    let config = PawanConfig::default();

    // Should have default targets
    assert!(config.get_target("self").is_some());

    // Nonexistent target
    assert!(config.get_target("nonexistent").is_none());
}

// ============================================================================
// Agent Tests (with mocked Ollama)
// ============================================================================

#[tokio::test]
async fn test_agent_creation() {
    let temp_dir = TempDir::new().unwrap();
    let config = PawanConfig::default();

    let agent = PawanAgent::new(config, temp_dir.path().to_path_buf());

    // Agent should be created with empty history
    assert!(agent.history().is_empty());
}

#[tokio::test]
async fn test_agent_clear_history() {
    let temp_dir = TempDir::new().unwrap();
    let config = PawanConfig::default();

    let mut agent = PawanAgent::new(config, temp_dir.path().to_path_buf());

    // Add a message
    agent.add_message(pawan::agent::Message {
        role: Role::User,
        content: "Hello".to_string(),
        tool_calls: vec![],
        tool_result: None,
    });

    assert_eq!(agent.history().len(), 1);

    agent.clear_history();

    assert!(agent.history().is_empty());
}

#[tokio::test]
async fn test_agent_tool_definitions() {
    let temp_dir = TempDir::new().unwrap();
    let config = PawanConfig::default();

    let agent = PawanAgent::new(config, temp_dir.path().to_path_buf());
    let definitions = agent.get_tool_definitions();

    // get_tool_definitions returns visible tools (core + standard = 22)
    assert_eq!(definitions.len(), 22);

    // Verify tool names
    let names: Vec<&str> = definitions.iter().map(|d| d.name.as_str()).collect();
    assert!(names.contains(&"read_file"));
    assert!(names.contains(&"write_file"));
    assert!(names.contains(&"bash"));
}

// ============================================================================
// Message Serialization Tests
// ============================================================================

#[test]
fn test_message_serialization() {
    use pawan::agent::Message;

    let msg = Message {
        role: Role::User,
        content: "Hello".to_string(),
        tool_calls: vec![],
        tool_result: None,
    };

    let json = serde_json::to_string(&msg).unwrap();
    let parsed: Message = serde_json::from_str(&json).unwrap();

    assert_eq!(parsed.role, Role::User);
    assert_eq!(parsed.content, "Hello");
}

#[test]
fn test_tool_call_request_serialization() {
    use pawan::agent::ToolCallRequest;

    let tc = ToolCallRequest {
        id: "123".to_string(),
        name: "read_file".to_string(),
        arguments: json!({"path": "test.txt"}),
    };

    let json = serde_json::to_string(&tc).unwrap();
    let parsed: ToolCallRequest = serde_json::from_str(&json).unwrap();

    assert_eq!(parsed.id, "123");
    assert_eq!(parsed.name, "read_file");
}

// ============================================================================
// End-to-end Tests (require OLLAMA_URL to be set or Ollama running)
// ============================================================================

/// Test agent executes a simple prompt with mock backend (no real LLM required)
#[tokio::test]
async fn test_agent_simple_execution() {
    use pawan::agent::backend::mock::{MockBackend, MockResponse};

    let temp_dir = TempDir::new().unwrap();
    let config = PawanConfig::default();

    let backend = MockBackend::new(vec![MockResponse::text("Hello! I'm Pawan.")]);
    let mut agent = PawanAgent::new(config, temp_dir.path().to_path_buf())
        .with_backend(Box::new(backend));

    let response = agent.execute("Say hello").await;

    assert!(response.is_ok(), "Agent execution failed: {:?}", response.err());
    let response = response.unwrap();
    assert_eq!(response.content, "Hello! I'm Pawan.");
    assert_eq!(response.iterations, 1);
    assert!(response.tool_calls.is_empty());
}

/// Test agent tool-calling loop: mock issues a tool call, then final text response
#[tokio::test]
async fn test_agent_tool_call_loop() {
    use pawan::agent::backend::mock::{MockBackend, MockResponse};
    use serde_json::json;

    let temp_dir = TempDir::new().unwrap();

    // Write a file the agent will "read"
    std::fs::write(temp_dir.path().join("hello.txt"), "hello world").unwrap();

    let config = PawanConfig::default();
    let backend = MockBackend::new(vec![
        // First response: tool call to read the file
        MockResponse::tool_call("read_file", json!({"path": "hello.txt"})),
        // Second response: final text after seeing the file content
        MockResponse::text("The file contains: hello world"),
    ]);

    let mut agent = PawanAgent::new(config, temp_dir.path().to_path_buf())
        .with_backend(Box::new(backend));

    let response = agent.execute("What is in hello.txt?").await.unwrap();

    assert_eq!(response.content, "The file contains: hello world");
    assert_eq!(response.iterations, 2);
    assert_eq!(response.tool_calls.len(), 1);
    assert_eq!(response.tool_calls[0].name, "read_file");
    assert!(response.tool_calls[0].success);
}

/// Test agent heal prompt uses mock backend (no cargo, no real LLM)
#[tokio::test]
async fn test_agent_heal_prompt_sent() {
    use pawan::agent::backend::mock::{MockBackend, MockResponse};

    let temp_dir = TempDir::new().unwrap();
    let config = PawanConfig::default();

    let backend = MockBackend::new(vec![MockResponse::text(
        "I'll heal this project: cargo check shows no errors.",
    )]);
    let mut agent = PawanAgent::new(config, temp_dir.path().to_path_buf())
        .with_backend(Box::new(backend));

    let response = agent.heal().await;

    assert!(response.is_ok(), "Heal failed: {:?}", response.err());
    let response = response.unwrap();
    // Heal prompt should be in history (user message)
    assert!(!agent.history().is_empty());
    // The heal prompt mentions workspace path (search all messages, not just index 0)
    let path_str = temp_dir.path().to_str().unwrap();
    assert!(
        agent.history().iter().any(|m| m.content.contains(path_str)),
        "heal prompt not found in history"
    );
    assert_eq!(response.iterations, 1);
}

/// Test agent with denied tool — should return error result but not crash
#[tokio::test]
async fn test_agent_tool_denied_by_permission() {
    use pawan::agent::backend::mock::{MockBackend, MockResponse};
    use pawan::config::ToolPermission;
    use serde_json::json;

    let temp_dir = TempDir::new().unwrap();
    let mut config = PawanConfig::default();
    config
        .permissions
        .insert("bash".to_string(), ToolPermission::Deny);

    let backend = MockBackend::new(vec![
        MockResponse::tool_call("bash", json!({"command": "rm -rf /"})),
        MockResponse::text("I couldn't run that command."),
    ]);

    let mut agent = PawanAgent::new(config, temp_dir.path().to_path_buf())
        .with_backend(Box::new(backend));

    let response = agent.execute("Delete everything").await.unwrap();

    // The bash call should appear in records but with success=false
    assert_eq!(response.tool_calls.len(), 1);
    assert!(!response.tool_calls[0].success);
    assert_eq!(response.content, "I couldn't run that command.");
}

/// Test that context pruning triggers when context estimate exceeds max_context_tokens

/// Test that context pruning triggers when context estimate exceeds max_context_tokens
#[tokio::test]
async fn test_context_pruning() {
    use pawan::agent::backend::mock::{MockBackend, MockResponse};

    let temp_dir = tempfile::TempDir::new().unwrap();
    let mut config = PawanConfig::default();
    // Set very low context limit to trigger pruning
    config.max_context_tokens = 100;

    // Generate many tool calls to inflate history
    let mut responses = Vec::new();
    for i in 0..20 {
        responses.push(MockResponse::tool_call(
            "read_file",
            json!({"file_path": format!("/tmp/test_{}.txt", i)}),
        ));
    }
    responses.push(MockResponse::text("Done reading all files."));

    let backend = MockBackend::new(responses);
    let mut agent = PawanAgent::new(config, temp_dir.path().to_path_buf())
        .with_backend(Box::new(backend));

    let response = agent.execute("Read many files").await.unwrap();

    // Verify the agent completed successfully
    assert_eq!(response.content, "Done reading all files.");

    // Verify history was pruned (should be much less than 20+ messages)
    // System prompt + pruned summary + last 4 messages = ~6 messages max
    let history_len = agent.history().len();
    assert!(
        history_len <= 10,
        "History should be pruned but has {} messages",
        history_len
    );
    // Verify that pruning occurred by checking history length
    // The history should be much shorter than the original 20+ messages
    assert!(history_len <= 10, "History should be pruned but has {} messages", history_len);

    // Verify the first message is the user prompt (system prompt is handled by backend)
    assert_eq!(agent.history()[0].role, Role::User);
    assert_eq!(agent.history()[0].content, "Read many files");
	
}

/// Test spawn_agents parallel execution
/// Requires pawan binary to be built — run with: cargo test test_spawn_agents_parallel -- --ignored
#[tokio::test]
#[ignore]
async fn test_spawn_agents_parallel() {
    use pawan::tools::ToolRegistry;

    let temp_dir = tempfile::TempDir::new().unwrap();

    // Create two temp files with known content
    let file1 = temp_dir.path().join("file1.txt");
    let file2 = temp_dir.path().join("file2.txt");
    std::fs::write(&file1, "content_alpha").unwrap();
    std::fs::write(&file2, "content_beta").unwrap();

    let registry = ToolRegistry::with_defaults(temp_dir.path().to_path_buf());

    // Call spawn_agents with two tasks
    let args = json!({
        "tasks": [
            {"prompt": format!("Read the file at {} and tell me its contents", file1.display()), "timeout": 30},
            {"prompt": format!("Read the file at {} and tell me its contents", file2.display()), "timeout": 30}
        ]
    });

    let result = registry.execute("spawn_agents", args).await;

    match result {
        Ok(value) => {
            assert_eq!(value["total_tasks"], 2);
            let results = value["results"].as_array().unwrap();
            assert_eq!(results.len(), 2);
            // Both should have attempted (success depends on binary availability)
        }
        Err(_) => {
            // Expected if pawan binary is not built
            eprintln!("spawn_agents test skipped: pawan binary not available");
        }
    }
}


/// Test model fallback chain configuration
#[test]
fn test_model_fallback_chain() {
    // Test default config has no fallback models
    let config = PawanConfig::default();
    assert!(config.fallback_models.is_empty(), "Default should have no fallback models");

    // Test setting fallback models directly
    let mut config = PawanConfig::default();
    config.fallback_models = vec![
        "meta/llama-3.3-70b-instruct".to_string(),
        "nvidia/llama-3.1-nemotron-70b-instruct".to_string(),
    ];
    assert_eq!(config.fallback_models.len(), 2);
    assert_eq!(config.fallback_models[0], "meta/llama-3.3-70b-instruct");

    // Test env override parsing
    std::env::set_var("PAWAN_FALLBACK_MODELS", "model-a, model-b, model-c");
    let mut config = PawanConfig::default();
    config.apply_env_overrides();
    assert_eq!(config.fallback_models.len(), 3);
    assert_eq!(config.fallback_models[0], "model-a");
    assert_eq!(config.fallback_models[1], "model-b");
    assert_eq!(config.fallback_models[2], "model-c");

    // Clean up env
    std::env::remove_var("PAWAN_FALLBACK_MODELS");

    // Test empty env var
    std::env::set_var("PAWAN_FALLBACK_MODELS", "");
    let mut config = PawanConfig::default();
    config.apply_env_overrides();
    assert!(config.fallback_models.is_empty(), "Empty env should give no fallback models");
    std::env::remove_var("PAWAN_FALLBACK_MODELS");
}


/// Test that tool results exceeding max_result_chars are truncated

/// Test that tool results exceeding max_result_chars are truncated
#[tokio::test]
async fn test_tool_result_truncation() {
    use pawan::agent::backend::mock::{MockBackend, MockResponse};

    let temp_dir = tempfile::TempDir::new().unwrap();

    // Create a file larger than 8000 chars
    let large_content = "A".repeat(20000);
    let large_file = temp_dir.path().join("large.txt");
    std::fs::write(&large_file, &large_content).unwrap();

    let mut config = PawanConfig::default();
    config.max_result_chars = 8000;

    let backend = MockBackend::new(vec![
        MockResponse::tool_call("read_file", json!({"path": large_file.to_string_lossy()})),
        MockResponse::text("I read the large file."),
    ]);

    let mut agent = PawanAgent::new(config, temp_dir.path().to_path_buf())
        .with_backend(Box::new(backend));

    let response = agent.execute("Read the large file").await.unwrap();

    assert_eq!(response.content, "I read the large file.");

    // Check that tool result in history was truncated
    let tool_messages: Vec<_> = agent.history().iter()
        .filter(|m| m.role == Role::Tool)
        .collect();

    assert!(!tool_messages.is_empty(), "Should have tool messages in history");

    // The tool result content should be truncated
    let tool_content = &tool_messages[0].content;
    assert!(
        tool_content.len() <= 9000, // Some overhead for JSON wrapping
        "Tool result should be truncated but was {} chars",
        tool_content.len()
    );
}

/// Test that PawanConfig max_retries can be set and applied
#[test]
fn test_config_max_retries() {
    let mut config = PawanConfig::default();
    assert_eq!(config.max_retries, 3, "Default max_retries should be 3");

    config.max_retries = 5;
    assert_eq!(config.max_retries, 5);

    // Test max_result_chars default
    assert_eq!(config.max_result_chars, 8000);
    config.max_result_chars = 16000;
    assert_eq!(config.max_result_chars, 16000);

    // Test max_context_tokens default
    assert_eq!(config.max_context_tokens, 100000);
}

/// Test session save and restore roundtrip with new fields
/// Test session save and restore roundtrip with new fields
#[test]
fn test_session_save_restore_roundtrip() {
    use pawan::agent::Message;
    use pawan::agent::Role;

    let mut session = Session::new("test-model");
    session.total_tokens = 42000;
    session.iteration_count = 7;
    session.messages.push(Message {
        role: Role::User,
        content: "test message".to_string(),
        tool_calls: vec![],
        tool_result: None,
    });

    // Save
    let path = session.save().unwrap();
    let id = session.id.clone();

    // Load and verify
    let loaded = Session::load(&id).unwrap();
    assert_eq!(loaded.model, "test-model");
    assert_eq!(loaded.total_tokens, 42000);
    assert_eq!(loaded.iteration_count, 7);
    assert_eq!(loaded.messages.len(), 1);
    assert_eq!(loaded.messages[0].content, "test message");

    // Clean up
    std::fs::remove_file(path).ok();
}

/// Test that Prompt permission blocks write bash commands in headless mode
#[tokio::test]
async fn test_prompt_permission_blocks_write_bash() {
    use pawan::agent::backend::mock::{MockBackend, MockResponse};
    use pawan::config::ToolPermission;

    let temp_dir = TempDir::new().unwrap();
    let mut config = PawanConfig::default();
    config.permissions.insert("bash".to_string(), ToolPermission::Prompt);

    let backend = MockBackend::new(vec![
        // LLM tries a write command
        MockResponse::tool_call("bash", json!({"command": "rm -rf ./build"})),
        MockResponse::text("I need approval for that."),
    ]);

    let mut agent = PawanAgent::new(config, temp_dir.path().to_path_buf())
        .with_backend(Box::new(backend));

    let response = agent.execute("Clean the build directory").await.unwrap();

    // Write command should be denied under Prompt in headless
    assert_eq!(response.tool_calls.len(), 1);
    assert!(!response.tool_calls[0].success);
}

/// Test that Prompt permission auto-allows read-only bash commands
#[tokio::test]
async fn test_prompt_permission_allows_read_only_bash() {
    use pawan::agent::backend::mock::{MockBackend, MockResponse};
    use pawan::config::ToolPermission;

    let temp_dir = TempDir::new().unwrap();
    let mut config = PawanConfig::default();
    config.permissions.insert("bash".to_string(), ToolPermission::Prompt);

    let backend = MockBackend::new(vec![
        // LLM tries a read-only command
        MockResponse::tool_call("bash", json!({"command": "ls -la"})),
        MockResponse::text("Here are the files."),
    ]);

    let mut agent = PawanAgent::new(config, temp_dir.path().to_path_buf())
        .with_backend(Box::new(backend));

    let response = agent.execute("List files").await.unwrap();

    // Read-only command should be auto-allowed under Prompt
    assert_eq!(response.tool_calls.len(), 1);
    assert!(response.tool_calls[0].success, "Read-only bash should succeed under Prompt permission");
}

/// Test bash validation blocks dangerous commands regardless of permission
#[tokio::test]
async fn test_bash_validation_blocks_dangerous() {
    use pawan::agent::backend::mock::{MockBackend, MockResponse};

    let temp_dir = TempDir::new().unwrap();
    let config = PawanConfig::default(); // Allow permission (default)

    let backend = MockBackend::new(vec![
        MockResponse::tool_call("bash", json!({"command": "rm -rf /"})),
        MockResponse::text("I couldn't do that."),
    ]);

    let mut agent = PawanAgent::new(config, temp_dir.path().to_path_buf())
        .with_backend(Box::new(backend));

    let response = agent.execute("Delete root").await.unwrap();

    // Bash validation should block even with Allow permission
    assert_eq!(response.tool_calls.len(), 1);
    assert!(!response.tool_calls[0].success, "Dangerous command should be blocked by validation");
}

/// Test ToolPermission::resolve defaults
#[test]
fn test_permission_resolve_integration() {
    use pawan::config::ToolPermission;
    use std::collections::HashMap;

    // Empty config: everything defaults to Allow
    let empty: HashMap<String, ToolPermission> = HashMap::new();
    assert_eq!(ToolPermission::resolve("bash", &empty), ToolPermission::Allow);
    assert_eq!(ToolPermission::resolve("read_file", &empty), ToolPermission::Allow);

    // Explicit overrides take precedence
    let mut perms = HashMap::new();
    perms.insert("bash".into(), ToolPermission::Prompt);
    perms.insert("write_file".into(), ToolPermission::Deny);
    assert_eq!(ToolPermission::resolve("bash", &perms), ToolPermission::Prompt);
    assert_eq!(ToolPermission::resolve("write_file", &perms), ToolPermission::Deny);
    assert_eq!(ToolPermission::resolve("read_file", &perms), ToolPermission::Allow);
}
