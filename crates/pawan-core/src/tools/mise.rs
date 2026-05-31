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

    fn resolve_mise_bin() -> crate::Result<String> {
        if binary_exists("mise") {
            Ok("mise".to_string())
        } else {
            let home = std::env::var("HOME").unwrap_or_else(|_| "/root".into());
            let local = format!("{}/.local/bin/mise", home);
            if std::path::Path::new(&local).exists() {
                Ok(local)
            } else {
                Err(crate::PawanError::Tool(
                    "mise not found. Install: curl https://mise.run | sh".into(),
                ))
            }
        }
    }

    fn parse_mise_action(args: &Value) -> crate::Result<&str> {
        args["action"]
            .as_str()
            .ok_or_else(|| crate::PawanError::Tool("action required".into()))
    }

    fn build_mise_command(action: &str, args: &Value, global: bool) -> crate::Result<Vec<String>> {
        match action {
            "install" => Self::mise_install(args),
            "uninstall" => Self::mise_uninstall(args),
            "upgrade" => Self::mise_upgrade(args),
            "list" => Ok(Self::mise_list()),
            "search" => Self::mise_search(args),
            "use" => Self::mise_use(args, global),
            "outdated" => Self::mise_outdated(args),
            "prune" => Self::mise_prune(args),
            "exec" => Self::mise_exec(args),
            "run" => Self::mise_run(args),
            "watch" => Self::mise_watch(args),
            "tasks" => Ok(Self::mise_tasks()),
            "env" => Ok(Self::mise_env()),
            "doctor" => Ok(Self::mise_doctor()),
            "self-update" => Ok(Self::mise_self_update()),
            "trust" => Self::mise_trust(args),
            _ => Err(crate::PawanError::Tool(format!(
                "Unknown action: {action}. See tool description for available actions."
            ))),
        }
    }

    fn mise_install(args: &Value) -> crate::Result<Vec<String>> {
        let tool = args["tool"]
            .as_str()
            .ok_or_else(|| crate::PawanError::Tool("tool required for install".into()))?;
        Ok(vec!["install".into(), tool.into(), "-y".into()])
    }

    fn mise_uninstall(args: &Value) -> crate::Result<Vec<String>> {
        let tool = args["tool"]
            .as_str()
            .ok_or_else(|| crate::PawanError::Tool("tool required for uninstall".into()))?;
        Ok(vec!["uninstall".into(), tool.into()])
    }

    fn mise_upgrade(args: &Value) -> crate::Result<Vec<String>> {
        let mut v = vec!["upgrade".into()];
        if let Some(tool) = args["tool"].as_str() {
            v.push(tool.into());
        }
        Ok(v)
    }

    fn mise_list() -> Vec<String> {
        vec!["ls".into()]
    }

    fn mise_search(args: &Value) -> crate::Result<Vec<String>> {
        let tool = args["tool"]
            .as_str()
            .ok_or_else(|| crate::PawanError::Tool("tool required for search".into()))?;
        Ok(vec!["registry".into(), tool.into()])
    }

    fn mise_use(args: &Value, global: bool) -> crate::Result<Vec<String>> {
        let tool = args["tool"]
            .as_str()
            .ok_or_else(|| crate::PawanError::Tool("tool required for use".into()))?;
        let mut v = vec!["use".into()];
        if global {
            v.push("--global".into());
        }
        v.push(tool.into());
        Ok(v)
    }

    fn mise_outdated(args: &Value) -> crate::Result<Vec<String>> {
        let mut v = vec!["outdated".into()];
        if let Some(tool) = args["tool"].as_str() {
            v.push(tool.into());
        }
        Ok(v)
    }

    fn mise_prune(args: &Value) -> crate::Result<Vec<String>> {
        let mut v = vec!["prune".into(), "-y".into()];
        if let Some(tool) = args["tool"].as_str() {
            v.push(tool.into());
        }
        Ok(v)
    }

    fn mise_exec(args: &Value) -> crate::Result<Vec<String>> {
        let tool = args["tool"]
            .as_str()
            .ok_or_else(|| crate::PawanError::Tool("tool required for exec".into()))?;
        let extra = args["args"].as_str().unwrap_or("");
        let mut v = vec!["exec".into(), tool.into(), "--".into()];
        if !extra.is_empty() {
            v.extend(extra.split_whitespace().map(|s| s.to_string()));
        }
        Ok(v)
    }

    fn mise_run(args: &Value) -> crate::Result<Vec<String>> {
        let task = args["task"]
            .as_str()
            .ok_or_else(|| crate::PawanError::Tool("task required for run".into()))?;
        let mut v = vec!["run".into(), task.into()];
        if let Some(extra) = args["args"].as_str() {
            v.push("--".into());
            v.extend(extra.split_whitespace().map(|s| s.to_string()));
        }
        Ok(v)
    }

    fn mise_watch(args: &Value) -> crate::Result<Vec<String>> {
        let task = args["task"]
            .as_str()
            .ok_or_else(|| crate::PawanError::Tool("task required for watch".into()))?;
        let mut v = vec!["watch".into(), task.into()];
        if let Some(extra) = args["args"].as_str() {
            v.push("--".into());
            v.extend(extra.split_whitespace().map(|s| s.to_string()));
        }
        Ok(v)
    }

    fn mise_tasks() -> Vec<String> {
        vec!["tasks".into(), "ls".into()]
    }

    fn mise_env() -> Vec<String> {
        vec!["env".into()]
    }

    fn mise_doctor() -> Vec<String> {
        vec!["doctor".into()]
    }

    fn mise_self_update() -> Vec<String> {
        vec!["self-update".into(), "-y".into()]
    }

    fn mise_trust(args: &Value) -> crate::Result<Vec<String>> {
        let mut v = vec!["trust".into()];
        if let Some(extra) = args["args"].as_str() {
            v.push(extra.into());
        }
        Ok(v)
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
        let mise_bin = Self::resolve_mise_bin()?;
        let action = Self::parse_mise_action(&args)?;
        let global = args["global"].as_bool().unwrap_or(false);
        let cmd_args = Self::build_mise_command(action, &args, global)?;

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
