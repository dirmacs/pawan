//! Deagle code intelligence tools — graph-backed symbol search via deagle CLI.
//!
//! Deagle is a Rust-native code intelligence engine (tree-sitter + SQLite) that
//! indexes codebases into a graph database. Pawan shells out to the `deagle`
//! binary for symbol search, AST pattern matching, and keyword FTS5 ranking.
//!
//! These tools complement the native search tools (rg, grep, ast_grep) by
//! providing structured graph-aware results: symbol kinds, language detection,
//! and BM25 ranking for keyword relevance.
//!
//! All tools require `deagle` to be in PATH and the codebase to be indexed
//! (`deagle map .` run once in the workspace). The `deagle_map` tool can be
//! called by the agent to (re)index on demand.

use super::Tool;
use async_trait::async_trait;
use serde_json::{json, Value};
use std::path::PathBuf;
use std::process::Stdio;

/// Check if deagle binary is available.
fn deagle_available() -> bool {
    which::which("deagle").is_ok()
}

/// Run a deagle subcommand and capture output.
async fn run_deagle(
    args: &[&str],
    cwd: &std::path::Path,
) -> Result<(String, String, bool), String> {
    let output = tokio::process::Command::new("deagle")
        .args(args)
        .current_dir(cwd)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .await
        .map_err(|e| format!("Failed to run deagle: {}", e))?;

    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    Ok((stdout, stderr, output.status.success()))
}

fn ensure_deagle() -> Result<(), crate::PawanError> {
    if !deagle_available() {
        return Err(crate::PawanError::Tool(
            "deagle not found in PATH. Install: cargo install deagle".into(),
        ));
    }
    Ok(())
}

// ─── deagle search — graph symbol search ────────────────────────────────────

/// Graph-backed symbol search by name with optional kind filter.
pub struct DeagleSearchTool {
    workspace_root: PathBuf,
}

impl DeagleSearchTool {
    pub fn new(workspace_root: PathBuf) -> Self {
        Self { workspace_root }
    }
}

#[async_trait]
impl Tool for DeagleSearchTool {
    fn name(&self) -> &str {
        "deagle_search"
    }

    fn description(&self) -> &str {
        "Graph-backed symbol search via deagle. Finds functions, structs, traits, classes, \
         imports by name. Returns symbol kind, language, file path, and line number. \
         Much more structured than grep — use when you need to find a specific symbol \
         definition or check what kind of entity a name refers to. \
         Supports fuzzy matching and kind filtering (function, struct, trait, class, import)."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "query": { "type": "string", "description": "Symbol name to search for (empty string lists all)" },
                "kind": { "type": "string", "description": "Filter by kind: function, struct, trait, class, import, file" },
                "fuzzy": { "type": "boolean", "description": "Use fuzzy matching (default: false, exact substring)" },
                "limit": { "type": "integer", "description": "Max results to return (default: 50)" }
            },
            "required": ["query"]
        })
    }

    fn thulp_definition(&self) -> thulp_core::ToolDefinition {
        use thulp_core::{Parameter, ParameterType};
        thulp_core::ToolDefinition::builder(self.name())
            .description(self.description())
            .parameter(
                Parameter::builder("query")
                    .param_type(ParameterType::String)
                    .required(true)
                    .description("Symbol name to search for (empty string lists all)")
                    .build(),
            )
            .parameter(
                Parameter::builder("kind")
                    .param_type(ParameterType::String)
                    .required(false)
                    .description("Filter by kind: function, struct, trait, class, import, file")
                    .build(),
            )
            .parameter(
                Parameter::builder("fuzzy")
                    .param_type(ParameterType::Boolean)
                    .required(false)
                    .description("Use fuzzy matching (default: false)")
                    .build(),
            )
            .parameter(
                Parameter::builder("limit")
                    .param_type(ParameterType::Integer)
                    .required(false)
                    .description("Max results (default: 50)")
                    .build(),
            )
            .build()
    }

    async fn execute(&self, args: Value) -> crate::Result<Value> {
        ensure_deagle()?;
        let query = args["query"]
            .as_str()
            .ok_or_else(|| crate::PawanError::Tool("query required".into()))?;
        let limit = args["limit"].as_u64().unwrap_or(50);

        let limit_str = limit.to_string();
        let mut cmd_args: Vec<&str> = vec!["search", query, "--limit", &limit_str];
        if let Some(kind) = args["kind"].as_str() {
            cmd_args.extend_from_slice(&["--kind", kind]);
        }
        if args["fuzzy"].as_bool().unwrap_or(false) {
            cmd_args.push("--fuzzy");
        }

        let (stdout, stderr, success) = run_deagle(&cmd_args, &self.workspace_root)
            .await
            .map_err(crate::PawanError::Tool)?;

        let match_count = stdout.lines().filter(|l| !l.is_empty()).count().saturating_sub(2); // header lines

        Ok(json!({
            "results": stdout,
            "match_count": match_count,
            "success": success,
            "stderr": if stderr.is_empty() { None::<String> } else { Some(stderr) }
        }))
    }
}

// ─── deagle keyword — FTS5 BM25 ranked search ──────────────────────────────

/// Full-text keyword search with BM25 relevance ranking.
pub struct DeagleKeywordTool {
    workspace_root: PathBuf,
}

impl DeagleKeywordTool {
    pub fn new(workspace_root: PathBuf) -> Self {
        Self { workspace_root }
    }
}

#[async_trait]
impl Tool for DeagleKeywordTool {
    fn name(&self) -> &str {
        "deagle_keyword"
    }

    fn description(&self) -> &str {
        "Full-text keyword search via deagle with BM25 ranking (SQLite FTS5). \
         Returns entities ranked by relevance to the query. \
         Use when you need to find code related to a concept rather than a specific name — \
         e.g. 'authentication logic' or 'error handling patterns'. \
         More semantic than grep because it ranks by term frequency and inverse document frequency."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "query": { "type": "string", "description": "Keyword query (supports phrases in quotes)" },
                "limit": { "type": "integer", "description": "Max results (default: 20)" }
            },
            "required": ["query"]
        })
    }

    fn thulp_definition(&self) -> thulp_core::ToolDefinition {
        use thulp_core::{Parameter, ParameterType};
        thulp_core::ToolDefinition::builder(self.name())
            .description(self.description())
            .parameter(
                Parameter::builder("query")
                    .param_type(ParameterType::String)
                    .required(true)
                    .description("Keyword query")
                    .build(),
            )
            .parameter(
                Parameter::builder("limit")
                    .param_type(ParameterType::Integer)
                    .required(false)
                    .description("Max results (default: 20)")
                    .build(),
            )
            .build()
    }

    async fn execute(&self, args: Value) -> crate::Result<Value> {
        ensure_deagle()?;
        let query = args["query"]
            .as_str()
            .ok_or_else(|| crate::PawanError::Tool("query required".into()))?;
        let limit = args["limit"].as_u64().unwrap_or(20);

        let limit_str = limit.to_string();
        let cmd_args = vec!["keyword", query, "--limit", &limit_str];

        let (stdout, stderr, success) = run_deagle(&cmd_args, &self.workspace_root)
            .await
            .map_err(crate::PawanError::Tool)?;

        Ok(json!({
            "results": stdout,
            "success": success,
            "stderr": if stderr.is_empty() { None::<String> } else { Some(stderr) }
        }))
    }
}

// ─── deagle sg — structural AST pattern search ─────────────────────────────

/// Structural AST pattern search powered by ast-grep.
pub struct DeagleSgTool {
    workspace_root: PathBuf,
}

impl DeagleSgTool {
    pub fn new(workspace_root: PathBuf) -> Self {
        Self { workspace_root }
    }
}

#[async_trait]
impl Tool for DeagleSgTool {
    fn name(&self) -> &str {
        "deagle_sg"
    }

    fn description(&self) -> &str {
        "AST-based structural pattern search via deagle (ast-grep). \
         Find code by structure, not by text. Use patterns like \
         `impl $TYPE { $$$ }` to find all impl blocks, or \
         `pub fn $NAME($$$) { $$$ }` to find all public functions. \
         $VAR matches one node, $$$VAR matches multiple. \
         Much more precise than regex for refactoring and code audits."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "pattern": { "type": "string", "description": "AST pattern with $VAR metavariables" },
                "lang": { "type": "string", "description": "Language: rust, python, go, typescript, javascript, java, c, cpp" },
                "path": { "type": "string", "description": "Path to search (default: workspace root)" }
            },
            "required": ["pattern"]
        })
    }

    fn thulp_definition(&self) -> thulp_core::ToolDefinition {
        use thulp_core::{Parameter, ParameterType};
        thulp_core::ToolDefinition::builder(self.name())
            .description(self.description())
            .parameter(
                Parameter::builder("pattern")
                    .param_type(ParameterType::String)
                    .required(true)
                    .description("AST pattern with $VAR metavariables")
                    .build(),
            )
            .parameter(
                Parameter::builder("lang")
                    .param_type(ParameterType::String)
                    .required(false)
                    .description("Language filter")
                    .build(),
            )
            .parameter(
                Parameter::builder("path")
                    .param_type(ParameterType::String)
                    .required(false)
                    .description("Path to search")
                    .build(),
            )
            .build()
    }

    async fn execute(&self, args: Value) -> crate::Result<Value> {
        ensure_deagle()?;
        let pattern = args["pattern"]
            .as_str()
            .ok_or_else(|| crate::PawanError::Tool("pattern required".into()))?;
        let path = args["path"].as_str().unwrap_or(".");

        let mut cmd_args: Vec<&str> = vec!["sg", pattern];
        if let Some(lang) = args["lang"].as_str() {
            cmd_args.extend_from_slice(&["--lang", lang]);
        }
        cmd_args.push(path);

        let (stdout, stderr, success) = run_deagle(&cmd_args, &self.workspace_root)
            .await
            .map_err(crate::PawanError::Tool)?;

        let match_count = stdout.lines().filter(|l| !l.is_empty()).count();

        Ok(json!({
            "matches": stdout,
            "match_count": match_count,
            "success": success || match_count > 0,
            "stderr": if stderr.is_empty() { None::<String> } else { Some(stderr) }
        }))
    }
}

// ─── deagle stats — graph statistics ────────────────────────────────────────

/// Graph database statistics — node/edge counts, size.
pub struct DeagleStatsTool {
    workspace_root: PathBuf,
}

impl DeagleStatsTool {
    pub fn new(workspace_root: PathBuf) -> Self {
        Self { workspace_root }
    }
}

#[async_trait]
impl Tool for DeagleStatsTool {
    fn name(&self) -> &str {
        "deagle_stats"
    }

    fn description(&self) -> &str {
        "Show deagle graph database statistics: total nodes, edges, database size. \
         Use this to verify the codebase has been indexed, or to gauge codebase size \
         before deeper analysis. Run `deagle_map` first if stats show empty graph."
    }

    fn parameters_schema(&self) -> Value {
        json!({ "type": "object", "properties": {} })
    }

    fn thulp_definition(&self) -> thulp_core::ToolDefinition {
        thulp_core::ToolDefinition::builder(self.name())
            .description(self.description())
            .build()
    }

    async fn execute(&self, _args: Value) -> crate::Result<Value> {
        ensure_deagle()?;
        let (stdout, stderr, success) = run_deagle(&["stats"], &self.workspace_root)
            .await
            .map_err(crate::PawanError::Tool)?;

        Ok(json!({
            "stats": stdout,
            "success": success,
            "stderr": if stderr.is_empty() { None::<String> } else { Some(stderr) }
        }))
    }
}

// ─── deagle map — index/reindex codebase ────────────────────────────────────

/// Index a codebase into the deagle graph database.
pub struct DeagleMapTool {
    workspace_root: PathBuf,
}

impl DeagleMapTool {
    pub fn new(workspace_root: PathBuf) -> Self {
        Self { workspace_root }
    }
}

#[async_trait]
impl Tool for DeagleMapTool {
    fn name(&self) -> &str {
        "deagle_map"
    }

    fn description(&self) -> &str {
        "Index or re-index a codebase into the deagle graph database. \
         Uses tree-sitter parsers for 7 languages (Rust, Python, Go, TS/JS, Java, C, C++). \
         Incremental — only re-parses changed files (SHA-256 hash detection). \
         Run once to bootstrap, then again after significant code changes. \
         Required before `deagle_search`, `deagle_keyword`, `deagle_sg` work."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": { "type": "string", "description": "Path to index (default: workspace root)" }
            }
        })
    }

    fn thulp_definition(&self) -> thulp_core::ToolDefinition {
        use thulp_core::{Parameter, ParameterType};
        thulp_core::ToolDefinition::builder(self.name())
            .description(self.description())
            .parameter(
                Parameter::builder("path")
                    .param_type(ParameterType::String)
                    .required(false)
                    .description("Path to index (default: workspace root)")
                    .build(),
            )
            .build()
    }

    async fn execute(&self, args: Value) -> crate::Result<Value> {
        ensure_deagle()?;
        let path = args["path"].as_str().unwrap_or(".");
        let (stdout, stderr, success) = run_deagle(&["map", path], &self.workspace_root)
            .await
            .map_err(crate::PawanError::Tool)?;

        Ok(json!({
            "output": stdout,
            "success": success,
            "stderr": if stderr.is_empty() { None::<String> } else { Some(stderr) }
        }))
    }
}

// ─── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_deagle_search_tool_metadata() {
        let tool = DeagleSearchTool::new(PathBuf::from("."));
        assert_eq!(tool.name(), "deagle_search");
        assert!(tool.description().contains("symbol search"));
        let schema = tool.parameters_schema();
        assert!(schema["properties"]["query"].is_object());
    }

    #[test]
    fn test_deagle_keyword_tool_metadata() {
        let tool = DeagleKeywordTool::new(PathBuf::from("."));
        assert_eq!(tool.name(), "deagle_keyword");
        assert!(tool.description().contains("BM25"));
    }

    #[test]
    fn test_deagle_sg_tool_metadata() {
        let tool = DeagleSgTool::new(PathBuf::from("."));
        assert_eq!(tool.name(), "deagle_sg");
        assert!(tool.description().contains("AST"));
    }

    #[test]
    fn test_deagle_stats_tool_metadata() {
        let tool = DeagleStatsTool::new(PathBuf::from("."));
        assert_eq!(tool.name(), "deagle_stats");
    }

    #[test]
    fn test_deagle_map_tool_metadata() {
        let tool = DeagleMapTool::new(PathBuf::from("."));
        assert_eq!(tool.name(), "deagle_map");
        assert!(tool.description().contains("tree-sitter"));
    }

    #[test]
    fn test_thulp_definitions() {
        let tools: Vec<Box<dyn Tool>> = vec![
            Box::new(DeagleSearchTool::new(PathBuf::from("."))),
            Box::new(DeagleKeywordTool::new(PathBuf::from("."))),
            Box::new(DeagleSgTool::new(PathBuf::from("."))),
            Box::new(DeagleStatsTool::new(PathBuf::from("."))),
            Box::new(DeagleMapTool::new(PathBuf::from("."))),
        ];
        for tool in tools {
            let def = tool.thulp_definition();
            assert_eq!(def.name, tool.name());
            assert!(!def.description.is_empty());
        }
    }
}
