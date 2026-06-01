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
use futures::stream::{self, StreamExt};
use serde::Deserialize;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::Semaphore;
use tokio::time::timeout;

use crate::subagent::SubagentHandle;

const DEFAULT_TIMEOUT_SECS: u64 = 300;

const MAX_PARALLEL_SUBAGENTS: usize = 4;

#[derive(Debug, Clone, Deserialize)]
struct TaskItem {
    agent: String,
    assignment: String,
    #[serde(default)]
    context: Option<String>,
    #[serde(default)]
    description: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct TaskArgs {
    /// Single-task mode: agent type.
    #[serde(default)]
    agent: Option<String>,
    /// Single-task mode: assignment text.
    #[serde(default)]
    assignment: Option<String>,
    /// Parallel mode: one or more subagent jobs (max 8).
    #[serde(default)]
    tasks: Option<Vec<TaskItem>>,
    #[serde(default)]
    context: Option<String>,
    #[serde(default)]
    description: Option<String>,
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
        if Self::known_agent_types().contains(&agent) {
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

    fn registry_for(agent: &str, workspace_root: &Path) -> ToolRegistry {
        use ToolTier::*;
        let workspace_root = workspace_root.to_path_buf();
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

    fn short_label(description: Option<&str>, assignment: &str) -> String {
        description
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .unwrap_or_else(|| {
                let one_line = assignment.lines().next().unwrap_or(assignment).trim();
                if one_line.chars().count() > 48 {
                    format!("{}…", one_line.chars().take(45).collect::<String>())
                } else {
                    one_line.to_string()
                }
            })
    }

    fn aggregate_batch_results(results: Vec<(usize, Result<Value>)>) -> Value {
        let mut succeeded = 0usize;
        let mut failed = 0usize;
        let mut items = Vec::with_capacity(results.len());

        for (index, result) in results {
            match result {
                Ok(v) => {
                    let status = v.get("status").and_then(|s| s.as_str()).unwrap_or("error");
                    if status == "completed" {
                        succeeded += 1;
                    } else {
                        failed += 1;
                    }
                    items.push(json!({
                        "index": index,
                        "status": status,
                        "agent": v.get("agent"),
                        "result": v.get("result"),
                        "duration_ms": v.get("duration_ms"),
                        "usage": v.get("usage"),
                        "subagent_id": v.get("subagent_id"),
                    }));
                }
                Err(e) => {
                    failed += 1;
                    items.push(json!({
                        "index": index,
                        "status": "error",
                        "result": e.to_string(),
                    }));
                }
            }
        }

        let total = items.len();
        json!({
            "mode": "batch",
            "success": failed == 0,
            "total": total,
            "succeeded": succeeded,
            "failed": failed,
            "results": items,
        })
    }

    #[allow(clippy::too_many_arguments)]
    async fn run_subagent(
        &self,
        agent_type: &str,
        assignment: &str,
        context: Option<&str>,
        model: Option<&str>,
        timeout_secs: u64,
        label: &str,
        backend_override: Option<Box<dyn LlmBackend>>,
    ) -> Result<Value> {
        let started = Instant::now();
        let handle = SubagentHandle::start(label, "task", Some(agent_type.to_string()));

        let mut config = PawanConfig {
            system_prompt: Some(Self::system_prompt_for(agent_type)),
            max_context_tokens: 32_000,
            max_tool_iterations: 20,
            ..Default::default()
        };
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

        let progress = handle.clone();
        let on_tool_start: crate::agent::ToolStartCallback = Box::new(move |name: &str| {
            progress.set_tool(name);
        });
        let progress_done = handle.clone();
        let on_tool: crate::agent::ToolCallback = Box::new(move |_record| {
            progress_done.clear_tool();
        });

        let run = agent.execute_with_callbacks(&prompt, None, Some(on_tool), Some(on_tool_start));
        let response = match timeout(Duration::from_secs(timeout_secs), run).await {
            Ok(Ok(res)) => res,
            Ok(Err(e)) => {
                handle.complete_err(e.to_string());
                let duration_ms = started.elapsed().as_millis() as u64;
                let out = json!({
                    "agent": agent_type,
                    "status": "error",
                    "result": e.to_string(),
                    "duration_ms": duration_ms,
                    "subagent_id": handle.id(),
                });
                handle.dismiss();
                return Ok(out);
            }
            Err(_) => {
                handle.complete_err(format!("subagent timeout after {timeout_secs}s"));
                let duration_ms = started.elapsed().as_millis() as u64;
                let out = json!({
                    "agent": agent_type,
                    "status": "error",
                    "result": format!("subagent timeout after {timeout_secs}s"),
                    "duration_ms": duration_ms,
                    "subagent_id": handle.id(),
                });
                handle.dismiss();
                return Ok(out);
            }
        };

        handle.complete_ok();
        let duration_ms = started.elapsed().as_millis() as u64;
        let out = json!({
            "agent": agent_type,
            "status": "completed",
            "result": response.content,
            "duration_ms": duration_ms,
            "subagent_id": handle.id(),
            "usage": {
                "prompt_tokens": response.usage.prompt_tokens,
                "completion_tokens": response.usage.completion_tokens,
                "total_tokens": response.usage.total_tokens,
                "reasoning_tokens": response.usage.reasoning_tokens,
                "action_tokens": response.usage.action_tokens,
            }
        });
        handle.dismiss();
        Ok(out)
    }

    async fn run_tasks_parallel(
        &self,
        tasks: Vec<TaskItem>,
        model: Option<&str>,
        timeout_secs: u64,
    ) -> Result<Value> {
        if tasks.is_empty() {
            return Ok(json!({
                "mode": "batch",
                "success": true,
                "total": 0,
                "succeeded": 0,
                "failed": 0,
                "results": [],
            }));
        }

        let semaphore = Arc::new(Semaphore::new(MAX_PARALLEL_SUBAGENTS));
        let model = model.map(str::to_string);

        let results: Vec<(usize, Result<Value>)> = stream::iter(tasks.into_iter().enumerate())
            .map(|(index, item)| {
                let sem = Arc::clone(&semaphore);
                let tool = self.clone();
                let model = model.clone();
                async move {
                    let _permit = sem.acquire().await.expect("semaphore");
                    let label =
                        TaskTool::short_label(item.description.as_deref(), &item.assignment);
                    let result = tool
                        .run_subagent(
                            &item.agent,
                            &item.assignment,
                            item.context.as_deref(),
                            model.as_deref(),
                            timeout_secs,
                            &label,
                            None,
                        )
                        .await;
                    (index, result)
                }
            })
            .buffered(MAX_PARALLEL_SUBAGENTS)
            .collect()
            .await;

        Ok(Self::aggregate_batch_results(results))
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
                "agent": {"type": "string", "description": "Agent type (single-task mode)"},
                "assignment": {"type": "string", "description": "Assignment (single-task mode)"},
                "tasks": {
                    "type": "array",
                    "description": "Parallel subagents (max 8). Each item needs agent + assignment.",
                    "items": {
                        "type": "object",
                        "properties": {
                            "agent": {"type": "string"},
                            "assignment": {"type": "string"},
                            "context": {"type": "string"},
                            "description": {"type": "string", "description": "Short label for TUI"}
                        },
                        "required": ["agent", "assignment"]
                    }
                },
                "context": {"type": "string"},
                "description": {"type": "string", "description": "Short label for TUI (single-task)"},
                "model": {"type": "string"},
                "timeout": {"type": "integer"}
            }
        })
    }

    async fn execute(&self, args: Value) -> Result<Value> {
        let parsed: TaskArgs = serde_json::from_value(args)
            .map_err(|e| PawanError::Tool(format!("invalid task args: {e}")))?;

        let timeout_secs = parsed.timeout.unwrap_or(DEFAULT_TIMEOUT_SECS);

        if let Some(tasks) = parsed.tasks {
            if tasks.len() > 8 {
                return Err(PawanError::Tool(
                    "task tool accepts at most 8 parallel subagents".into(),
                ));
            }
            for item in &tasks {
                Self::validate_agent_type(&item.agent).map_err(PawanError::Tool)?;
                Self::validate_assignment(&item.assignment).map_err(PawanError::Tool)?;
            }
            return self
                .run_tasks_parallel(tasks, parsed.model.as_deref(), timeout_secs)
                .await;
        }

        let agent = parsed
            .agent
            .as_deref()
            .ok_or_else(|| PawanError::Tool("agent is required (or pass tasks array)".into()))?;
        let assignment = parsed.assignment.as_deref().ok_or_else(|| {
            PawanError::Tool("assignment is required (or pass tasks array)".into())
        })?;

        Self::validate_agent_type(agent).map_err(PawanError::Tool)?;
        Self::validate_assignment(assignment).map_err(PawanError::Tool)?;

        let label = Self::short_label(parsed.description.as_deref(), assignment);
        self.run_subagent(
            agent,
            assignment,
            parsed.context.as_deref(),
            parsed.model.as_deref(),
            timeout_secs,
            &label,
            None,
        )
        .await
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn batch_aggregate_counts_failures() {
        let results = vec![
            (0, Ok(json!({"status": "completed"}))),
            (1, Ok(json!({"status": "error"}))),
            (2, Err(PawanError::Tool("boom".into()))),
        ];
        let summary = TaskTool::aggregate_batch_results(results);
        assert_eq!(summary["succeeded"], 1);
        assert_eq!(summary["failed"], 2);
        assert_eq!(summary["success"], false);
    }

    use super::*;
    use crate::agent::backend::mock::{MockBackend, MockResponse};
    use serde_json::json;

    #[tokio::test]
    async fn unknown_agent_type_rejects() {
        let dir = tempfile::tempdir().unwrap();
        let tool = TaskTool::new(dir.path().to_path_buf());
        let err = tool
            .execute(json!({"agent": "nope", "assignment": "hi", "timeout": 5}))
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
                "timeout test",
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
                "explore repo",
                Some(backend),
            )
            .await
            .unwrap();

        assert_eq!(out["agent"].as_str().unwrap(), "explore");
        assert_eq!(out["status"].as_str().unwrap(), "completed");
        assert!(out["result"].as_str().unwrap().contains("Findings:"));
    }
}
