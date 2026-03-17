//! Pawan MCP Server — exposes pawan's agent capabilities as MCP tools
//!
//! Start with: `pawan mcp serve`
//! Connect from doltdot/OpenClaw: add to mcp.json as a stdio server

use pawan::agent::PawanAgent;
use pawan::config::PawanConfig;
use pawan::healing::Healer;
use pawan::PawanError;
use rmcp::handler::server::tool::ToolRouter;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::*;
use rmcp::service::{RequestContext, RoleServer, ServiceExt};
use rmcp::{tool, tool_router, ErrorData as McpError, ServerHandler};
use schemars::JsonSchema;
use serde::Deserialize;
use std::path::PathBuf;

/// Request parameters for pawan_run tool
#[derive(Debug, Deserialize, JsonSchema)]
pub struct RunRequest {
    /// The prompt to execute
    pub prompt: String,
    /// Working directory (defaults to current dir)
    pub workspace: Option<String>,
}

/// Request parameters for pawan_task tool
#[derive(Debug, Deserialize, JsonSchema)]
pub struct TaskRequest {
    /// Description of the coding task
    pub task: String,
    /// Working directory (defaults to current dir)
    pub workspace: Option<String>,
}

/// Request parameters for pawan_heal tool
#[derive(Debug, Deserialize, JsonSchema)]
pub struct HealRequest {
    /// Working directory to heal (defaults to current dir)
    pub workspace: Option<String>,
}

/// Request parameters for pawan_review tool
#[derive(Debug, Deserialize, JsonSchema)]
pub struct ReviewRequest {
    /// File path to review
    pub file: String,
    /// Working directory (defaults to current dir)
    pub workspace: Option<String>,
}

/// Request parameters for pawan_explain tool
#[derive(Debug, Deserialize, JsonSchema)]
pub struct ExplainRequest {
    /// What to explain: file path, function name, or concept
    pub query: String,
    /// Working directory (defaults to current dir)
    pub workspace: Option<String>,
}

/// Request parameters for pawan_status tool
#[derive(Debug, Deserialize, JsonSchema)]
pub struct StatusRequest {
    /// Working directory (defaults to current dir)
    pub workspace: Option<String>,
}
/// Pawan MCP Server — wraps PawanAgent as MCP tools
#[derive(Clone)]
pub struct PawanServer {
    config: PawanConfig,
    #[allow(dead_code)]
    tool_router: ToolRouter<Self>,
}

fn workspace_path(workspace: Option<&str>) -> PathBuf {
    workspace
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")))
}

fn create_agent(config: &PawanConfig, workspace: Option<&str>) -> PawanAgent {
    let ws = workspace_path(workspace);
    PawanAgent::new(config.clone(), ws)
}

#[tool_router]
impl PawanServer {
    pub fn new(config: PawanConfig) -> Self {
        Self {
            config,
            tool_router: Self::tool_router(),
        }
    }

    #[tool(
        name = "pawan_run",
        description = "Execute a prompt through pawan's agent loop with tool calling. The agent can read/write files, run shell commands, search code, and use git."
    )]
    async fn run(
        &self,
        params: Parameters<RunRequest>,
    ) -> Result<CallToolResult, McpError> {
        let mut agent = create_agent(&self.config, params.0.workspace.as_deref());

        match agent.execute(&params.0.prompt).await {
            Ok(response) => Ok(CallToolResult::success(vec![Content::text(format!(
                "{}\n\n---\n{} iterations, {} tool calls",
                response.content,
                response.iterations,
                response.tool_calls.len()
            ))])),
            Err(e) => Ok(CallToolResult::error(vec![Content::text(format!(
                "pawan error: {}",
                e
            ))])),
        }
    }

    #[tool(
        name = "pawan_task",
        description = "Execute a coding task. Pawan will explore the codebase, make changes, and verify they compile."
    )]
    async fn task(
        &self,
        params: Parameters<TaskRequest>,
    ) -> Result<CallToolResult, McpError> {
        let mut agent = create_agent(&self.config, params.0.workspace.as_deref());

        match agent.task(&params.0.task).await {
            Ok(response) => Ok(CallToolResult::success(vec![Content::text(format!(
                "{}\n\n---\n{} iterations, {} tool calls",
                response.content,
                response.iterations,
                response.tool_calls.len()
            ))])),
            Err(e) => Ok(CallToolResult::error(vec![Content::text(format!(
                "pawan error: {}",
                e
            ))])),
        }
    }

    #[tool(
        name = "pawan_heal",
        description = "Self-heal a Rust project: fix compilation errors, clippy warnings, and failing tests."
    )]
    async fn heal(
        &self,
        params: Parameters<HealRequest>,
    ) -> Result<CallToolResult, McpError> {
        let mut agent = create_agent(&self.config, params.0.workspace.as_deref());

        match agent.heal().await {
            Ok(response) => Ok(CallToolResult::success(vec![Content::text(format!(
                "{}\n\n---\n{} iterations, {} tool calls",
                response.content,
                response.iterations,
                response.tool_calls.len()
            ))])),
            Err(e) => Ok(CallToolResult::error(vec![Content::text(format!(
                "pawan error: {}",
                e
            ))])),
        }
    }

    #[tool(
        name = "pawan_review",
        description = "AI-powered code review of a specific file. Returns suggestions for improvements."
    )]
    async fn review(
        &self,
        params: Parameters<ReviewRequest>,
    ) -> Result<CallToolResult, McpError> {
        let mut agent = create_agent(&self.config, params.0.workspace.as_deref());
        let prompt = format!("Review the file at {} and provide feedback on code quality, bugs, and improvements.", params.0.file);

        match agent.execute(&prompt).await {
            Ok(response) => Ok(CallToolResult::success(vec![Content::text(response.content)])),
            Err(e) => Ok(CallToolResult::error(vec![Content::text(format!("pawan error: {}", e))])),
        }
    }

    #[tool(
        name = "pawan_explain",
        description = "AI-powered explanation of a file, function, or concept in the codebase."
    )]
    async fn explain(
        &self,
        params: Parameters<ExplainRequest>,
    ) -> Result<CallToolResult, McpError> {
        let mut agent = create_agent(&self.config, params.0.workspace.as_deref());
        let prompt = format!("Explain: {}", params.0.query);

        match agent.execute(&prompt).await {
            Ok(response) => Ok(CallToolResult::success(vec![Content::text(response.content)])),
            Err(e) => Ok(CallToolResult::error(vec![Content::text(format!("pawan error: {}", e))])),
        }
    }

    #[tool(
        name = "pawan_status",
        description = "Show project status: compilation errors, warnings, test failures, and git status."
    )]
    async fn status(
        &self,
        params: Parameters<StatusRequest>,
    ) -> Result<CallToolResult, McpError> {
        let ws = workspace_path(params.0.workspace.as_deref());
        let healer = Healer::new(ws, self.config.healing.clone());

        match healer.count_issues().await {
            Ok((errors, warnings, failed_tests)) => {
                Ok(CallToolResult::success(vec![Content::text(format!(
                    "Project Status:\n  Errors: {}\n  Warnings: {}\n  Failed tests: {}",
                    errors, warnings, failed_tests
                ))]))
            }
            Err(e) => Ok(CallToolResult::error(vec![Content::text(format!("pawan error: {}", e))])),
        }
    }
}

impl ServerHandler for PawanServer {
    async fn initialize(
        &self,
        request: InitializeRequestParam,
        context: RequestContext<RoleServer>,
    ) -> Result<InitializeResult, McpError> {
        context.peer.set_peer_info(request);
        Ok(InitializeResult {
            protocol_version: ProtocolVersion::V_2024_11_05,
            capabilities: ServerCapabilities {
                tools: Some(ToolsCapability {
                    list_changed: None,
                }),
                ..Default::default()
            },
            server_info: Implementation {
                name: "pawan".into(),
                title: Some("Pawan CLI Coding Agent".into()),
                version: env!("CARGO_PKG_VERSION").into(),
                icons: None,
                website_url: Some("https://github.com/dirmacs/pawan".into()),
            },
            instructions: Some(
                "Pawan is a self-healing CLI coding agent. Use pawan_run for general prompts, pawan_task for coding tasks, pawan_heal to fix compilation errors, pawan_review for code review, pawan_explain for explanations, pawan_status for project health.".into(),
            ),
        })
    }
}

/// Start the MCP server on stdio
pub async fn serve(config: PawanConfig) -> pawan::Result<()> {
    let server = PawanServer::new(config);
    let transport = rmcp::transport::io::stdio();

    let service = server
        .serve(transport)
        .await
        .map_err(|e| PawanError::Config(format!("Failed to start MCP server: {}", e)))?;

    service
        .waiting()
        .await
        .map_err(|e| PawanError::Config(format!("MCP server error: {}", e)))?;

    Ok(())
}
