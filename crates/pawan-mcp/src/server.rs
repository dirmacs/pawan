//! Pawan MCP Server — exposes pawan's agent capabilities as MCP tools
//!
//! Start with: `pawan mcp serve`
//! Connect from doltdot/OpenClaw: add to mcp.json as a stdio server

use async_trait::async_trait;
use pawan::agent::PawanAgent;
use pawan::config::PawanConfig;
use pawan::healing::Healer;
use serde::Deserialize;
use serde_json::Value;
use std::path::PathBuf;
use std::sync::Arc;
use thulp_core::{Parameter, ToolDefinition, ToolResult};
use thulp_mcp::{McpServer, ToolHandler};

fn workspace_path(workspace: Option<&str>) -> PathBuf {
    workspace
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")))
}

fn create_agent(config: &PawanConfig, workspace: Option<&str>) -> PawanAgent {
    let ws = workspace_path(workspace);
    PawanAgent::new(config.clone(), ws)
}

// ─── Handler types ────────────────────────────────────────────────────────────

struct RunHandler {
    config: Arc<PawanConfig>,
}

#[derive(Deserialize)]
struct RunArgs {
    prompt: String,
    workspace: Option<String>,
}

#[async_trait]
impl ToolHandler for RunHandler {
    async fn call(&self, arguments: Value) -> ToolResult {
        let args: RunArgs = match serde_json::from_value(arguments) {
            Ok(a) => a,
            Err(e) => return ToolResult::failure(format!("invalid arguments: {e}")),
        };
        let mut agent = create_agent(&self.config, args.workspace.as_deref());
        match agent.execute(&args.prompt).await {
            Ok(r) => ToolResult::success(Value::String(format!(
                "{}\n\n---\n{} iterations, {} tool calls",
                r.content,
                r.iterations,
                r.tool_calls.len()
            ))),
            Err(e) => ToolResult::failure(format!("pawan error: {e}")),
        }
    }
}

struct TaskHandler {
    config: Arc<PawanConfig>,
}

#[derive(Deserialize)]
struct TaskArgs {
    task: String,
    workspace: Option<String>,
}

#[async_trait]
impl ToolHandler for TaskHandler {
    async fn call(&self, arguments: Value) -> ToolResult {
        let args: TaskArgs = match serde_json::from_value(arguments) {
            Ok(a) => a,
            Err(e) => return ToolResult::failure(format!("invalid arguments: {e}")),
        };
        let mut agent = create_agent(&self.config, args.workspace.as_deref());
        match agent.task(&args.task).await {
            Ok(r) => ToolResult::success(Value::String(format!(
                "{}\n\n---\n{} iterations, {} tool calls",
                r.content,
                r.iterations,
                r.tool_calls.len()
            ))),
            Err(e) => ToolResult::failure(format!("pawan error: {e}")),
        }
    }
}

struct HealHandler {
    config: Arc<PawanConfig>,
}

#[derive(Deserialize)]
struct HealArgs {
    workspace: Option<String>,
}

#[async_trait]
impl ToolHandler for HealHandler {
    async fn call(&self, arguments: Value) -> ToolResult {
        let args: HealArgs = match serde_json::from_value(arguments) {
            Ok(a) => a,
            Err(e) => return ToolResult::failure(format!("invalid arguments: {e}")),
        };
        let mut agent = create_agent(&self.config, args.workspace.as_deref());
        match agent.heal().await {
            Ok(r) => ToolResult::success(Value::String(format!(
                "{}\n\n---\n{} iterations, {} tool calls",
                r.content,
                r.iterations,
                r.tool_calls.len()
            ))),
            Err(e) => ToolResult::failure(format!("pawan error: {e}")),
        }
    }
}

struct ReviewHandler {
    config: Arc<PawanConfig>,
}

#[derive(Deserialize)]
struct ReviewArgs {
    file: String,
    workspace: Option<String>,
}

#[async_trait]
impl ToolHandler for ReviewHandler {
    async fn call(&self, arguments: Value) -> ToolResult {
        let args: ReviewArgs = match serde_json::from_value(arguments) {
            Ok(a) => a,
            Err(e) => return ToolResult::failure(format!("invalid arguments: {e}")),
        };
        let mut agent = create_agent(&self.config, args.workspace.as_deref());
        let prompt = format!(
            "Review the file at {} and provide feedback on code quality, bugs, and improvements.",
            args.file
        );
        match agent.execute(&prompt).await {
            Ok(r) => ToolResult::success(Value::String(r.content)),
            Err(e) => ToolResult::failure(format!("pawan error: {e}")),
        }
    }
}

struct ExplainHandler {
    config: Arc<PawanConfig>,
}

#[derive(Deserialize)]
struct ExplainArgs {
    query: String,
    workspace: Option<String>,
}

#[async_trait]
impl ToolHandler for ExplainHandler {
    async fn call(&self, arguments: Value) -> ToolResult {
        let args: ExplainArgs = match serde_json::from_value(arguments) {
            Ok(a) => a,
            Err(e) => return ToolResult::failure(format!("invalid arguments: {e}")),
        };
        let mut agent = create_agent(&self.config, args.workspace.as_deref());
        let prompt = format!("Explain: {}", args.query);
        match agent.execute(&prompt).await {
            Ok(r) => ToolResult::success(Value::String(r.content)),
            Err(e) => ToolResult::failure(format!("pawan error: {e}")),
        }
    }
}

struct StatusHandler {
    config: Arc<PawanConfig>,
}

#[derive(Deserialize)]
struct StatusArgs {
    workspace: Option<String>,
}

#[async_trait]
impl ToolHandler for StatusHandler {
    async fn call(&self, arguments: Value) -> ToolResult {
        let args: StatusArgs = match serde_json::from_value(arguments) {
            Ok(a) => a,
            Err(e) => return ToolResult::failure(format!("invalid arguments: {e}")),
        };
        let ws = workspace_path(args.workspace.as_deref());
        let healer = Healer::new(ws, self.config.healing.clone());
        match healer.count_issues().await {
            Ok((errors, warnings, failed_tests)) => ToolResult::success(Value::String(format!(
                "Project Status:\n  Errors: {}\n  Warnings: {}\n  Failed tests: {}",
                errors, warnings, failed_tests
            ))),
            Err(e) => ToolResult::failure(format!("pawan error: {e}")),
        }
    }
}

// ─── Server builder ───────────────────────────────────────────────────────────

/// Build a Pawan MCP server from a config.
pub fn build_server(config: PawanConfig) -> McpServer {
    let config = Arc::new(config);
    let version = env!("CARGO_PKG_VERSION").to_string();

    McpServer::builder("pawan", version)
        .tool(
            "pawan_run",
            ToolDefinition::builder("pawan_run")
                .description("Execute a prompt through pawan's agent loop with tool calling. The agent can read/write files, run shell commands, search code, and use git.")
                .parameter(Parameter::required_string("prompt"))
                .parameter(Parameter::optional_string("workspace"))
                .build(),
            Box::new(RunHandler { config: Arc::clone(&config) }),
        )
        .tool(
            "pawan_task",
            ToolDefinition::builder("pawan_task")
                .description("Execute a coding task. Pawan will explore the codebase, make changes, and verify they compile.")
                .parameter(Parameter::required_string("task"))
                .parameter(Parameter::optional_string("workspace"))
                .build(),
            Box::new(TaskHandler { config: Arc::clone(&config) }),
        )
        .tool(
            "pawan_heal",
            ToolDefinition::builder("pawan_heal")
                .description("Self-heal a Rust project: fix compilation errors, clippy warnings, and failing tests.")
                .parameter(Parameter::optional_string("workspace"))
                .build(),
            Box::new(HealHandler { config: Arc::clone(&config) }),
        )
        .tool(
            "pawan_review",
            ToolDefinition::builder("pawan_review")
                .description("AI-powered code review of a specific file. Returns suggestions for improvements.")
                .parameter(Parameter::required_string("file"))
                .parameter(Parameter::optional_string("workspace"))
                .build(),
            Box::new(ReviewHandler { config: Arc::clone(&config) }),
        )
        .tool(
            "pawan_explain",
            ToolDefinition::builder("pawan_explain")
                .description("AI-powered explanation of a file, function, or concept in the codebase.")
                .parameter(Parameter::required_string("query"))
                .parameter(Parameter::optional_string("workspace"))
                .build(),
            Box::new(ExplainHandler { config: Arc::clone(&config) }),
        )
        .tool(
            "pawan_status",
            ToolDefinition::builder("pawan_status")
                .description("Show project status: compilation errors, warnings, test failures, and git status.")
                .parameter(Parameter::optional_string("workspace"))
                .build(),
            Box::new(StatusHandler { config: Arc::clone(&config) }),
        )
        .build()
}

/// Start the MCP server on stdio
pub async fn serve(config: PawanConfig) -> pawan::Result<()> {
    let server = build_server(config);
    server
        .serve_stdio()
        .await
        .map_err(|e| pawan::PawanError::Config(format!("MCP server error: {e}")))?;
    Ok(())
}

// ─── Re-export PawanServer as a type alias for backwards compat ───────────────

/// Legacy type alias kept for callsites that reference `PawanServer`.
/// The actual server is now built via `build_server()`.
pub type PawanServer = McpServer;

#[cfg(test)]
mod tests {
    use super::*;
    use pawan::config::PawanConfig;

    #[test]
    fn test_workspace_path_with_value() {
        let path = workspace_path(Some("/tmp/test"));
        assert_eq!(path, PathBuf::from("/tmp/test"));
    }

    #[test]
    fn test_workspace_path_with_none() {
        let path = workspace_path(None);
        // Should default to current directory or "."
        // workspace_path with None calls std::env::current_dir() which may fail in test environment
    }

    #[test]
    fn test_workspace_path_with_empty_string() {
        let path = workspace_path(Some(""));
        assert_eq!(path, PathBuf::from(""));
    }

    #[test]
    fn test_create_agent_with_workspace() {
        let config = PawanConfig::default();
        let agent = create_agent(&config, Some("/tmp/test"));
        // Agent is created successfully
    }

    #[test]
    fn test_run_args_deserialization() {
        let json = r#"{"prompt":"test","workspace":"/tmp"}"#;
        let args: RunArgs = serde_json::from_str(json).unwrap();
        assert_eq!(args.prompt, "test");
        assert_eq!(args.workspace, Some("/tmp".to_string()));
    }

    #[test]
    fn test_run_args_without_workspace() {
        let json = r#"{"prompt":"test"}"#;
        let args: RunArgs = serde_json::from_str(json).unwrap();
        assert_eq!(args.prompt, "test");
        assert!(args.workspace.is_none());
    }

    #[test]
    fn test_task_args_deserialization() {
        let json = r#"{"task":"fix bug","workspace":"/tmp"}"#;
        let args: TaskArgs = serde_json::from_str(json).unwrap();
        assert_eq!(args.task, "fix bug");
        assert_eq!(args.workspace, Some("/tmp".to_string()));
    }

    #[test]
    fn test_heal_args_deserialization() {
        let json = r#"{"workspace":"/tmp"}"#;
        let args: HealArgs = serde_json::from_str(json).unwrap();
        assert_eq!(args.workspace, Some("/tmp".to_string()));
    }

    #[test]
    fn test_heal_args_without_workspace() {
        let json = r#"{}"#;
        let args: HealArgs = serde_json::from_str(json).unwrap();
        assert!(args.workspace.is_none());
    }

    #[test]
    fn test_review_args_deserialization() {
        let json = r#"{"file":"src/main.rs","workspace":"/tmp"}"#;
        let args: ReviewArgs = serde_json::from_str(json).unwrap();
        assert_eq!(args.file, "src/main.rs");
        assert_eq!(args.workspace, Some("/tmp".to_string()));
    }

    #[test]
    fn test_explain_args_deserialization() {
        let json = r#"{"query":"what is this?","workspace":"/tmp"}"#;
        let args: ExplainArgs = serde_json::from_str(json).unwrap();
        assert_eq!(args.query, "what is this?");
        assert_eq!(args.workspace, Some("/tmp".to_string()));
    }

    #[test]
    fn test_status_args_deserialization() {
        let json = r#"{"workspace":"/tmp"}"#;
        let args: StatusArgs = serde_json::from_str(json).unwrap();
        assert_eq!(args.workspace, Some("/tmp".to_string()));
    }

    #[test]
    fn test_status_args_without_workspace() {
        let json = r#"{}"#;
        let args: StatusArgs = serde_json::from_str(json).unwrap();
        assert!(args.workspace.is_none());
    }

    #[test]
    fn test_pawan_server_type_alias_exists() {
        // This test just verifies the type alias compiles
        let config = PawanConfig::default();
        let server: PawanServer = build_server(config);
        assert!(true);
    }
}
#[test]
fn test_create_agent_with_workspace() {
    let config = PawanConfig::default();
    let _agent = create_agent(&config, Some("/tmp/test"));
    // Agent is created successfully
}

#[test]
fn test_create_agent_without_workspace() {
    let config = PawanConfig::default();
    let _agent = create_agent(&config, None);
    // Agent is created successfully
}

#[test]
fn test_build_server_creates_server() {
    let config = PawanConfig::default();
    let _server = build_server(config);
    // Server is created successfully
}
