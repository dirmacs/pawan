//! Task tool: spawn an in-process subagent with restricted tools.
//!
//! This tool runs a child `PawanAgent` with a narrowed `ToolRegistry`, a smaller
//! context window, and a hard timeout. Subagents are depth-limited (they cannot
//! spawn other agents).

use super::Tool;
use crate::agent::backend::LlmBackend;
use crate::agent::PawanAgent;
use crate::config::PawanConfig;
use crate::tools::{bash, batch, edit, file, git, lsp_tool, mise, native, ToolRegistry, ToolTier};
use crate::{PawanError, Result};
use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{json, Value};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tokio::time::timeout;

const DEFAULT_TIMEOUT_SECS: u64 = 300;

#[derive(Debug, Clone, Deserialize)]
struct TaskArgs {
    agent: String,
    assignment: String,
    #[serde(default)]
    context: Option<String>,
    #[serde(default)]
    model: Option<String>,
    /// Timeout in seconds (default: 300).
    #[serde(default)]
    timeout: Option<u64>,
}

#[derive(Clone)]
pub struct TaskTool {
    workspace_root: PathBuf,
}

impl TaskTool {
    pub fn new(workspace_root: PathBuf) -> Self {
        Self { workspace_root }
    }

    fn known_agent_types() -> &'static [&'static str] {
        &[
            "explore",
            "plan",
            "task",
            "reviewer",
            "designer",
            "librarian",
        ]
    }

    fn validate_agent_type(agent: &str) -> std::result::Result<(), String> {
        if Self::known_agent_types().iter().any(|t| *t == agent) {
            Ok(())
        } else {
            Err(format!(
                "unknown agent type '{agent}'. Valid types: {}",
                Self::known_agent_types().join(", ")
            ))
        }
    }

    fn validate_assignment(assignment: &str) -> std::result::Result<(), String> {
        if assignment.trim().is_empty() {
            Err("assignment must be non-empty".to_string())
        } else {
            Ok(())
        }
    }

    fn system_prompt_for(agent: &str) -> String {
        match agent {
            "explore" => "You are a read-only exploration subagent. Use only the allowed read/search tools to gather facts. Do not propose or apply code edits. Return concise findings with file paths and evidence.".to_string(),
            "plan" => "You are an architecture subagent. Do not modify code. Make design decisions and propose an implementation plan with tradeoffs, invariants, and acceptance criteria.".to_string(),
            "reviewer" => "You are a code review subagent. Do not modify code. Identify bugs, security issues, and quality concerns. Return a structured review report with severity and recommendations.".to_string(),
            "designer" => "You are a UI/UX design subagent. If editing tools are available, you may implement UI changes carefully. Prioritize accessibility and consistency.".to_string(),
            "librarian" => "You are a research subagent. Verify details from authoritative sources and the local codebase. Do not modify code. Return actionable guidance.".to_string(),
            _ => "You are a subagent executing a delegated task. Follow the assignment precisely and return the final result. Do not spawn other agents.".to_string(),
        }
    }

    fn build_user_prompt(context: Option<&str>, assignment: &str) -> String {
        match context {
            Some(ctx) if !ctx.trim().is_empty() => format!(
                "{ctx}\n\n[Assignment]\n{assignment}\n\n[Constraints]\n- Subagent depth limit: you cannot spawn other agents.\n"
            ),
            _ => format!(
                "[Assignment]\n{assignment}\n\n[Constraints]\n- Subagent depth limit: you cannot spawn other agents.\n"
            ),
        }
    }

    fn registry_for(agent: &str, workspace_root: &PathBuf) -> ToolRegistry {
        use ToolTier::*;
        let mut reg = ToolRegistry::new();

        // Read/search tools
        reg.register_with_tier(
            Arc::new(file::ReadFileTool::new(workspace_root.clone())),
            Core,
        );
        reg.register_with_tier(
            Arc::new(file::ListDirectoryTool::new(workspace_root.clone())),
            Standard,
        );
        reg.register_with_tier(
            Arc::new(native::GlobSearchTool::new(workspace_root.clone())),
            Core,
        );
        reg.register_with_tier(
            Arc::new(native::GrepSearchTool::new(workspace_root.clone())),
            Core,
        );
        reg.register_with_tier(
            Arc::new(native::AstGrepTool::new(workspace_root.clone())),
            Core,
        );
        reg.register_with_tier(
            Arc::new(native::RipgrepTool::new(workspace_root.clone())),
            Extended,
        );
        reg.register_with_tier(
            Arc::new(native::FdTool::new(workspace_root.clone())),
            Extended,
        );

        match agent {
            "explore" | "plan" | "reviewer" | "librarian" => {
                reg.register_with_tier(
                    Arc::new(git::GitStatusTool::new(workspace_root.clone())),
                    Standard,
                );
                reg.register_with_tier(
                    Arc::new(git::GitDiffTool::new(workspace_root.clone())),
                    Standard,
                );
                reg.register_with_tier(
                    Arc::new(git::GitLogTool::new(workspace_root.clone())),
                    Standard,
                );
                reg.register_with_tier(
                    Arc::new(git::GitBlameTool::new(workspace_root.clone())),
                    Standard,
                );
                reg
            }
            "task" | "designer" => {
                reg.register_with_tier(Arc::new(bash::BashTool::new(workspace_root.clone())), Core);
                reg.register_with_tier(
                    Arc::new(file::WriteFileTool::new(workspace_root.clone())),
                    Core,
                );
                reg.register_with_tier(
                    Arc::new(edit::EditFileTool::new(workspace_root.clone())),
                    Core,
                );
                reg.register_with_tier(
                    Arc::new(edit::EditFileLinesTool::new(workspace_root.clone())),
                    Standard,
                );
                reg.register_with_tier(
                    Arc::new(edit::InsertAfterTool::new(workspace_root.clone())),
                    Standard,
                );
                reg.register_with_tier(
                    Arc::new(edit::AppendFileTool::new(workspace_root.clone())),
                    Standard,
                );

                reg.register_with_tier(
                    Arc::new(git::GitStatusTool::new(workspace_root.clone())),
                    Standard,
                );
                reg.register_with_tier(
                    Arc::new(git::GitDiffTool::new(workspace_root.clone())),
                    Standard,
                );
                reg.register_with_tier(
                    Arc::new(git::GitAddTool::new(workspace_root.clone())),
                    Standard,
                );
                reg.register_with_tier(
                    Arc::new(git::GitCommitTool::new(workspace_root.clone())),
                    Standard,
                );
                reg.register_with_tier(
                    Arc::new(git::GitLogTool::new(workspace_root.clone())),
                    Standard,
                );
                reg.register_with_tier(
                    Arc::new(git::GitBlameTool::new(workspace_root.clone())),
                    Standard,
                );
                reg.register_with_tier(
                    Arc::new(git::GitBranchTool::new(workspace_root.clone())),
                    Standard,
                );
                reg.register_with_tier(
                    Arc::new(git::GitCheckoutTool::new(workspace_root.clone())),
                    Standard,
                );
                reg.register_with_tier(
                    Arc::new(git::GitStashTool::new(workspace_root.clone())),
                    Standard,
                );

                reg.register_with_tier(
                    Arc::new(batch::BatchTool::new(workspace_root.clone())),
                    Standard,
                );
                reg.register_with_tier(
                    Arc::new(lsp_tool::LspTool::new(workspace_root.clone())),
                    Extended,
                );
                reg.register_with_tier(
                    Arc::new(mise::MiseTool::new(workspace_root.clone())),
                    Extended,
                );
                reg.register_with_tier(
                    Arc::new(native::SdTool::new(workspace_root.clone())),
                    Extended,
                );
                reg.register_with_tier(
                    Arc::new(native::ErdTool::new(workspace_root.clone())),
                    Extended,
                );
                reg
            }
            _ => reg,
        }
    }

    async fn run_subagent(
        &self,
        agent_type: &str,
        assignment: &str,
        context: Option<&str>,
        model: Option<&str>,
        timeout_secs: u64,
        backend_override: Option<Box<dyn LlmBackend>>,
    ) -> Result<Value> {
        let mut config = PawanConfig::default();
        config.system_prompt = Some(Self::system_prompt_for(agent_type));
        config.max_context_tokens = 32_000;
        config.max_tool_iterations = 20;
        config.eruka.enabled = false;
        if let Some(m) = model {
            config.model = m.to_string();
        }

        let tools = Self::registry_for(agent_type, &self.workspace_root);
        let prompt = Self::build_user_prompt(context, assignment);

        let mut agent = PawanAgent::new(config, self.workspace_root.clone()).with_tools(tools);
        if let Some(backend) = backend_override {
            agent = agent.with_backend(backend);
        }

        let run = agent.execute(&prompt);
        let response = match timeout(Duration::from_secs(timeout_secs), run).await {
            Ok(res) => res.map_err(|e| PawanError::Tool(format!("subagent error: {e}")))?,
            Err(_) => {
                return Ok(json!({
                    "agent": agent_type,
                    "status": "error",
                    "result": format!("subagent timeout after {timeout_secs}s"),
                }));
            }
        };

        Ok(json!({
            "agent": agent_type,
            "status": "completed",
            "result": response.content,
            "usage": {
                "prompt_tokens": response.usage.prompt_tokens,
                "completion_tokens": response.usage.completion_tokens,
                "total_tokens": response.usage.total_tokens,
                "reasoning_tokens": response.usage.reasoning_tokens,
                "action_tokens": response.usage.action_tokens,
            }
        }))
    }
}

#[async_trait]
impl Tool for TaskTool {
    fn name(&self) -> &str {
        "task"
    }

    fn description(&self) -> &str {
        "Spawn an in-process subagent with restricted tools to complete an assignment."
    }

    fn mutating(&self) -> bool {
        true
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "agent": {"type": "string"},
                "assignment": {"type": "string"},
                "context": {"type": "string"},
                "model": {"type": "string"},
                "timeout": {"type": "integer"}
            },
            "required": ["agent", "assignment"]
        })
    }

    async fn execute(&self, args: Value) -> Result<Value> {
        let parsed: TaskArgs = serde_json::from_value(args)
            .map_err(|e| PawanError::Tool(format!("invalid task args: {e}")))?;

        Self::validate_agent_type(&parsed.agent).map_err(PawanError::Tool)?;
        Self::validate_assignment(&parsed.assignment).map_err(PawanError::Tool)?;

        let timeout_secs = parsed.timeout.unwrap_or(DEFAULT_TIMEOUT_SECS);

        self.run_subagent(
            &parsed.agent,
            &parsed.assignment,
            parsed.context.as_deref(),
            parsed.model.as_deref(),
            timeout_secs,
            None,
        )
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::backend::mock::{MockBackend, MockResponse};
    use serde_json::json;

    #[tokio::test]
    async fn unknown_agent_type_rejects() {
        let dir = tempfile::tempdir().unwrap();
        let tool = TaskTool::new(dir.path().to_path_buf());
        let err = tool
            .execute(json!({"agent": "nope", "assignment": "hi"}))
            .await
            .unwrap_err();
        assert!(err.to_string().contains("unknown agent type"));
    }

    #[tokio::test]
    async fn timeout_returns_error_status() {
        let dir = tempfile::tempdir().unwrap();
        let tool = TaskTool::new(dir.path().to_path_buf());

        let out = tool
            .run_subagent(
                "explore",
                "This will time out immediately.",
                None,
                None,
                0,
                None,
            )
            .await
            .unwrap();

        assert_eq!(out["status"].as_str().unwrap(), "error");
        assert!(out["result"].as_str().unwrap().contains("timeout"));
    }

    #[tokio::test]
    async fn explore_agent_runs_and_returns_findings_with_mock_backend() {
        let dir = tempfile::tempdir().unwrap();
        let tool = TaskTool::new(dir.path().to_path_buf());

        let backend = Box::new(MockBackend::new(vec![MockResponse::text(
            "Findings: crates/pawan-core/src/lib.rs is the crate root.",
        )]));

        let out = tool
            .run_subagent(
                "explore",
                "Explore the repo and return findings.",
                Some("Context here"),
                None,
                5,
                Some(backend),
            )
            .await
            .unwrap();

        assert_eq!(out["agent"].as_str().unwrap(), "explore");
        assert_eq!(out["status"].as_str().unwrap(), "completed");
        assert!(out["result"].as_str().unwrap().contains("Findings:"));
    }
}
