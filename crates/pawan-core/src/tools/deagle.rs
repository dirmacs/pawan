//! Deagle code intelligence tools — graph-backed symbol search.
//!
//! As of the Option B rewrite, pawan embeds `deagle-core` and
//! `deagle-parse` as library dependencies instead of shelling out to
//! the `deagle` binary. Users no longer need `cargo install deagle`;
//! all five tools work out of the box after `cargo install pawan`.
//!
//! Structure:
//! - `DeagleSearchTool` — `GraphDb::search_nodes` / `fuzzy_search_nodes` with kind filter
//! - `DeagleKeywordTool` — `GraphDb::keyword_search` (FTS5 BM25 ranked)
//! - `DeagleSgTool` — `deagle_parse::pattern::search_pattern` (ast-grep structural)
//! - `DeagleStatsTool` — `GraphDb::node_count` / `edge_count`
//! - `DeagleMapTool` — walks the workspace, parses with tree-sitter,
//!   inserts into the graph (mirrors `deagle map` in deagle-cli)
//!
//! The graph database lives at `<workspace_root>/.deagle/graph.db` by
//! default — same as the deagle CLI, so indexes built by the binary
//! remain usable when users upgrade.

use super::Tool;
use async_trait::async_trait;
use deagle_core::{Edge, EdgeKind, GraphDb, Language, Node, NodeKind};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

/// Default graph database path relative to the workspace root.
const GRAPH_DB_RELATIVE: &str = ".deagle/graph.db";

/// Resolve the graph database path inside a workspace.
fn graph_db_path(workspace_root: &Path) -> PathBuf {
    workspace_root.join(GRAPH_DB_RELATIVE)
}

/// Open the graph DB, creating the parent dir if needed. Returns a
/// friendly error when the DB doesn't exist yet.
fn open_graph(workspace_root: &Path) -> crate::Result<GraphDb> {
    let db_path = graph_db_path(workspace_root);
    if let Some(parent) = db_path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| crate::PawanError::Tool(format!("create .deagle dir: {}", e)))?;
    }
    GraphDb::open(&db_path).map_err(|e| {
        crate::PawanError::Tool(format!(
            "failed to open deagle graph at {}: {}. Run deagle_map first.",
            db_path.display(),
            e
        ))
    })
}

/// Format a `Vec<Node>` as a deagle-cli-style text table so the LLM
/// output matches what the subprocess version produced.
fn format_nodes_table(nodes: &[Node]) -> String {
    if nodes.is_empty() {
        return String::from("No results.");
    }
    let mut out = String::new();
    out.push_str(&format!(
        "{:<30} {:<12} {:<10} LOCATION\n",
        "NAME", "KIND", "LANG"
    ));
    out.push_str(&"-".repeat(80));
    out.push('\n');
    for node in nodes {
        out.push_str(&format!(
            "{:<30} {:<12} {:<10} {}:{}\n",
            node.name, node.kind, node.language, node.file_path, node.line_start,
        ));
    }
    out.push_str(&format!("\n{} result(s)\n", nodes.len()));
    out
}

/// Parse a user-provided kind string into a NodeKind filter. Returns
/// `None` if the string doesn't match any known kind (callers should
/// treat `None` as "no filter" rather than erroring — keeps parity with
/// the deagle CLI's permissive behavior).
fn parse_kind_filter(s: &str) -> Option<NodeKind> {
    match s.to_lowercase().as_str() {
        "file" => Some(NodeKind::File),
        "module" => Some(NodeKind::Module),
        "function" => Some(NodeKind::Function),
        "method" => Some(NodeKind::Method),
        "class" => Some(NodeKind::Class),
        "struct" => Some(NodeKind::Struct),
        "enum" => Some(NodeKind::Enum),
        "trait" => Some(NodeKind::Trait),
        "interface" => Some(NodeKind::Interface),
        "constant" => Some(NodeKind::Constant),
        "variable" => Some(NodeKind::Variable),
        "type_alias" | "typealias" => Some(NodeKind::TypeAlias),
        "import" => Some(NodeKind::Import),
        _ => None,
    }
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
        "Graph-backed symbol search via embedded deagle. Finds functions, structs, traits, \
         classes, imports by name. Returns symbol kind, language, file path, and line number. \
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
        let query = args["query"]
            .as_str()
            .ok_or_else(|| crate::PawanError::Tool("query required".into()))?;
        let limit = args["limit"].as_u64().unwrap_or(50) as usize;
        let fuzzy = args["fuzzy"].as_bool().unwrap_or(false);
        let kind_filter = args["kind"].as_str().and_then(parse_kind_filter);

        let db = open_graph(&self.workspace_root)?;
        let mut nodes = if fuzzy {
            db.fuzzy_search_nodes(query)
                .map_err(|e| crate::PawanError::Tool(format!("deagle search: {}", e)))?
        } else {
            db.search_nodes(query)
                .map_err(|e| crate::PawanError::Tool(format!("deagle search: {}", e)))?
        };

        if let Some(k) = kind_filter {
            nodes.retain(|n| n.kind == k);
        }
        if nodes.len() > limit {
            nodes.truncate(limit);
        }

        let match_count = nodes.len();
        let results = format_nodes_table(&nodes);

        Ok(json!({
            "results": results,
            "match_count": match_count,
            "success": true,
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
        "Full-text keyword search via embedded deagle with BM25 ranking (SQLite FTS5). \
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
        let query = args["query"]
            .as_str()
            .ok_or_else(|| crate::PawanError::Tool("query required".into()))?;
        let limit = args["limit"].as_u64().unwrap_or(20) as usize;

        let db = open_graph(&self.workspace_root)?;
        let mut nodes = db
            .keyword_search(query)
            .map_err(|e| crate::PawanError::Tool(format!("deagle keyword: {}", e)))?;
        if nodes.len() > limit {
            nodes.truncate(limit);
        }

        let results = format_nodes_table(&nodes);

        Ok(json!({
            "results": results,
            "success": true,
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
        "AST-based structural pattern search via embedded deagle (ast-grep). \
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
                "lang": { "type": "string", "description": "Language: rust, python, go, typescript, javascript, java, c, cpp, ruby" },
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
        let pattern = args["pattern"]
            .as_str()
            .ok_or_else(|| crate::PawanError::Tool("pattern required".into()))?;
        let rel_dir = args["path"].as_str().unwrap_or(".");
        let lang_filter = args["lang"]
            .as_str()
            .map(|s| parse_language(s))
            .filter(|l| *l != Language::Unknown);

        let search_root = if rel_dir == "." {
            self.workspace_root.clone()
        } else {
            self.workspace_root.join(rel_dir)
        };

        // Walk the tree, parse each source file, run the pattern.
        // Mirrors cmd_grep in deagle-cli.
        let walker = ignore::WalkBuilder::new(&search_root)
            .hidden(true)
            .git_ignore(true)
            .git_exclude(true)
            .build();

        let mut output = String::new();
        let mut total_matches = 0usize;

        for entry in walker.flatten() {
            let path = entry.path();
            if !path.is_file() {
                continue;
            }
            let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
            let file_lang = Language::from_extension(ext);
            if file_lang == Language::Unknown {
                continue;
            }
            if let Some(l) = lang_filter {
                if file_lang != l {
                    continue;
                }
            }

            let content = match std::fs::read_to_string(path) {
                Ok(c) if !c.is_empty() => c,
                _ => continue,
            };
            let rel_path = path.strip_prefix(&self.workspace_root).unwrap_or(path);

            if let Ok(matches) =
                deagle_parse::pattern::search_pattern(rel_path, &content, pattern, file_lang)
            {
                for m in &matches {
                    let first_line = m.text.lines().next().unwrap_or("");
                    output.push_str(&format!(
                        "{}:{}: {}\n",
                        m.file_path, m.line_start, first_line
                    ));
                    total_matches += 1;
                }
            }
        }

        if total_matches == 0 {
            output.push_str("No matches found.\n");
        } else {
            output.push_str(&format!("\n{} match(es)\n", total_matches));
        }

        Ok(json!({
            "matches": output,
            "match_count": total_matches,
            "success": true,
        }))
    }
}

/// Parse a user-provided language string into a `deagle_core::Language`.
/// Returns `Language::Unknown` for anything it can't resolve.
/// deagle-core 0.1.5 supports 9 languages (added Ruby in this release).
fn parse_language(s: &str) -> Language {
    match s.to_lowercase().as_str() {
        "rust" | "rs" => Language::Rust,
        "python" | "py" => Language::Python,
        "go" => Language::Go,
        "typescript" | "ts" => Language::TypeScript,
        "javascript" | "js" => Language::JavaScript,
        "java" => Language::Java,
        "cpp" | "c++" => Language::Cpp,
        "c" => Language::C,
        "ruby" | "rb" => Language::Ruby,
        _ => Language::Unknown,
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
        "Show embedded deagle graph database statistics: total nodes, edges, database path. \
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
        let db_path = graph_db_path(&self.workspace_root);
        // If the DB doesn't exist yet, return an empty-but-successful
        // report rather than erroring — mirrors deagle stats behavior
        // on unindexed dirs.
        if !db_path.exists() {
            return Ok(json!({
                "stats": format!(
                    "Database: {}\nNodes:    0\nEdges:    0\n(not yet indexed — run deagle_map first)",
                    db_path.display()
                ),
                "success": true,
            }));
        }

        let db = open_graph(&self.workspace_root)?;
        let nodes = db
            .node_count()
            .map_err(|e| crate::PawanError::Tool(format!("deagle stats: {}", e)))?;
        let edges = db
            .edge_count()
            .map_err(|e| crate::PawanError::Tool(format!("deagle stats: {}", e)))?;

        Ok(json!({
            "stats": format!(
                "Database: {}\nNodes:    {}\nEdges:    {}",
                db_path.display(), nodes, edges
            ),
            "success": true,
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
        "Index or re-index a codebase into the embedded deagle graph database. \
         Uses tree-sitter parsers for 9 languages (Rust, Python, Go, TypeScript, JavaScript, Java, C, C++, Ruby). \
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
        let rel_dir = args["path"].as_str().unwrap_or(".");
        let index_root = if rel_dir == "." {
            self.workspace_root.clone()
        } else {
            self.workspace_root.join(rel_dir)
        };

        let workspace_root = self.workspace_root.clone();
        // Run the CPU-bound walk+parse+insert on a blocking thread so it
        // doesn't stall the tokio runtime.
        let result = tokio::task::spawn_blocking(move || map_directory(&workspace_root, &index_root))
            .await
            .map_err(|e| crate::PawanError::Tool(format!("deagle map join: {}", e)))??;

        Ok(json!({
            "output": result,
            "success": true,
        }))
    }
}

/// Perform the full incremental map: walk, parse, insert. Returns a
/// human-readable summary string. Called from a blocking tokio thread.
fn map_directory(workspace_root: &Path, dir: &Path) -> crate::Result<String> {
    use rayon::prelude::*;

    let db_path = graph_db_path(workspace_root);
    if let Some(parent) = db_path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| crate::PawanError::Tool(format!("create .deagle: {}", e)))?;
    }
    let db = GraphDb::open(&db_path)
        .map_err(|e| crate::PawanError::Tool(format!("open graph: {}", e)))?;

    // Collect files (ignore-aware)
    let files: Vec<_> = ignore::WalkBuilder::new(dir)
        .hidden(true)
        .git_ignore(true)
        .git_global(true)
        .git_exclude(true)
        .build()
        .flatten()
        .filter(|e| e.path().is_file())
        .filter(|e| {
            let ext = e.path().extension().and_then(|x| x.to_str()).unwrap_or("");
            Language::from_extension(ext) != Language::Unknown
        })
        .collect();

    // Check hashes sequentially (SQLite not thread-safe)
    let files_to_parse: Vec<_> = files
        .iter()
        .filter(|entry| {
            let path = entry.path();
            let rel_path = path.strip_prefix(dir).unwrap_or(path);
            let rel_str = rel_path.to_string_lossy();
            let content = match std::fs::read_to_string(path) {
                Ok(c) if !c.is_empty() => c,
                _ => return false,
            };
            db.needs_reindex(&rel_str, &content).unwrap_or(true)
        })
        .collect();

    // Parse in parallel
    let results: Vec<_> = files_to_parse
        .par_iter()
        .filter_map(|entry| {
            let path = entry.path();
            let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
            let lang = Language::from_extension(ext);
            let content = std::fs::read_to_string(path).ok()?;
            if content.is_empty() {
                return None;
            }
            let rel_path = path.strip_prefix(dir).unwrap_or(path);
            let rel_str = rel_path.to_string_lossy().to_string();
            deagle_parse::parse_file_with_edges(rel_path, &content, lang)
                .ok()
                .map(|r| (rel_str, content, r))
        })
        .collect();

    // Insert (single thread — SQLite constraint)
    let mut file_count = 0usize;
    let mut node_count = 0usize;
    let mut edge_count = 0usize;

    for (rel_path, content, result) in &results {
        if result.nodes.is_empty() {
            continue;
        }
        let _ = db.remove_file(rel_path);
        file_count += 1;
        node_count += result.nodes.len();

        let db_ids = match db.insert_batch(&result.nodes, &[]) {
            Ok(ids) => ids,
            Err(_) => continue,
        };
        let _ = db.store_file_hash(rel_path, content);

        let resolved_edges: Vec<(i64, i64, EdgeKind)> = result
            .edges
            .iter()
            .filter(|(from_idx, to_idx, _)| {
                *from_idx < db_ids.len()
                    && *to_idx < db_ids.len()
                    && db_ids[*from_idx] > 0
                    && db_ids[*to_idx] > 0
            })
            .map(|(from_idx, to_idx, kind)| (db_ids[*from_idx], db_ids[*to_idx], *kind))
            .collect();
        edge_count += resolved_edges.len();

        for (from_id, to_id, kind) in &resolved_edges {
            let _ = db.insert_edge(&Edge {
                from_id: *from_id,
                to_id: *to_id,
                kind: *kind,
                confidence: 1.0,
            });
        }
    }

    let total_files = files.len();
    let skipped = total_files.saturating_sub(file_count);
    let summary = if skipped > 0 {
        format!(
            "Indexed {} files ({} unchanged), {} entities, {} edges\nDatabase: {}",
            file_count,
            skipped,
            node_count,
            edge_count,
            db_path.display()
        )
    } else {
        format!(
            "Indexed {} files, {} entities, {} edges\nDatabase: {}",
            file_count,
            node_count,
            edge_count,
            db_path.display()
        )
    };
    Ok(summary)
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

    #[test]
    fn test_deagle_tool_names_are_unique() {
        let tools: Vec<Box<dyn Tool>> = vec![
            Box::new(DeagleSearchTool::new(PathBuf::from("."))),
            Box::new(DeagleKeywordTool::new(PathBuf::from("."))),
            Box::new(DeagleSgTool::new(PathBuf::from("."))),
            Box::new(DeagleStatsTool::new(PathBuf::from("."))),
            Box::new(DeagleMapTool::new(PathBuf::from("."))),
        ];
        let names: std::collections::HashSet<String> =
            tools.iter().map(|t| t.name().to_string()).collect();
        assert_eq!(names.len(), 5);
        for expected in &["deagle_search", "deagle_keyword", "deagle_sg", "deagle_stats", "deagle_map"] {
            assert!(names.contains(*expected), "missing {}", expected);
        }
    }

    #[test]
    fn test_deagle_search_schema_required_query() {
        let tool = DeagleSearchTool::new(PathBuf::from("."));
        let schema = tool.parameters_schema();
        let required = schema["required"].as_array().unwrap();
        assert!(required.iter().any(|v| v == "query"));
        let props = schema["properties"].as_object().unwrap();
        assert!(props.contains_key("query"));
        assert!(props.contains_key("kind"));
        assert!(props.contains_key("fuzzy"));
        assert!(props.contains_key("limit"));
    }

    #[test]
    fn test_deagle_sg_schema_required_pattern() {
        let tool = DeagleSgTool::new(PathBuf::from("."));
        let schema = tool.parameters_schema();
        let required = schema["required"].as_array().unwrap();
        assert!(required.iter().any(|v| v == "pattern"));
        let props = schema["properties"].as_object().unwrap();
        assert!(props.contains_key("pattern"));
        assert!(props.contains_key("lang"));
        assert!(props.contains_key("path"));
    }

    #[tokio::test]
    async fn test_deagle_search_missing_query_errors() {
        let tool = DeagleSearchTool::new(PathBuf::from("."));
        let result = tool.execute(serde_json::json!({})).await;
        assert!(result.is_err());
        let err = format!("{}", result.unwrap_err());
        assert!(err.contains("query"));
    }

    #[tokio::test]
    async fn test_deagle_keyword_missing_query_errors() {
        let tool = DeagleKeywordTool::new(PathBuf::from("."));
        let result = tool.execute(serde_json::json!({})).await;
        assert!(result.is_err());
        let err = format!("{}", result.unwrap_err());
        assert!(err.contains("query"));
    }

    #[tokio::test]
    async fn test_deagle_sg_missing_pattern_errors() {
        let tool = DeagleSgTool::new(PathBuf::from("."));
        let result = tool.execute(serde_json::json!({})).await;
        assert!(result.is_err());
        let err = format!("{}", result.unwrap_err());
        assert!(err.contains("pattern"));
    }

    #[tokio::test]
    async fn test_deagle_search_query_wrong_type_errors() {
        let tool = DeagleSearchTool::new(PathBuf::from("."));
        let result = tool.execute(serde_json::json!({"query": 42})).await;
        assert!(result.is_err());
    }

    #[test]
    fn test_deagle_keyword_schema_required_query() {
        let tool = DeagleKeywordTool::new(PathBuf::from("."));
        let schema = tool.parameters_schema();
        let required = schema["required"].as_array().unwrap();
        assert!(required.iter().any(|v| v == "query"));
    }

    #[test]
    fn test_deagle_stats_schema_has_no_properties() {
        let tool = DeagleStatsTool::new(PathBuf::from("."));
        let schema = tool.parameters_schema();
        let props = schema["properties"].as_object().unwrap();
        assert!(props.is_empty());
    }

    #[test]
    fn test_deagle_map_schema_has_no_required() {
        let tool = DeagleMapTool::new(PathBuf::from("."));
        let schema = tool.parameters_schema();
        let has_required = schema.get("required").is_some_and(|r| {
            r.as_array().map(|a| !a.is_empty()).unwrap_or(false)
        });
        assert!(!has_required);
    }

    #[test]
    fn test_parse_kind_filter_known_kinds() {
        // These must cover every variant of NodeKind so user queries
        // like --kind function actually filter. Case-insensitive.
        assert_eq!(parse_kind_filter("function"), Some(NodeKind::Function));
        assert_eq!(parse_kind_filter("FUNCTION"), Some(NodeKind::Function));
        assert_eq!(parse_kind_filter("struct"), Some(NodeKind::Struct));
        assert_eq!(parse_kind_filter("trait"), Some(NodeKind::Trait));
        assert_eq!(parse_kind_filter("class"), Some(NodeKind::Class));
        assert_eq!(parse_kind_filter("import"), Some(NodeKind::Import));
        assert_eq!(parse_kind_filter("file"), Some(NodeKind::File));
        assert_eq!(parse_kind_filter("type_alias"), Some(NodeKind::TypeAlias));
        assert_eq!(parse_kind_filter("typealias"), Some(NodeKind::TypeAlias));
    }

    #[test]
    fn test_parse_kind_filter_unknown_returns_none() {
        // Unknown kinds must return None (= no filter), not error —
        // matches the permissive behavior of deagle-cli.
        assert_eq!(parse_kind_filter("garbage"), None);
        assert_eq!(parse_kind_filter(""), None);
    }

    #[test]
    fn test_parse_language_covers_all_supported() {
        // deagle-core 0.1.5 supports 9 languages + Unknown fallback
        assert_eq!(parse_language("rust"), Language::Rust);
        assert_eq!(parse_language("rs"), Language::Rust);
        assert_eq!(parse_language("python"), Language::Python);
        assert_eq!(parse_language("py"), Language::Python);
        assert_eq!(parse_language("go"), Language::Go);
        assert_eq!(parse_language("typescript"), Language::TypeScript);
        assert_eq!(parse_language("ts"), Language::TypeScript);
        assert_eq!(parse_language("javascript"), Language::JavaScript);
        assert_eq!(parse_language("java"), Language::Java);
        assert_eq!(parse_language("cpp"), Language::Cpp);
        assert_eq!(parse_language("c++"), Language::Cpp);
        assert_eq!(parse_language("c"), Language::C);
        assert_eq!(parse_language("ruby"), Language::Ruby);
        assert_eq!(parse_language("rb"), Language::Ruby);
        assert_eq!(parse_language("unknown-lang"), Language::Unknown);
    }

    #[test]
    fn test_format_nodes_table_empty() {
        assert_eq!(format_nodes_table(&[]), "No results.");
    }

    #[test]
    fn test_format_nodes_table_includes_headers_and_counts() {
        let nodes = vec![Node {
            id: 1,
            name: "my_fn".into(),
            kind: NodeKind::Function,
            language: Language::Rust,
            file_path: "src/lib.rs".into(),
            line_start: 42,
            line_end: 50,
            content: None,
        }];
        let formatted = format_nodes_table(&nodes);
        assert!(formatted.contains("NAME"));
        assert!(formatted.contains("KIND"));
        assert!(formatted.contains("LOCATION"));
        assert!(formatted.contains("my_fn"));
        assert!(formatted.contains("function"));
        assert!(formatted.contains("src/lib.rs:42"));
        assert!(formatted.contains("1 result(s)"));
    }

    #[test]
    fn test_graph_db_path_is_under_workspace() {
        let root = PathBuf::from("/tmp/test-workspace");
        let path = graph_db_path(&root);
        assert_eq!(path, PathBuf::from("/tmp/test-workspace/.deagle/graph.db"));
    }

    #[tokio::test]
    async fn test_deagle_stats_on_empty_workspace_is_non_fatal() {
        // Unindexed workspace → stats must NOT error. It should return a
        // "0 nodes, 0 edges" placeholder with a hint.
        let tmp = tempfile::TempDir::new().unwrap();
        let tool = DeagleStatsTool::new(tmp.path().to_path_buf());
        let result = tool.execute(serde_json::json!({})).await.unwrap();
        let stats = result["stats"].as_str().unwrap();
        assert!(stats.contains("Nodes:"));
        assert!(stats.contains("0"));
    }
}
