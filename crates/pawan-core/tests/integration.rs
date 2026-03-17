//! Integration tests for Pawan with mocked Ollama server
//!
//! These tests use wiremock to simulate Ollama API responses,
//! allowing us to test the full agent workflow without a real LLM.
#![allow(dead_code)]

use pawan::agent::{PawanAgent, Role};
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

    let definitions = registry.get_definitions();

    // Should have 19 tools (11 base + 5 git + 2 sub-agent + edit_file_lines)
    assert_eq!(definitions.len(), 19);

    // Each definition should have name, description, and parameters
    for def in &definitions {
        assert!(!def.name.is_empty());
        assert!(!def.description.is_empty());
        assert!(def.parameters.is_object());
    }
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

    assert_eq!(result["file_count"].as_u64().unwrap(), 1);
    assert_eq!(result["total_matches"].as_u64().unwrap(), 1);
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

    assert_eq!(config.model, "stepfun-ai/step-3.5-flash");
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
    // Default model is StepFun (not deepseek), so thinking mode is off
    assert!(!config.use_thinking_mode());
    // But with a deepseek model, thinking mode activates
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
    assert!(config.get_target("ares").is_some());
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

    // Should have all tools (19 total)
    assert_eq!(definitions.len(), 19);

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
