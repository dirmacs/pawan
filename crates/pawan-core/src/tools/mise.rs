//! mise and zoxide tool wrappers.

use super::native_search::{binary_exists, run_cmd};
use super::Tool;
use async_trait::async_trait;
use serde_json::{json, Value};
use std::path::PathBuf;

// ─── mise (universal tool installer) ────────────────────────────────────────

pub struct MiseTool {
    workspace_root: PathBuf,
}

impl MiseTool {
    pub fn new(workspace_root: PathBuf) -> Self {
        Self { workspace_root }
    }
}

#[async_trait]
impl Tool for MiseTool {
    fn name(&self) -> &str {
        "mise"
    }

    fn description(&self) -> &str {
        "mise — polyglot tool manager, environment manager, and task runner. Replaces asdf, nvm, \
         pyenv, direnv, make, and npm scripts. Three powers: (1) install/manage any dev tool or \
         language runtime, (2) manage per-project env vars, (3) run/watch project tasks. \
         Pawan should use this to self-install any missing CLI tool (erd, ast-grep, fd, rg, etc)."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": [
                        "install", "uninstall", "upgrade", "list", "use", "search",
                        "exec", "run", "tasks", "env", "outdated", "prune",
                        "doctor", "self-update", "trust", "watch"
                    ],
                    "description": "Tool management: install, uninstall, upgrade, list, use, search, outdated, prune. \
                                    Execution: exec (run with tool env), run (run a task), watch (rerun task on file change). \
                                    Environment: env (show/set env vars). Tasks: tasks (list/manage tasks). \
                                    Maintenance: doctor, self-update, trust, prune."
                },
                "tool": {
                    "type": "string",
                    "description": "Tool name with optional version. Examples: 'erdtree', 'node@22', 'python@3.12', \
                                    'ast-grep', 'ripgrep', 'fd', 'sd', 'bat', 'delta', 'jq', 'yq', 'go', 'bun', 'deno'"
                },
                "task": {
                    "type": "string",
                    "description": "Task name for run/watch/tasks actions (defined in mise.toml or .mise/tasks/)"
                },
                "args": {
                    "type": "string",
                    "description": "Additional arguments (space-separated). For exec: command to run. For run: task args."
                },
                "global": {
                    "type": "boolean",
                    "description": "Apply globally (--global flag) instead of project-local. Default: false."
                }
            },
            "required": ["action"]
        })
    }

    fn thulp_definition(&self) -> thulp_core::ToolDefinition {
        use thulp_core::{Parameter, ParameterType};
        thulp_core::ToolDefinition::builder(self.name())
            .description(self.description())
            .parameter(
                Parameter::builder("action")
                    .param_type(ParameterType::String)
                    .required(true)
                    .description("Tool management: install, uninstall, upgrade, list, use, search, outdated, prune. \
                                  Execution: exec (run with tool env), run (run a task), watch (rerun task on file change). \
                                  Environment: env (show/set env vars). Tasks: tasks (list/manage tasks). \
                                  Maintenance: doctor, self-update, trust, prune.")
                    .build(),
            )
            .parameter(
                Parameter::builder("tool")
                    .param_type(ParameterType::String)
                    .required(false)
                    .description("Tool name with optional version. Examples: 'erdtree', 'node@22', 'python@3.12', \
                                  'ast-grep', 'ripgrep', 'fd', 'sd', 'bat', 'delta', 'jq', 'yq', 'go', 'bun', 'deno'")
                    .build(),
            )
            .parameter(
                Parameter::builder("task")
                    .param_type(ParameterType::String)
                    .required(false)
                    .description("Task name for run/watch/tasks actions (defined in mise.toml or .mise/tasks/)")
                    .build(),
            )
            .parameter(
                Parameter::builder("args")
                    .param_type(ParameterType::String)
                    .required(false)
                    .description("Additional arguments (space-separated). For exec: command to run. For run: task args.")
                    .build(),
            )
            .parameter(
                Parameter::builder("global")
                    .param_type(ParameterType::Boolean)
                    .required(false)
                    .description("Apply globally (--global flag) instead of project-local. Default: false.")
                    .build(),
            )
            .build()
    }

    async fn execute(&self, args: Value) -> crate::Result<Value> {
        let mise_bin = if binary_exists("mise") {
            "mise".to_string()
        } else {
            let home = std::env::var("HOME").unwrap_or_else(|_| "/root".into());
            let local = format!("{}/.local/bin/mise", home);
            if std::path::Path::new(&local).exists() {
                local
            } else {
                return Err(crate::PawanError::Tool(
                    "mise not found. Install: curl https://mise.run | sh".into(),
                ));
            }
        };

        let action = args["action"]
            .as_str()
            .ok_or_else(|| crate::PawanError::Tool("action required".into()))?;
        let global = args["global"].as_bool().unwrap_or(false);

        let cmd_args: Vec<String> = match action {
            "install" => {
                let tool = args["tool"]
                    .as_str()
                    .ok_or_else(|| crate::PawanError::Tool("tool required for install".into()))?;
                vec!["install".into(), tool.into(), "-y".into()]
            }
            "uninstall" => {
                let tool = args["tool"]
                    .as_str()
                    .ok_or_else(|| crate::PawanError::Tool("tool required for uninstall".into()))?;
                vec!["uninstall".into(), tool.into()]
            }
            "upgrade" => {
                let mut v = vec!["upgrade".into()];
                if let Some(tool) = args["tool"].as_str() {
                    v.push(tool.into());
                }
                v
            }
            "list" => vec!["ls".into()],
            "search" => {
                let tool = args["tool"]
                    .as_str()
                    .ok_or_else(|| crate::PawanError::Tool("tool required for search".into()))?;
                vec!["registry".into(), tool.into()]
            }
            "use" => {
                let tool = args["tool"]
                    .as_str()
                    .ok_or_else(|| crate::PawanError::Tool("tool required for use".into()))?;
                let mut v = vec!["use".into()];
                if global {
                    v.push("--global".into());
                }
                v.push(tool.into());
                v
            }
            "outdated" => {
                let mut v = vec!["outdated".into()];
                if let Some(tool) = args["tool"].as_str() {
                    v.push(tool.into());
                }
                v
            }
            "prune" => {
                let mut v = vec!["prune".into(), "-y".into()];
                if let Some(tool) = args["tool"].as_str() {
                    v.push(tool.into());
                }
                v
            }
            "exec" => {
                let tool = args["tool"]
                    .as_str()
                    .ok_or_else(|| crate::PawanError::Tool("tool required for exec".into()))?;
                let extra = args["args"].as_str().unwrap_or("");
                let mut v = vec!["exec".into(), tool.into(), "--".into()];
                if !extra.is_empty() {
                    v.extend(extra.split_whitespace().map(|s| s.to_string()));
                }
                v
            }
            "run" => {
                let task = args["task"]
                    .as_str()
                    .ok_or_else(|| crate::PawanError::Tool("task required for run".into()))?;
                let mut v = vec!["run".into(), task.into()];
                if let Some(extra) = args["args"].as_str() {
                    v.push("--".into());
                    v.extend(extra.split_whitespace().map(|s| s.to_string()));
                }
                v
            }
            "watch" => {
                let task = args["task"]
                    .as_str()
                    .ok_or_else(|| crate::PawanError::Tool("task required for watch".into()))?;
                let mut v = vec!["watch".into(), task.into()];
                if let Some(extra) = args["args"].as_str() {
                    v.push("--".into());
                    v.extend(extra.split_whitespace().map(|s| s.to_string()));
                }
                v
            }
            "tasks" => vec!["tasks".into(), "ls".into()],
            "env" => vec!["env".into()],
            "doctor" => vec!["doctor".into()],
            "self-update" => vec!["self-update".into(), "-y".into()],
            "trust" => {
                let mut v = vec!["trust".into()];
                if let Some(extra) = args["args"].as_str() {
                    v.push(extra.into());
                }
                v
            }
            _ => {
                return Err(crate::PawanError::Tool(format!(
                    "Unknown action: {action}. See tool description for available actions."
                )))
            }
        };

        let cmd_refs: Vec<&str> = cmd_args.iter().map(|s| s.as_str()).collect();
        let (stdout, stderr, success) = run_cmd(&mise_bin, &cmd_refs, &self.workspace_root)
            .await
            .map_err(crate::PawanError::Tool)?;

        Ok(json!({
            "success": success,
            "action": action,
            "output": stdout,
            "stderr": if stderr.is_empty() { None::<String> } else { Some(stderr) }
        }))
    }
}

// ─── zoxide (smart cd) ─────────────────────────────────────────────────────

pub struct ZoxideTool {
    workspace_root: PathBuf,
}

impl ZoxideTool {
    pub fn new(workspace_root: PathBuf) -> Self {
        Self { workspace_root }
    }
}

#[async_trait]
impl Tool for ZoxideTool {
    fn name(&self) -> &str {
        "z"
    }

    fn description(&self) -> &str {
        "zoxide — smart directory jumper. Learns from your cd history. \
         Use 'query' to find a directory by fuzzy match (e.g. 'myproject' finds ~/projects/myproject). \
         Use 'add' to teach it a new path. Use 'list' to see known paths."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "action": { "type": "string", "description": "query, add, or list" },
                "path": { "type": "string", "description": "Path or search term" }
            },
            "required": ["action"]
        })
    }

    fn thulp_definition(&self) -> thulp_core::ToolDefinition {
        use thulp_core::{Parameter, ParameterType};
        thulp_core::ToolDefinition::builder(self.name())
            .description(self.description())
            .parameter(
                Parameter::builder("action")
                    .param_type(ParameterType::String)
                    .required(true)
                    .description("query, add, or list")
                    .build(),
            )
            .parameter(
                Parameter::builder("path")
                    .param_type(ParameterType::String)
                    .required(false)
                    .description("Path or search term")
                    .build(),
            )
            .build()
    }

    async fn execute(&self, args: Value) -> crate::Result<Value> {
        let action = args["action"]
            .as_str()
            .ok_or_else(|| crate::PawanError::Tool("action required (query/add/list)".into()))?;

        let cmd_args: Vec<String> = match action {
            "query" => {
                let path = args["path"].as_str().ok_or_else(|| {
                    crate::PawanError::Tool("path/search term required for query".into())
                })?;
                vec!["query".into(), path.into()]
            }
            "add" => {
                let path = args["path"]
                    .as_str()
                    .ok_or_else(|| crate::PawanError::Tool("path required for add".into()))?;
                vec!["add".into(), path.into()]
            }
            "list" => vec!["query".into(), "--list".into()],
            _ => {
                return Err(crate::PawanError::Tool(format!(
                    "Unknown action: {}. Use query/add/list",
                    action
                )))
            }
        };

        let cmd_refs: Vec<&str> = cmd_args.iter().map(|s| s.as_str()).collect();
        let (stdout, stderr, success) = run_cmd("zoxide", &cmd_refs, &self.workspace_root)
            .await
            .map_err(crate::PawanError::Tool)?;

        Ok(json!({
            "success": success,
            "result": stdout.trim(),
            "stderr": if stderr.is_empty() { None::<String> } else { Some(stderr) }
        }))
    }
}

// ─── tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[tokio::test]
    async fn test_mise_tool_schema() {
        let tmp = TempDir::new().unwrap();
        let tool = MiseTool::new(tmp.path().to_path_buf());
        assert_eq!(tool.name(), "mise");
        let schema = tool.parameters_schema();
        assert!(schema["properties"]["action"].is_object());
        assert!(schema["properties"]["task"].is_object());
    }

    #[tokio::test]
    async fn test_zoxide_tool_basics() {
        let tmp = TempDir::new().unwrap();
        let tool = ZoxideTool::new(tmp.path().into());
        assert_eq!(tool.name(), "z");
        assert!(!tool.description().is_empty());
        let schema = tool.parameters_schema();
        assert!(schema["required"]
            .as_array()
            .unwrap()
            .contains(&serde_json::json!("action")));
    }

    #[tokio::test]
    async fn test_mise_tool_unknown_action_returns_error() {
        let tmp = TempDir::new().unwrap();
        let tool = MiseTool::new(tmp.path().into());
        let result = tool
            .execute(serde_json::json!({ "action": "totally_not_a_real_verb" }))
            .await;
        let err = result.expect_err("unknown mise action must error");
        let msg = format!("{}", err);
        assert!(
            (msg.contains("Unknown action") && msg.contains("totally_not_a_real_verb"))
                || msg.contains("mise not found"),
            "error must name the unknown action (or report mise missing), got: {}",
            msg
        );
    }

    #[tokio::test]
    async fn test_mise_tool_install_without_tool_returns_error() {
        let tmp = TempDir::new().unwrap();
        let tool = MiseTool::new(tmp.path().into());
        let result = tool
            .execute(serde_json::json!({ "action": "install" }))
            .await;
        let err = result.expect_err("mise install without tool must error");
        let msg = format!("{}", err);
        assert!(
            msg.contains("tool required for install") || msg.contains("mise not found"),
            "error should mention 'tool required for install' (or report mise missing), got: {}",
            msg
        );
    }
}
