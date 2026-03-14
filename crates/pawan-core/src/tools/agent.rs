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

        let pawan_bin = self.find_pawan_binary();

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

        // Try to parse JSON output
        let result = if let Ok(json_result) = serde_json::from_str::<Value>(&stdout) {
            json_result
        } else {
            json!({
                "content": stdout.trim(),
                "raw_output": true
            })
        };

        if !status.success() {
            return Ok(json!({
                "success": false,
                "exit_code": status.code(),
                "result": result,
                "stderr": stderr.trim(),
            }));
        }

        Ok(json!({
            "success": true,
            "result": result,
        }))
    }
}
