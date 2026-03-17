//! Sub-agent spawning tool
//!
//! Spawns a pawan subprocess to handle a task independently.
//! This is the OMO replacement — enables multi-agent orchestration.

use super::Tool;
use crate::{PawanError, Result};
use async_trait::async_trait;
use serde_json::{json, Value};
use std::path::PathBuf;
use std::process::Stdio;
use tokio::io::AsyncReadExt;
use tokio::process::Command;
use std::io::Write;
use tracing;

/// Tool for spawning a sub-agent (pawan subprocess)
pub struct SpawnAgentTool {
    workspace_root: PathBuf,
}

impl SpawnAgentTool {
    pub fn new(workspace_root: PathBuf) -> Self {
        Self { workspace_root }
    }

    /// Find the pawan binary — tries cargo target first, then PATH
    fn find_pawan_binary(&self) -> String {
        // Check for debug/release binary in workspace target
        for candidate in &[
            self.workspace_root.join("target/release/pawan"),
            self.workspace_root.join("target/debug/pawan"),
        ] {
            if candidate.exists() {
                return candidate.to_string_lossy().to_string();
            }
        }
        // Fall back to PATH
        "pawan".to_string()
    }
}

#[async_trait]
impl Tool for SpawnAgentTool {
    fn name(&self) -> &str {
        "spawn_agent"
    }

    fn description(&self) -> &str {
        "Spawn a sub-agent (pawan subprocess) to handle a task independently. \
         Returns the agent's response as JSON. Use this for parallel or delegated tasks."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "prompt": {
                    "type": "string",
                    "description": "The task/prompt for the sub-agent"
                },
                "model": {
                    "type": "string",
                    "description": "Model to use (optional, defaults to parent's model)"
                },
                "timeout": {
                    "type": "integer",
                    "description": "Timeout in seconds (default: 120)"
                },
                "workspace": {
                    "type": "string",
                    "description": "Workspace directory for the sub-agent (default: same as parent)"
                },
                "retries": {
                    "type": "integer",
                    "description": "Number of retry attempts on failure (default: 0, max: 2)"
                }
            },
            "required": ["prompt"]
        })
    }

    async fn execute(&self, args: Value) -> Result<Value> {
        let prompt = args["prompt"]
            .as_str()
            .ok_or_else(|| PawanError::Tool("prompt is required for spawn_agent".into()))?;

        let timeout = args["timeout"].as_u64().unwrap_or(120);
        let model = args["model"].as_str();
        let workspace = args["workspace"]
            .as_str()
            .map(PathBuf::from)
            .unwrap_or_else(|| self.workspace_root.clone());
        let max_retries = args["retries"].as_u64().unwrap_or(0).min(2) as usize;

        // Generate unique agent ID for progress tracking
        let agent_id = uuid::Uuid::new_v4().to_string()[..8].to_string();
        let status_path = format!("/tmp/pawan-agent-{}.status", agent_id);
        let started_at = chrono::Utc::now().to_rfc3339();

        let pawan_bin = self.find_pawan_binary();

        for attempt in 0..=max_retries {
            let mut cmd = Command::new(&pawan_bin);
            cmd.arg("run")
                .arg("-o")
                .arg("json")
                .arg("--timeout")
                .arg(timeout.to_string())
                .arg("-w")
                .arg(workspace.to_string_lossy().to_string());

            if let Some(m) = model {
                cmd.arg("-m").arg(m);
            }

            cmd.arg(prompt);

            cmd.stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .stdin(Stdio::null());

            // Write initial status
            if let Ok(mut f) = std::fs::File::create(&status_path) {
                let _ = write!(f, r#"{{"state":"running","prompt":"{}","started_at":"{}","attempt":{}}}"#,
                    prompt.chars().take(100).collect::<String>().replace('"', "'"), started_at, attempt + 1);
            }

            let mut child = cmd.spawn().map_err(|e| {
                PawanError::Tool(format!(
                    "Failed to spawn sub-agent: {}. Binary: {}",
                    e, pawan_bin
                ))
            })?;

            let mut stdout = String::new();
            let mut stderr = String::new();

            if let Some(mut handle) = child.stdout.take() {
                handle.read_to_string(&mut stdout).await.ok();
            }
            if let Some(mut handle) = child.stderr.take() {
                handle.read_to_string(&mut stderr).await.ok();
            }

            let status = child.wait().await.map_err(PawanError::Io)?;

            let result = if let Ok(json_result) = serde_json::from_str::<Value>(&stdout) {
                json_result
            } else {
                json!({
                    "content": stdout.trim(),
                    "raw_output": true
                })
            };

            if status.success() || attempt == max_retries {
                // Update status file with completion
                let duration_ms = chrono::Utc::now().signed_duration_since(chrono::DateTime::parse_from_rfc3339(&started_at).unwrap_or_default()).num_milliseconds();
                if let Ok(mut f) = std::fs::File::create(&status_path) {
                    let state = if status.success() { "done" } else { "failed" };
                    let _ = write!(f, r#"{{"state":"{}","exit_code":{},"duration_ms":{},"attempt":{}}}"#,
                        state, status.code().unwrap_or(-1), duration_ms, attempt + 1);
                }

                return Ok(json!({
                    "success": status.success(),
                    "attempt": attempt + 1,
                    "total_attempts": attempt + 1,
                    "result": result,
                    "stderr": stderr.trim(),
                }));
            }
            // Failed but retries remaining — continue loop
            // Failed but retries remaining — continue loop
            tracing::warn!(attempt = attempt + 1, "spawn_agent attempt failed, retrying");
        }

        // Should not reach here, but satisfy the compiler
        Err(PawanError::Tool("spawn_agent: all retry attempts exhausted".into()))
    }
}

/// Tool for spawning multiple sub-agents in parallel
pub struct SpawnAgentsTool {
    workspace_root: PathBuf,
}

impl SpawnAgentsTool {
    pub fn new(workspace_root: PathBuf) -> Self {
        Self { workspace_root }
    }
}

#[async_trait]
impl Tool for SpawnAgentsTool {
    fn name(&self) -> &str {
        "spawn_agents"
    }

    fn description(&self) -> &str {
        "Spawn multiple sub-agents in parallel. Each task runs concurrently and results are returned as an array."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "tasks": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "properties": {
                            "prompt": {"type": "string"},
                            "model": {"type": "string"},
                            "timeout": {"type": "integer"},
                            "workspace": {"type": "string"}
                        },
                        "required": ["prompt"]
                    }
                }
            },
            "required": ["tasks"]
        })
    }

    async fn execute(&self, args: Value) -> Result<Value> {
        let tasks = args["tasks"]
            .as_array()
            .ok_or_else(|| PawanError::Tool("tasks array is required for spawn_agents".into()))?;

        let single_tool = SpawnAgentTool::new(self.workspace_root.clone());

        let futures: Vec<_> = tasks
            .iter()
            .map(|task| single_tool.execute(task.clone()))
            .collect();

        let results = futures::future::join_all(futures).await;

        let output: Vec<Value> = results
            .into_iter()
            .map(|r| match r {
                Ok(v) => v,
                Err(e) => json!({"success": false, "error": e.to_string()}),
            })
            .collect();

        Ok(json!({
            "success": true,
            "results": output,
            "total_tasks": tasks.len(),
        }))
    }
}