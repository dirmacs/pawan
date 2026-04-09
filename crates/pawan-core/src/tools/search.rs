//! Search tools (glob and grep)

use super::Tool;
use async_trait::async_trait;
use serde_json::{json, Value};
use std::path::PathBuf;

/// Tool for finding files by glob pattern
pub struct GlobSearchTool {
    workspace_root: PathBuf,
}

impl GlobSearchTool {
    pub fn new(workspace_root: PathBuf) -> Self {
        Self { workspace_root }
    }
}

#[async_trait]
impl Tool for GlobSearchTool {
    fn name(&self) -> &str {
        "glob_search"
    }

    fn description(&self) -> &str {
        "Find files matching a glob pattern. Respects .gitignore. \
         Examples: '**/*.rs', 'src/**/*.toml', 'Cargo.*'"
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "pattern": {
                    "type": "string",
                    "description": "Glob pattern to match files"
                },
                "path": {
                    "type": "string",
                    "description": "Directory to search in (optional, defaults to workspace root)"
                },
                "max_results": {
                    "type": "integer",
                    "description": "Maximum number of results (default: 100)"
                }
            },
            "required": ["pattern"]
        })
    }

    async fn execute(&self, args: Value) -> crate::Result<Value> {
        let pattern = args["pattern"]
            .as_str()
            .ok_or_else(|| crate::PawanError::Tool("pattern is required".into()))?;

        let base_path = args["path"]
            .as_str()
            .map(|p| self.workspace_root.join(p))
            .unwrap_or_else(|| self.workspace_root.clone());

        let max_results = args["max_results"].as_u64().unwrap_or(100) as usize;

        // Use ignore crate to respect .gitignore
        let mut builder = ignore::WalkBuilder::new(&base_path);
        builder.hidden(false); // Include hidden files if explicitly matched

        let mut matches = Vec::new();
        let glob_matcher = glob::Pattern::new(pattern)
            .map_err(|e| crate::PawanError::Tool(format!("Invalid glob pattern: {}", e)))?;

        for result in builder.build() {
            if matches.len() >= max_results {
                break;
            }

            if let Ok(entry) = result {
                let path = entry.path();
                if path.is_file() {
                    let relative = path.strip_prefix(&self.workspace_root).unwrap_or(path);
                    let relative_str = relative.to_string_lossy();

                    if glob_matcher.matches(&relative_str) {
                        let metadata = path.metadata().ok();
                        let size = metadata.as_ref().map(|m| m.len()).unwrap_or(0);
                        let modified = metadata.and_then(|m| m.modified().ok()).map(|t| {
                            t.duration_since(std::time::UNIX_EPOCH)
                                .map(|d| d.as_secs())
                                .unwrap_or(0)
                        });
                        matches.push(json!({
                            "path": relative_str,
                            "size": size,
                            "modified": modified
                        }));
                    }
                }
            }
        }

        // Sort by modification time (newest first)
        matches.sort_by(|a, b| {
            let a_mod = a["modified"].as_u64().unwrap_or(0);
            let b_mod = b["modified"].as_u64().unwrap_or(0);
            b_mod.cmp(&a_mod)
        });

        Ok(json!({
            "pattern": pattern,
            "matches": matches,
            "count": matches.len(),
            "truncated": matches.len() >= max_results
        }))
    }
}

/// Tool for searching file contents
pub struct GrepSearchTool {
    workspace_root: PathBuf,
}

impl GrepSearchTool {
    pub fn new(workspace_root: PathBuf) -> Self {
        Self { workspace_root }
    }
}

#[async_trait]
impl Tool for GrepSearchTool {
    fn name(&self) -> &str {
        "grep_search"
    }

    fn description(&self) -> &str {
        "Search file contents for a pattern. Supports regex. \
         Returns file paths and line numbers with matches."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "pattern": {
                    "type": "string",
                    "description": "Pattern to search for (supports regex)"
                },
                "path": {
                    "type": "string",
                    "description": "Directory to search in (optional, defaults to workspace root)"
                },
                "include": {
                    "type": "string",
                    "description": "File pattern to include (e.g., '*.rs', '*.{ts,tsx}')"
                },
                "max_results": {
                    "type": "integer",
                    "description": "Maximum number of matching files (default: 50)"
                },
                "context_lines": {
                    "type": "integer",
                    "description": "Lines of context around matches (default: 0)"
                }
            },
            "required": ["pattern"]
        })
    }

    async fn execute(&self, args: Value) -> crate::Result<Value> {
        let pattern = args["pattern"]
            .as_str()
            .ok_or_else(|| crate::PawanError::Tool("pattern is required".into()))?;

        let base_path = args["path"]
            .as_str()
            .map(|p| self.workspace_root.join(p))
            .unwrap_or_else(|| self.workspace_root.clone());

        let include = args["include"].as_str();
        let max_results = args["max_results"].as_u64().unwrap_or(50) as usize;
        let context_lines = args["context_lines"].as_u64().unwrap_or(0) as usize;

        // Build regex
        let regex = regex::Regex::new(pattern)
            .map_err(|e| crate::PawanError::Tool(format!("Invalid regex: {}", e)))?;

        // Build glob matcher for include filter
        let include_matcher = include
            .map(glob::Pattern::new)
            .transpose()
            .map_err(|e| crate::PawanError::Tool(format!("Invalid include pattern: {}", e)))?;

        let mut file_matches = Vec::new();

        // Walk directory
        let mut builder = ignore::WalkBuilder::new(&base_path);
        builder.hidden(false);

        for result in builder.build() {
            if file_matches.len() >= max_results {
                break;
            }

            if let Ok(entry) = result {
                let path = entry.path();
                if !path.is_file() {
                    continue;
                }

                let relative = path.strip_prefix(&self.workspace_root).unwrap_or(path);
                let relative_str = relative.to_string_lossy();

                // Check include filter
                if let Some(ref matcher) = include_matcher {
                    // Match against filename only
                    let filename = path
                        .file_name()
                        .map(|n| n.to_string_lossy())
                        .unwrap_or_default();
                    if !matcher.matches(&filename) && !matcher.matches(&relative_str) {
                        continue;
                    }
                }

                // Read and search file
                if let Ok(content) = std::fs::read_to_string(path) {
                    let mut line_matches = Vec::new();
                    let lines: Vec<&str> = content.lines().collect();

                    for (line_num, line) in lines.iter().enumerate() {
                        if regex.is_match(line) {
                            let mut match_info = json!({
                                "line": line_num + 1,
                                "content": line.chars().take(200).collect::<String>()
                            });

                            // Add context if requested
                            if context_lines > 0 {
                                let start = line_num.saturating_sub(context_lines);
                                let end = (line_num + context_lines + 1).min(lines.len());
                                let context: Vec<String> = lines[start..end]
                                    .iter()
                                    .enumerate()
                                    .map(|(i, l)| format!("{}: {}", start + i + 1, l))
                                    .collect();
                                match_info["context"] = json!(context);
                            }

                            line_matches.push(match_info);
                        }
                    }

                    if !line_matches.is_empty() {
                        file_matches.push(json!({
                            "path": relative_str,
                            "matches": line_matches,
                            "match_count": line_matches.len()
                        }));
                    }
                }
            }
        }

        // Sort by match count (most matches first)
        file_matches.sort_by(|a, b| {
            let a_count = a["match_count"].as_u64().unwrap_or(0);
            let b_count = b["match_count"].as_u64().unwrap_or(0);
            b_count.cmp(&a_count)
        });

        let total_matches: u64 = file_matches
            .iter()
            .map(|f| f["match_count"].as_u64().unwrap_or(0))
            .sum();

        Ok(json!({
            "pattern": pattern,
            "files": file_matches,
            "file_count": file_matches.len(),
            "total_matches": total_matches,
            "truncated": file_matches.len() >= max_results
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[tokio::test]
    async fn test_glob_search() {
        let temp_dir = TempDir::new().unwrap();
        std::fs::write(temp_dir.path().join("file1.rs"), "rust code").unwrap();
        std::fs::write(temp_dir.path().join("file2.rs"), "more rust").unwrap();
        std::fs::write(temp_dir.path().join("file3.txt"), "text file").unwrap();

        let tool = GlobSearchTool::new(temp_dir.path().to_path_buf());
        let result = tool.execute(json!({"pattern": "*.rs"})).await.unwrap();

        assert_eq!(result["count"], 2);
    }

    #[tokio::test]
    async fn test_grep_search() {
        let temp_dir = TempDir::new().unwrap();
        std::fs::write(
            temp_dir.path().join("test.rs"),
            "fn main() {\n    println!(\"hello\");\n}",
        )
        .unwrap();

        let tool = GrepSearchTool::new(temp_dir.path().to_path_buf());
        let result = tool
            .execute(json!({
                "pattern": "println",
                "include": "*.rs"
            }))
            .await
            .unwrap();

        assert_eq!(result["file_count"], 1);
        assert_eq!(result["total_matches"], 1);
    }

    // --- GlobSearchTool expanded tests ---

    #[tokio::test]
    async fn test_glob_no_matches() {
        let tmp = TempDir::new().unwrap();
        std::fs::write(tmp.path().join("file.txt"), "text").unwrap();
        let tool = GlobSearchTool::new(tmp.path().into());
        let result = tool.execute(json!({"pattern": "*.rs"})).await.unwrap();
        assert_eq!(result["count"], 0);
        assert_eq!(result["truncated"], false);
    }

    #[tokio::test]
    async fn test_glob_invalid_pattern() {
        let tmp = TempDir::new().unwrap();
        let tool = GlobSearchTool::new(tmp.path().into());
        let result = tool.execute(json!({"pattern": "[invalid"})).await;
        assert!(result.is_err(), "Invalid glob should error");
    }

    #[tokio::test]
    async fn test_glob_missing_pattern() {
        let tmp = TempDir::new().unwrap();
        let tool = GlobSearchTool::new(tmp.path().into());
        let result = tool.execute(json!({})).await;
        assert!(result.is_err(), "Missing pattern should error");
    }

    #[tokio::test]
    async fn test_glob_max_results() {
        let tmp = TempDir::new().unwrap();
        for i in 0..10 {
            std::fs::write(tmp.path().join(format!("f{}.rs", i)), "code").unwrap();
        }
        let tool = GlobSearchTool::new(tmp.path().into());
        let result = tool.execute(json!({"pattern": "*.rs", "max_results": 3})).await.unwrap();
        assert_eq!(result["count"], 3);
        assert_eq!(result["truncated"], true);
    }

    #[tokio::test]
    async fn test_glob_subdirectory() {
        let tmp = TempDir::new().unwrap();
        std::fs::create_dir(tmp.path().join("sub")).unwrap();
        std::fs::write(tmp.path().join("sub/a.rs"), "code").unwrap();
        std::fs::write(tmp.path().join("b.rs"), "code").unwrap();
        let tool = GlobSearchTool::new(tmp.path().into());
        // Search only in sub/
        let result = tool.execute(json!({"pattern": "*.rs", "path": "sub"})).await.unwrap();
        assert_eq!(result["count"], 1);
    }

    #[tokio::test]
    async fn test_glob_result_has_metadata() {
        let tmp = TempDir::new().unwrap();
        std::fs::write(tmp.path().join("f.rs"), "hello world").unwrap();
        let tool = GlobSearchTool::new(tmp.path().into());
        let result = tool.execute(json!({"pattern": "*.rs"})).await.unwrap();
        let first = &result["matches"][0];
        assert!(first["path"].as_str().is_some());
        assert!(first["size"].as_u64().unwrap() > 0);
        assert!(first["modified"].as_u64().is_some());
    }

    // --- GrepSearchTool expanded tests ---

    #[tokio::test]
    async fn test_grep_no_matches() {
        let tmp = TempDir::new().unwrap();
        std::fs::write(tmp.path().join("f.rs"), "fn main() {}").unwrap();
        let tool = GrepSearchTool::new(tmp.path().into());
        let result = tool.execute(json!({"pattern": "nonexistent_string_xyz"})).await.unwrap();
        assert_eq!(result["file_count"], 0);
        assert_eq!(result["total_matches"], 0);
    }

    #[tokio::test]
    async fn test_grep_regex() {
        let tmp = TempDir::new().unwrap();
        std::fs::write(tmp.path().join("f.rs"), "fn foo() {}\nfn bar() {}\nfn baz() {}").unwrap();
        let tool = GrepSearchTool::new(tmp.path().into());
        let result = tool.execute(json!({"pattern": "fn \\w+\\(\\)"})).await.unwrap();
        assert_eq!(result["total_matches"], 3);
    }

    #[tokio::test]
    async fn test_grep_invalid_regex() {
        let tmp = TempDir::new().unwrap();
        let tool = GrepSearchTool::new(tmp.path().into());
        let result = tool.execute(json!({"pattern": "[invalid"})).await;
        assert!(result.is_err(), "Invalid regex should error");
    }

    #[tokio::test]
    async fn test_grep_missing_pattern() {
        let tmp = TempDir::new().unwrap();
        let tool = GrepSearchTool::new(tmp.path().into());
        let result = tool.execute(json!({})).await;
        assert!(result.is_err(), "Missing pattern should error");
    }

    #[tokio::test]
    async fn test_grep_include_filter() {
        let tmp = TempDir::new().unwrap();
        std::fs::write(tmp.path().join("a.rs"), "hello").unwrap();
        std::fs::write(tmp.path().join("b.txt"), "hello").unwrap();
        let tool = GrepSearchTool::new(tmp.path().into());
        let result = tool.execute(json!({"pattern": "hello", "include": "*.rs"})).await.unwrap();
        assert_eq!(result["file_count"], 1);
        let path = result["files"][0]["path"].as_str().unwrap();
        assert!(path.ends_with(".rs"));
    }

    #[tokio::test]
    async fn test_grep_context_lines() {
        let tmp = TempDir::new().unwrap();
        std::fs::write(tmp.path().join("f.rs"), "line1\nline2\nTARGET\nline4\nline5").unwrap();
        let tool = GrepSearchTool::new(tmp.path().into());
        let result = tool.execute(json!({"pattern": "TARGET", "context_lines": 1})).await.unwrap();
        let matches = result["files"][0]["matches"].as_array().unwrap();
        assert!(matches[0]["context"].is_array());
        let ctx = matches[0]["context"].as_array().unwrap();
        assert_eq!(ctx.len(), 3); // 1 before + match + 1 after
    }

    #[tokio::test]
    async fn test_grep_max_results() {
        let tmp = TempDir::new().unwrap();
        for i in 0..10 {
            std::fs::write(tmp.path().join(format!("f{}.rs", i)), "match_me").unwrap();
        }
        let tool = GrepSearchTool::new(tmp.path().into());
        let result = tool.execute(json!({"pattern": "match_me", "max_results": 3})).await.unwrap();
        assert_eq!(result["file_count"], 3);
        assert_eq!(result["truncated"], true);
    }

    #[tokio::test]
    async fn test_grep_multiple_matches_in_file() {
        let tmp = TempDir::new().unwrap();
        std::fs::write(tmp.path().join("f.rs"), "foo\nbar\nfoo\nbaz\nfoo").unwrap();
        let tool = GrepSearchTool::new(tmp.path().into());
        let result = tool.execute(json!({"pattern": "foo"})).await.unwrap();
        assert_eq!(result["files"][0]["match_count"], 3);
        assert_eq!(result["total_matches"], 3);
    }

    #[tokio::test]
    async fn test_grep_line_truncation() {
        let tmp = TempDir::new().unwrap();
        let long_line = "x".repeat(500);
        std::fs::write(tmp.path().join("f.rs"), &long_line).unwrap();
        let tool = GrepSearchTool::new(tmp.path().into());
        let result = tool.execute(json!({"pattern": "x+"})).await.unwrap();
        let content = result["files"][0]["matches"][0]["content"].as_str().unwrap();
        assert_eq!(content.len(), 200, "Line content should be truncated to 200 chars");
    }

    #[tokio::test]
    async fn test_grep_sorted_by_match_count() {
        let tmp = TempDir::new().unwrap();
        std::fs::write(tmp.path().join("few.rs"), "x").unwrap();
        std::fs::write(tmp.path().join("many.rs"), "x\nx\nx\nx\nx").unwrap();
        let tool = GrepSearchTool::new(tmp.path().into());
        let result = tool.execute(json!({"pattern": "x"})).await.unwrap();
        let files = result["files"].as_array().unwrap();
        assert!(files.len() == 2);
        // First file should have more matches
        let first_count = files[0]["match_count"].as_u64().unwrap();
        let second_count = files[1]["match_count"].as_u64().unwrap();
        assert!(first_count >= second_count, "Results should be sorted by match count desc");
    }
}
