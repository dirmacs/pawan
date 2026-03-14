//! Edit tool for precise string replacement

use super::Tool;
use async_trait::async_trait;
use serde_json::{json, Value};
use std::path::PathBuf;

/// Tool for editing files with precise string replacement
pub struct EditFileTool {
    workspace_root: PathBuf,
}

impl EditFileTool {
    pub fn new(workspace_root: PathBuf) -> Self {
        Self { workspace_root }
    }

    fn resolve_path(&self, path: &str) -> PathBuf {
        let path = PathBuf::from(path);
        if path.is_absolute() {
            path
        } else {
            self.workspace_root.join(path)
        }
    }
}

#[async_trait]
impl Tool for EditFileTool {
    fn name(&self) -> &str {
        "edit_file"
    }

    fn description(&self) -> &str {
        "Edit a file by replacing an exact string with new text. \
         PREFER edit_file_lines for most edits — it is more reliable because it \
         uses line numbers instead of exact string matching. \
         Use edit_file only when the target string is short, unique, and trivially \
         identifiable (e.g. a one-line change in a small file). \
         Fails if old_string is not found or appears more than once (use replace_all for the latter)."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Path to the file to edit"
                },
                "old_string": {
                    "type": "string",
                    "description": "The exact string to find and replace"
                },
                "new_string": {
                    "type": "string",
                    "description": "The string to replace it with"
                },
                "replace_all": {
                    "type": "boolean",
                    "description": "Replace all occurrences (default: false)"
                }
            },
            "required": ["path", "old_string", "new_string"]
        })
    }

    async fn execute(&self, args: Value) -> crate::Result<Value> {
        let path = args["path"]
            .as_str()
            .ok_or_else(|| crate::PawanError::Tool("path is required".into()))?;

        let old_string = args["old_string"]
            .as_str()
            .ok_or_else(|| crate::PawanError::Tool("old_string is required".into()))?;

        let new_string = args["new_string"]
            .as_str()
            .ok_or_else(|| crate::PawanError::Tool("new_string is required".into()))?;

        let replace_all = args["replace_all"].as_bool().unwrap_or(false);

        let full_path = self.resolve_path(path);

        if !full_path.exists() {
            return Err(crate::PawanError::NotFound(format!(
                "File not found: {}",
                full_path.display()
            )));
        }

        // Read current content
        let content = tokio::fs::read_to_string(&full_path)
            .await
            .map_err(crate::PawanError::Io)?;

        // Count occurrences
        let occurrence_count = content.matches(old_string).count();

        if occurrence_count == 0 {
            return Err(crate::PawanError::Tool(
                "old_string not found in file. Make sure the string matches exactly including whitespace.".to_string()
            ));
        }

        if occurrence_count > 1 && !replace_all {
            return Err(crate::PawanError::Tool(format!(
                "old_string found {} times. Use replace_all: true to replace all, \
                 or provide more context to make the match unique.",
                occurrence_count
            )));
        }

        // Perform replacement
        let new_content = if replace_all {
            content.replace(old_string, new_string)
        } else {
            content.replacen(old_string, new_string, 1)
        };

        // Write back
        tokio::fs::write(&full_path, &new_content)
            .await
            .map_err(crate::PawanError::Io)?;

        // Generate a diff preview
        let diff = generate_diff(&content, &new_content, path);

        Ok(json!({
            "success": true,
            "path": full_path.display().to_string(),
            "replacements": if replace_all { occurrence_count } else { 1 },
            "diff": diff
        }))
    }
}

/// Tool for editing files by replacing a range of lines
pub struct EditFileLinesTool {
    workspace_root: PathBuf,
}

impl EditFileLinesTool {
    pub fn new(workspace_root: PathBuf) -> Self {
        Self { workspace_root }
    }

    fn resolve_path(&self, path: &str) -> PathBuf {
        let path = PathBuf::from(path);
        if path.is_absolute() {
            path
        } else {
            self.workspace_root.join(path)
        }
    }
}

#[async_trait]
impl Tool for EditFileLinesTool {
    fn name(&self) -> &str {
        "edit_file_lines"
    }

    fn description(&self) -> &str {
        "PREFERRED edit tool. Replace a range of lines in a file with new content. \
         Workflow: (1) read_file to see line numbers, (2) call this tool with \
         start_line and end_line (1-based, inclusive). \
         More reliable than edit_file because it never fails due to exact string \
         matching — use it for any edit in a file longer than ~20 lines. \
         Set new_content to \"\" to delete lines."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Path to the file to edit"
                },
                "start_line": {
                    "type": "integer",
                    "description": "First line to replace (1-based, inclusive)"
                },
                "end_line": {
                    "type": "integer",
                    "description": "Last line to replace (1-based, inclusive)"
                },
                "new_content": {
                    "type": "string",
                    "description": "Replacement text for the specified lines. Empty string to delete lines."
                }
            },
            "required": ["path", "start_line", "end_line", "new_content"]
        })
    }

    async fn execute(&self, args: Value) -> crate::Result<Value> {
        let path = args["path"]
            .as_str()
            .ok_or_else(|| crate::PawanError::Tool("path is required".into()))?;

        let start_line = args["start_line"]
            .as_u64()
            .ok_or_else(|| crate::PawanError::Tool("start_line is required".into()))? as usize;

        let end_line = args["end_line"]
            .as_u64()
            .ok_or_else(|| crate::PawanError::Tool("end_line is required".into()))? as usize;

        let new_content = args["new_content"]
            .as_str()
            .ok_or_else(|| crate::PawanError::Tool("new_content is required".into()))?;

        if start_line == 0 {
            return Err(crate::PawanError::Tool(
                "start_line must be >= 1 (lines are 1-based)".into(),
            ));
        }

        if end_line < start_line {
            return Err(crate::PawanError::Tool(format!(
                "end_line ({end_line}) must be >= start_line ({start_line})"
            )));
        }

        let full_path = self.resolve_path(path);

        if !full_path.exists() {
            return Err(crate::PawanError::NotFound(format!(
                "File not found: {}",
                full_path.display()
            )));
        }

        let content = tokio::fs::read_to_string(&full_path)
            .await
            .map_err(crate::PawanError::Io)?;

        let had_trailing_newline = content.ends_with('\n');
        let lines: Vec<&str> = content.lines().collect();
        let total_lines = lines.len();

        if start_line > total_lines {
            return Err(crate::PawanError::Tool(format!(
                "start_line ({start_line}) exceeds file length ({total_lines} lines)"
            )));
        }

        if end_line > total_lines {
            return Err(crate::PawanError::Tool(format!(
                "end_line ({end_line}) exceeds file length ({total_lines} lines)"
            )));
        }

        let new_lines: Vec<&str> = new_content.lines().collect();
        let lines_replaced = end_line - start_line + 1;

        let before = &lines[..start_line - 1];
        let after = &lines[end_line..];

        let mut result_lines: Vec<&str> =
            Vec::with_capacity(before.len() + new_lines.len() + after.len());
        result_lines.extend_from_slice(before);
        result_lines.extend_from_slice(&new_lines);
        result_lines.extend_from_slice(after);

        let mut new_content_str = result_lines.join("\n");
        if had_trailing_newline && !new_content_str.is_empty() {
            new_content_str.push('\n');
        }

        tokio::fs::write(&full_path, &new_content_str)
            .await
            .map_err(crate::PawanError::Io)?;

        let diff = generate_diff(&content, &new_content_str, path);

        Ok(json!({
            "success": true,
            "path": full_path.display().to_string(),
            "lines_replaced": lines_replaced,
            "new_line_count": new_lines.len(),
            "diff": diff
        }))
    }
}

/// Generate a simple diff between two strings
fn generate_diff(old: &str, new: &str, filename: &str) -> String {
    use similar::{ChangeTag, TextDiff};

    let diff = TextDiff::from_lines(old, new);
    let mut result = String::new();

    result.push_str(&format!("--- a/{}\n", filename));
    result.push_str(&format!("+++ b/{}\n", filename));

    for (idx, group) in diff.grouped_ops(3).iter().enumerate() {
        if idx > 0 {
            result.push_str("...\n");
        }

        for op in group {
            for change in diff.iter_changes(op) {
                let sign = match change.tag() {
                    ChangeTag::Delete => "-",
                    ChangeTag::Insert => "+",
                    ChangeTag::Equal => " ",
                };
                result.push_str(&format!("{}{}", sign, change));
            }
        }
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[tokio::test]
    async fn test_edit_file_single_replacement() {
        let temp_dir = TempDir::new().unwrap();
        let file_path = temp_dir.path().join("test.rs");
        std::fs::write(&file_path, "fn main() {\n    println!(\"Hello\");\n}").unwrap();

        let tool = EditFileTool::new(temp_dir.path().to_path_buf());
        let result = tool
            .execute(json!({
                "path": "test.rs",
                "old_string": "println!(\"Hello\")",
                "new_string": "println!(\"Hello, World!\")"
            }))
            .await
            .unwrap();

        assert!(result["success"].as_bool().unwrap());
        assert_eq!(result["replacements"], 1);

        let new_content = std::fs::read_to_string(&file_path).unwrap();
        assert!(new_content.contains("Hello, World!"));
    }

    #[tokio::test]
    async fn test_edit_file_not_found() {
        let temp_dir = TempDir::new().unwrap();
        let file_path = temp_dir.path().join("test.rs");
        std::fs::write(&file_path, "fn main() {}").unwrap();

        let tool = EditFileTool::new(temp_dir.path().to_path_buf());
        let result = tool
            .execute(json!({
                "path": "test.rs",
                "old_string": "nonexistent",
                "new_string": "replacement"
            }))
            .await;

        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_edit_file_multiple_without_replace_all() {
        let temp_dir = TempDir::new().unwrap();
        let file_path = temp_dir.path().join("test.rs");
        std::fs::write(&file_path, "let x = 1;\nlet x = 2;").unwrap();

        let tool = EditFileTool::new(temp_dir.path().to_path_buf());
        let result = tool
            .execute(json!({
                "path": "test.rs",
                "old_string": "let x",
                "new_string": "let y"
            }))
            .await;

        // Should fail because there are multiple occurrences
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_edit_file_replace_all() {
        let temp_dir = TempDir::new().unwrap();
        let file_path = temp_dir.path().join("test.rs");
        std::fs::write(&file_path, "let x = 1;\nlet x = 2;").unwrap();

        let tool = EditFileTool::new(temp_dir.path().to_path_buf());
        let result = tool
            .execute(json!({
                "path": "test.rs",
                "old_string": "let x",
                "new_string": "let y",
                "replace_all": true
            }))
            .await
            .unwrap();

        assert!(result["success"].as_bool().unwrap());
        assert_eq!(result["replacements"], 2);

        let new_content = std::fs::read_to_string(&file_path).unwrap();
        assert!(!new_content.contains("let x"));
        assert!(new_content.contains("let y"));
    }

    // --- EditFileLinesTool tests ---

    #[tokio::test]
    async fn test_edit_file_lines_middle() {
        let temp_dir = TempDir::new().unwrap();
        let file_path = temp_dir.path().join("test.rs");
        std::fs::write(&file_path, "line1\nline2\nline3\n").unwrap();

        let tool = EditFileLinesTool::new(temp_dir.path().to_path_buf());
        let result = tool
            .execute(json!({
                "path": "test.rs",
                "start_line": 2,
                "end_line": 2,
                "new_content": "replaced"
            }))
            .await
            .unwrap();

        assert!(result["success"].as_bool().unwrap());
        assert_eq!(result["lines_replaced"], 1);
        let content = std::fs::read_to_string(&file_path).unwrap();
        assert_eq!(content, "line1\nreplaced\nline3\n");
    }

    #[tokio::test]
    async fn test_edit_file_lines_first() {
        let temp_dir = TempDir::new().unwrap();
        let file_path = temp_dir.path().join("test.rs");
        std::fs::write(&file_path, "line1\nline2\nline3\n").unwrap();

        let tool = EditFileLinesTool::new(temp_dir.path().to_path_buf());
        let result = tool
            .execute(json!({
                "path": "test.rs",
                "start_line": 1,
                "end_line": 1,
                "new_content": "new_line1"
            }))
            .await
            .unwrap();

        assert!(result["success"].as_bool().unwrap());
        let content = std::fs::read_to_string(&file_path).unwrap();
        assert_eq!(content, "new_line1\nline2\nline3\n");
    }

    #[tokio::test]
    async fn test_edit_file_lines_last() {
        let temp_dir = TempDir::new().unwrap();
        let file_path = temp_dir.path().join("test.rs");
        std::fs::write(&file_path, "line1\nline2\nline3\n").unwrap();

        let tool = EditFileLinesTool::new(temp_dir.path().to_path_buf());
        let result = tool
            .execute(json!({
                "path": "test.rs",
                "start_line": 3,
                "end_line": 3,
                "new_content": "new_line3"
            }))
            .await
            .unwrap();

        assert!(result["success"].as_bool().unwrap());
        let content = std::fs::read_to_string(&file_path).unwrap();
        assert_eq!(content, "line1\nline2\nnew_line3\n");
    }

    #[tokio::test]
    async fn test_edit_file_lines_multi_line_replacement() {
        let temp_dir = TempDir::new().unwrap();
        let file_path = temp_dir.path().join("test.rs");
        std::fs::write(&file_path, "fn foo() {\n    old();\n}\n").unwrap();

        let tool = EditFileLinesTool::new(temp_dir.path().to_path_buf());
        let result = tool
            .execute(json!({
                "path": "test.rs",
                "start_line": 1,
                "end_line": 3,
                "new_content": "fn foo() {\n    new_a();\n    new_b();\n}"
            }))
            .await
            .unwrap();

        assert!(result["success"].as_bool().unwrap());
        assert_eq!(result["lines_replaced"], 3);
        assert_eq!(result["new_line_count"], 4);
        let content = std::fs::read_to_string(&file_path).unwrap();
        assert!(content.contains("new_a()"));
        assert!(content.contains("new_b()"));
        assert!(!content.contains("old()"));
    }

    #[tokio::test]
    async fn test_edit_file_lines_delete() {
        let temp_dir = TempDir::new().unwrap();
        let file_path = temp_dir.path().join("test.rs");
        std::fs::write(&file_path, "line1\ndelete_me\nline3\n").unwrap();

        let tool = EditFileLinesTool::new(temp_dir.path().to_path_buf());
        let result = tool
            .execute(json!({
                "path": "test.rs",
                "start_line": 2,
                "end_line": 2,
                "new_content": ""
            }))
            .await
            .unwrap();

        assert!(result["success"].as_bool().unwrap());
        let content = std::fs::read_to_string(&file_path).unwrap();
        assert_eq!(content, "line1\nline3\n");
    }

    #[tokio::test]
    async fn test_edit_file_lines_out_of_bounds() {
        let temp_dir = TempDir::new().unwrap();
        let file_path = temp_dir.path().join("test.rs");
        std::fs::write(&file_path, "line1\nline2\n").unwrap();

        let tool = EditFileLinesTool::new(temp_dir.path().to_path_buf());
        let result = tool
            .execute(json!({
                "path": "test.rs",
                "start_line": 5,
                "end_line": 5,
                "new_content": "x"
            }))
            .await;

        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_edit_file_lines_end_before_start() {
        let temp_dir = TempDir::new().unwrap();
        let file_path = temp_dir.path().join("test.rs");
        std::fs::write(&file_path, "line1\nline2\n").unwrap();

        let tool = EditFileLinesTool::new(temp_dir.path().to_path_buf());
        let result = tool
            .execute(json!({
                "path": "test.rs",
                "start_line": 2,
                "end_line": 1,
                "new_content": "x"
            }))
            .await;

        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_edit_file_lines_preserves_no_trailing_newline() {
        let temp_dir = TempDir::new().unwrap();
        let file_path = temp_dir.path().join("test.rs");
        // File without trailing newline
        std::fs::write(&file_path, "line1\nline2").unwrap();

        let tool = EditFileLinesTool::new(temp_dir.path().to_path_buf());
        tool.execute(json!({
            "path": "test.rs",
            "start_line": 1,
            "end_line": 1,
            "new_content": "replaced"
        }))
        .await
        .unwrap();

        let content = std::fs::read_to_string(&file_path).unwrap();
        assert_eq!(content, "replaced\nline2");
    }
}
