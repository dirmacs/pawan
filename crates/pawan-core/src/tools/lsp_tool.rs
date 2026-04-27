//! ast-grep and LSP (rust-analyzer) tool wrappers.

use super::native_search::{ensure_binary, run_cmd};
use super::Tool;
use async_trait::async_trait;
use serde_json::{json, Value};
use std::path::PathBuf;
use std::process::Stdio;

// ─── ast-grep ────────────────────────────────────────────────────────────────

pub struct AstGrepTool {
    workspace_root: PathBuf,
}

impl AstGrepTool {
    pub fn new(workspace_root: PathBuf) -> Self {
        Self { workspace_root }
    }
}

#[async_trait]
impl Tool for AstGrepTool {
    fn name(&self) -> &str {
        "ast_grep"
    }

    fn description(&self) -> &str {
        "ast-grep — structural code search and rewrite using AST patterns. \
         Unlike regex, this matches code by syntax tree structure. Use $NAME for \
         single-node wildcards, $$$ARGS for variadic (multiple nodes). \
         Actions: 'search' finds matches, 'rewrite' transforms them in-place. \
         Examples: pattern='fn $NAME($$$ARGS)' finds all functions. \
         pattern='$EXPR.unwrap()' rewrite='$EXPR?' replaces unwrap with ?. \
         Supports: rust, python, javascript, typescript, go, c, cpp, java."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["search", "rewrite"],
                    "description": "search: find matching code. rewrite: transform matching code in-place."
                },
                "pattern": {
                    "type": "string",
                    "description": "AST pattern to match. Use $VAR for wildcards, $$$VAR for variadic. e.g. 'fn $NAME($$$ARGS) -> $RET { $$$ }'"
                },
                "rewrite": {
                    "type": "string",
                    "description": "Replacement pattern (only for action=rewrite). Use captured $VARs. e.g. '$EXPR?'"
                },
                "path": {
                    "type": "string",
                    "description": "File or directory to search/rewrite"
                },
                "lang": {
                    "type": "string",
                    "description": "Language: rust, python, javascript, typescript, go, c, cpp, java (default: auto-detect)"
                }
            },
            "required": ["action", "pattern", "path"]
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
                    .description("search: find matching code. rewrite: transform matching code in-place.")
                    .build(),
            )
            .parameter(
                Parameter::builder("pattern")
                    .param_type(ParameterType::String)
                    .required(true)
                    .description("AST pattern to match. Use $VAR for wildcards, $$$VAR for variadic. e.g. 'fn $NAME($$$ARGS) -> $RET { $$$ }'")
                    .build(),
            )
            .parameter(
                Parameter::builder("rewrite")
                    .param_type(ParameterType::String)
                    .required(false)
                    .description("Replacement pattern (only for action=rewrite). Use captured $VARs. e.g. '$EXPR?'")
                    .build(),
            )
            .parameter(
                Parameter::builder("path")
                    .param_type(ParameterType::String)
                    .required(true)
                    .description("File or directory to search/rewrite")
                    .build(),
            )
            .parameter(
                Parameter::builder("lang")
                    .param_type(ParameterType::String)
                    .required(false)
                    .description("Language: rust, python, javascript, typescript, go, c, cpp, java (default: auto-detect)")
                    .build(),
            )
            .build()
    }

    async fn execute(&self, args: Value) -> crate::Result<Value> {
        ensure_binary("ast-grep", &self.workspace_root).await?;

        let action = args["action"]
            .as_str()
            .ok_or_else(|| crate::PawanError::Tool("action required (search or rewrite)".into()))?;
        let pattern = args["pattern"]
            .as_str()
            .ok_or_else(|| crate::PawanError::Tool("pattern required".into()))?;
        let path = args["path"]
            .as_str()
            .ok_or_else(|| crate::PawanError::Tool("path required".into()))?;

        let mut cmd_args: Vec<String> = vec!["run".into()];

        if let Some(lang) = args["lang"].as_str() {
            cmd_args.push("--lang".into());
            cmd_args.push(lang.into());
        }

        cmd_args.push("--pattern".into());
        cmd_args.push(pattern.into());

        match action {
            "search" => {
                cmd_args.push(path.into());
            }
            "rewrite" => {
                let rewrite = args["rewrite"].as_str().ok_or_else(|| {
                    crate::PawanError::Tool("rewrite pattern required for action=rewrite".into())
                })?;
                cmd_args.push("--rewrite".into());
                cmd_args.push(rewrite.into());
                cmd_args.push("--update-all".into());
                cmd_args.push(path.into());
            }
            _ => {
                return Err(crate::PawanError::Tool(format!(
                    "Unknown action: {}. Use 'search' or 'rewrite'",
                    action
                )));
            }
        }

        let cmd_refs: Vec<&str> = cmd_args.iter().map(|s| s.as_str()).collect();
        let (stdout, stderr, success) = run_cmd("ast-grep", &cmd_refs, &self.workspace_root)
            .await
            .map_err(crate::PawanError::Tool)?;

        let match_count = stdout
            .lines()
            .filter(|l| l.starts_with('/') || l.contains("│"))
            .count();

        Ok(json!({
            "success": success,
            "action": action,
            "matches": match_count,
            "output": stdout,
            "stderr": if stderr.is_empty() { None::<String> } else { Some(stderr) }
        }))
    }
}

// ─── LSP (rust-analyzer powered code intelligence) ──────────────────────────

pub struct LspTool {
    workspace_root: PathBuf,
}

impl LspTool {
    pub fn new(workspace_root: PathBuf) -> Self {
        Self { workspace_root }
    }
}

#[async_trait]
impl Tool for LspTool {
    fn name(&self) -> &str {
        "lsp"
    }

    fn description(&self) -> &str {
        "LSP code intelligence via rust-analyzer. Provides type-aware code understanding \
         that grep/ast-grep can't: diagnostics without cargo check, structural search with \
         type info, symbol extraction, and analysis stats. Actions: diagnostics (find errors), \
         search (structural pattern search), ssr (structural search+replace with types), \
         symbols (parse file symbols), analyze (project-wide type stats)."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["diagnostics", "search", "ssr", "symbols", "analyze"],
                    "description": "diagnostics: find errors/warnings in project. \
                                    search: structural pattern search (e.g. '$a.foo($b)'). \
                                    ssr: structural search+replace (e.g. '$a.unwrap() ==>> $a?'). \
                                    symbols: parse file and list symbols. \
                                    analyze: project-wide type analysis stats."
                },
                "pattern": {
                    "type": "string",
                    "description": "For search: pattern like '$a.foo($b)'. For ssr: rule like '$a.unwrap() ==>> $a?'"
                },
                "path": {
                    "type": "string",
                    "description": "Project path (directory with Cargo.toml) for diagnostics/analyze. File path for symbols."
                },
                "severity": {
                    "type": "string",
                    "enum": ["error", "warning", "info", "hint"],
                    "description": "Minimum severity for diagnostics (default: warning)"
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
                    .description("diagnostics: find errors/warnings in project. \
                                  search: structural pattern search (e.g. '$a.foo($b)'). \
                                  ssr: structural search+replace (e.g. '$a.unwrap() ==>> $a?'). \
                                  symbols: parse file and list symbols. \
                                  analyze: project-wide type analysis stats.")
                    .build(),
            )
            .parameter(
                Parameter::builder("pattern")
                    .param_type(ParameterType::String)
                    .required(false)
                    .description("For search: pattern like '$a.foo($b)'. For ssr: rule like '$a.unwrap() ==>> $a?'")
                    .build(),
            )
            .parameter(
                Parameter::builder("path")
                    .param_type(ParameterType::String)
                    .required(false)
                    .description("Project path (directory with Cargo.toml) for diagnostics/analyze. File path for symbols.")
                    .build(),
            )
            .parameter(
                Parameter::builder("severity")
                    .param_type(ParameterType::String)
                    .required(false)
                    .description("Minimum severity for diagnostics (default: warning)")
                    .build(),
            )
            .build()
    }

    async fn execute(&self, args: Value) -> crate::Result<Value> {
        ensure_binary("rust-analyzer", &self.workspace_root).await?;

        let action = args["action"]
            .as_str()
            .ok_or_else(|| crate::PawanError::Tool("action required".into()))?;

        let timeout_dur = std::time::Duration::from_secs(60);

        match action {
            "diagnostics" => {
                let path = args["path"]
                    .as_str()
                    .unwrap_or(self.workspace_root.to_str().unwrap_or("."));
                let mut cmd_args = vec!["diagnostics", path];
                if let Some(sev) = args["severity"].as_str() {
                    cmd_args.extend(["--severity", sev]);
                }
                let result = tokio::time::timeout(
                    timeout_dur,
                    run_cmd("rust-analyzer", &cmd_args, &self.workspace_root),
                )
                .await;
                match result {
                    Ok(Ok((stdout, stderr, success))) => Ok(json!({
                        "success": success,
                        "diagnostics": stdout,
                        "count": stdout.lines().filter(|l| !l.is_empty()).count(),
                        "stderr": if stderr.is_empty() { None::<String> } else { Some(stderr) }
                    })),
                    Ok(Err(e)) => Err(crate::PawanError::Tool(e)),
                    Err(_) => Err(crate::PawanError::Tool(
                        "rust-analyzer diagnostics timed out (60s)".into(),
                    )),
                }
            }
            "search" => {
                let pattern = args["pattern"]
                    .as_str()
                    .ok_or_else(|| crate::PawanError::Tool("pattern required for search".into()))?;
                let result = tokio::time::timeout(
                    timeout_dur,
                    run_cmd("rust-analyzer", &["search", pattern], &self.workspace_root),
                )
                .await;
                match result {
                    Ok(Ok((stdout, stderr, success))) => Ok(json!({
                        "success": success,
                        "matches": stdout,
                        "count": stdout.lines().filter(|l| !l.is_empty()).count(),
                        "stderr": if stderr.is_empty() { None::<String> } else { Some(stderr) }
                    })),
                    Ok(Err(e)) => Err(crate::PawanError::Tool(e)),
                    Err(_) => Err(crate::PawanError::Tool(
                        "rust-analyzer search timed out (60s)".into(),
                    )),
                }
            }
            "ssr" => {
                let pattern = args["pattern"].as_str().ok_or_else(|| {
                    crate::PawanError::Tool(
                        "pattern required for ssr (format: '$a.unwrap() ==>> $a?')".into(),
                    )
                })?;
                let result = tokio::time::timeout(
                    timeout_dur,
                    run_cmd("rust-analyzer", &["ssr", pattern], &self.workspace_root),
                )
                .await;
                match result {
                    Ok(Ok((stdout, stderr, success))) => Ok(json!({
                        "success": success,
                        "output": stdout,
                        "stderr": if stderr.is_empty() { None::<String> } else { Some(stderr) }
                    })),
                    Ok(Err(e)) => Err(crate::PawanError::Tool(e)),
                    Err(_) => Err(crate::PawanError::Tool(
                        "rust-analyzer ssr timed out (60s)".into(),
                    )),
                }
            }
            "symbols" => {
                let path = args["path"]
                    .as_str()
                    .ok_or_else(|| crate::PawanError::Tool("path required for symbols".into()))?;
                let full_path = if std::path::Path::new(path).is_absolute() {
                    PathBuf::from(path)
                } else {
                    self.workspace_root.join(path)
                };
                let content = tokio::fs::read_to_string(&full_path).await.map_err(|e| {
                    crate::PawanError::Tool(format!("Failed to read {}: {}", path, e))
                })?;

                let mut child = tokio::process::Command::new("rust-analyzer")
                    .arg("symbols")
                    .stdin(Stdio::piped())
                    .stdout(Stdio::piped())
                    .stderr(Stdio::piped())
                    .spawn()
                    .map_err(|e| {
                        crate::PawanError::Tool(format!("Failed to spawn rust-analyzer: {}", e))
                    })?;

                if let Some(mut stdin) = child.stdin.take() {
                    use tokio::io::AsyncWriteExt;
                    let _ = stdin.write_all(content.as_bytes()).await;
                    drop(stdin);
                }

                let output = child.wait_with_output().await.map_err(|e| {
                    crate::PawanError::Tool(format!("rust-analyzer symbols failed: {}", e))
                })?;

                let stdout = String::from_utf8_lossy(&output.stdout).to_string();
                Ok(json!({
                    "success": output.status.success(),
                    "symbols": stdout,
                    "count": stdout.lines().filter(|l| !l.is_empty()).count()
                }))
            }
            "analyze" => {
                let path = args["path"]
                    .as_str()
                    .unwrap_or(self.workspace_root.to_str().unwrap_or("."));
                let result = tokio::time::timeout(
                    timeout_dur,
                    run_cmd(
                        "rust-analyzer",
                        &["analysis-stats", "--skip-inference", path],
                        &self.workspace_root,
                    ),
                )
                .await;
                match result {
                    Ok(Ok((stdout, stderr, success))) => Ok(json!({
                        "success": success,
                        "stats": stdout,
                        "stderr": if stderr.is_empty() { None::<String> } else { Some(stderr) }
                    })),
                    Ok(Err(e)) => Err(crate::PawanError::Tool(e)),
                    Err(_) => Err(crate::PawanError::Tool(
                        "rust-analyzer analysis-stats timed out (60s)".into(),
                    )),
                }
            }
            _ => Err(crate::PawanError::Tool(format!(
                "Unknown action: {action}. Use diagnostics/search/ssr/symbols/analyze"
            ))),
        }
    }
}

// ─── tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[tokio::test]
    async fn test_ast_grep_tool_schema() {
        let tmp = TempDir::new().unwrap();
        let tool = AstGrepTool::new(tmp.path().to_path_buf());
        assert_eq!(tool.name(), "ast_grep");
        let schema = tool.parameters_schema();
        assert!(schema["properties"]["action"].is_object());
        assert!(schema["properties"]["pattern"].is_object());
    }

    #[tokio::test]
    async fn test_lsp_tool_schema() {
        let tmp = TempDir::new().unwrap();
        let tool = LspTool::new(tmp.path().to_path_buf());
        assert_eq!(tool.name(), "lsp");
        let schema = tool.parameters_schema();
        assert!(schema["properties"]["action"].is_object());
    }
}
