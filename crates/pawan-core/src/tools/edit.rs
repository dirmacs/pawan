//! Edit tool for precise string replacement with write safety

use super::Tool;
use super::file::{normalize_path, validate_file_write};
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
        normalize_path(&self.workspace_root, path)
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

    fn thulp_definition(&self) -> thulp_core::ToolDefinition {
        use thulp_core::{Parameter, ParameterType};
        thulp_core::ToolDefinition::builder("edit_file")
            .description(self.description())
            .parameter(Parameter::builder("path").param_type(ParameterType::String).required(true)
                .description("Path to the file to edit").build())
            .parameter(Parameter::builder("old_string").param_type(ParameterType::String).required(true)
                .description("The exact string to find and replace").build())
            .parameter(Parameter::builder("new_string").param_type(ParameterType::String).required(true)
                .description("The string to replace it with").build())
            .parameter(Parameter::builder("replace_all").param_type(ParameterType::Boolean).required(false)
                .description("Replace all occurrences (default: false)").build())
            .build()
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
        validate_file_write(&full_path).map_err(|r| crate::PawanError::Tool(format!("Edit blocked: {} — {}", full_path.display(), r)))?;

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
        normalize_path(&self.workspace_root, path)
    }
}

#[async_trait]
impl Tool for EditFileLinesTool {
    fn name(&self) -> &str {
        "edit_file_lines"
    }

    fn description(&self) -> &str {
        "PREFERRED edit tool. Replace lines in a file. Two modes:\n\
         Mode 1 (line numbers): pass start_line + end_line (1-based, inclusive).\n\
         Mode 2 (anchor — MORE RELIABLE): pass anchor_text + anchor_count instead of line numbers. \
         The tool finds the line containing anchor_text, then replaces anchor_count lines starting from that line.\n\
         Always prefer Mode 2 (anchor) to avoid line-number miscounting.\n\
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
                    "description": "First line to replace (1-based, inclusive). Optional if anchor_text is provided."
                },
                "end_line": {
                    "type": "integer",
                    "description": "Last line to replace (1-based, inclusive). Optional if anchor_text is provided."
                },
                "anchor_text": {
                    "type": "string",
                    "description": "PREFERRED: unique text that appears on the first line to replace. The tool finds this line automatically — no line-number math needed."
                },
                "anchor_count": {
                    "type": "integer",
                    "description": "Number of lines to replace starting from the anchor line (default: 1). Only used with anchor_text."
                },
                "new_content": {
                    "type": "string",
                    "description": "Replacement text for the specified lines. Empty string to delete lines."
                }
            },
            "required": ["path", "new_content"]
        })
    }

    fn thulp_definition(&self) -> thulp_core::ToolDefinition {
        use thulp_core::{Parameter, ParameterType};
        thulp_core::ToolDefinition::builder("edit_file_lines")
            .description(self.description())
            .parameter(Parameter::builder("path").param_type(ParameterType::String).required(true)
                .description("Path to the file to edit").build())
            .parameter(Parameter::builder("start_line").param_type(ParameterType::Integer).required(false)
                .description("First line to replace (1-based, inclusive). Optional if anchor_text is provided.").build())
            .parameter(Parameter::builder("end_line").param_type(ParameterType::Integer).required(false)
                .description("Last line to replace (1-based, inclusive). Optional if anchor_text is provided.").build())
            .parameter(Parameter::builder("anchor_text").param_type(ParameterType::String).required(false)
                .description("PREFERRED: unique text on the first line to replace. No line-number math needed.").build())
            .parameter(Parameter::builder("anchor_count").param_type(ParameterType::Integer).required(false)
                .description("Number of lines to replace starting from anchor line (default: 1).").build())
            .parameter(Parameter::builder("new_content").param_type(ParameterType::String).required(true)
                .description("Replacement text for the specified lines. Empty string to delete lines.").build())
            .build()
    }

    async fn execute(&self, args: Value) -> crate::Result<Value> {
        let path = args["path"]
            .as_str()
            .ok_or_else(|| crate::PawanError::Tool("path is required".into()))?;

        let full_path = self.resolve_path(path);
        validate_file_write(&full_path).map_err(|r| crate::PawanError::Tool(format!("Edit blocked: {} — {}", full_path.display(), r)))?;
        if !full_path.exists() {
            return Err(crate::PawanError::NotFound(format!(
                "File not found: {}", full_path.display()
            )));
        }

        let content = tokio::fs::read_to_string(&full_path)
            .await
            .map_err(crate::PawanError::Io)?;

        let had_trailing_newline = content.ends_with('\n');
        let lines: Vec<&str> = content.lines().collect();
        let total_lines = lines.len();

        // Resolve start_line and end_line — either from explicit numbers or anchor
        let (start_line, end_line) = if let Some(anchor) = args["anchor_text"].as_str() {
            // Anchor mode: find line containing anchor_text
            // Fuzzy matching: normalize whitespace for comparison
            let anchor_count = args["anchor_count"].as_u64().unwrap_or(1) as usize;
            let anchor_normalized: String = anchor.split_whitespace().collect::<Vec<_>>().join(" ");
            let found = lines.iter().position(|l| {
                // Try exact match first
                if l.contains(anchor) { return true; }
                // Then try whitespace-normalized match
                let line_normalized: String = l.split_whitespace().collect::<Vec<_>>().join(" ");
                line_normalized.contains(&anchor_normalized)
            });
            match found {
                Some(idx) => {
                    let start = idx + 1; // convert to 1-based
                    let end = (start + anchor_count - 1).min(total_lines);
                    (start, end)
                }
                None => {
                    // Try case-insensitive as last resort
                    let anchor_lower = anchor_normalized.to_lowercase();
                    let found_ci = lines.iter().position(|l| {
                        let norm: String = l.split_whitespace().collect::<Vec<_>>().join(" ").to_lowercase();
                        norm.contains(&anchor_lower)
                    });
                    match found_ci {
                        Some(idx) => {
                            let start = idx + 1;
                            let end = (start + anchor_count - 1).min(total_lines);
                            (start, end)
                        }
                        None => {
                            return Err(crate::PawanError::Tool(format!(
                                "anchor_text {:?} not found in file ({} lines). Try a shorter or different anchor string.",
                                anchor, total_lines
                            )));
                        }
                    }
                }
            }
        } else {
            // Line number mode
            let start = args["start_line"]
                .as_u64()
                .ok_or_else(|| crate::PawanError::Tool(
                    "Either anchor_text or start_line+end_line is required".into()
                ))? as usize;
            let end = args["end_line"]
                .as_u64()
                .ok_or_else(|| crate::PawanError::Tool("end_line is required".into()))? as usize;
            (start, end)
        };

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

        if start_line > total_lines {
            return Err(crate::PawanError::Tool(format!(
                "start_line ({start_line}) exceeds file length ({total_lines} lines). \
                 TIP: use anchor_text instead of line numbers to avoid this error."
            )));
        }

        if end_line > total_lines {
            return Err(crate::PawanError::Tool(format!(
                "end_line ({end_line}) exceeds file length ({total_lines} lines). \
                 TIP: use anchor_text instead of line numbers to avoid this error."
            )));
        }

        let new_lines: Vec<&str> = new_content.lines().collect();
        let lines_replaced = end_line - start_line + 1;

        // Context echo: capture what's being replaced (helps LLM verify correctness)
        let replaced_lines: Vec<String> = lines[start_line - 1..end_line]
            .iter()
            .enumerate()
            .map(|(i, l)| format!("{:>4} | {}", start_line + i, l))
            .collect();
        let replaced_preview = replaced_lines.join("\n");

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
            "replaced_content": replaced_preview,
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

/// Tool for inserting text after a line matching a pattern.
/// Safer than edit_file_lines for additions — never replaces existing content.
pub struct InsertAfterTool {
/// Tool for inserting text after a line matching a pattern.
    workspace_root: PathBuf,
}

impl InsertAfterTool {
    pub fn new(workspace_root: PathBuf) -> Self {
        Self { workspace_root }
    }

    fn resolve_path(&self, path: &str) -> PathBuf {
        normalize_path(&self.workspace_root, path)
    }
}

#[async_trait]
impl Tool for InsertAfterTool {
    fn name(&self) -> &str {
        "insert_after"
    }

    fn description(&self) -> &str {
        "Insert text after a line matching a pattern. Finds the FIRST line containing \
         the anchor text. If that line opens a block (ends with '{'), inserts AFTER the \
         closing '}' of that block — safe for functions, structs, impls. Otherwise inserts \
         on the next line. Does not replace anything. Use for adding new code."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": { "type": "string", "description": "Path to the file" },
                "anchor_text": { "type": "string", "description": "Text to find — insertion happens AFTER this line" },
                "content": { "type": "string", "description": "Text to insert after the anchor line" }
            },
            "required": ["path", "anchor_text", "content"]
        })
    }

    fn thulp_definition(&self) -> thulp_core::ToolDefinition {
        use thulp_core::{Parameter, ParameterType};
        thulp_core::ToolDefinition::builder("insert_after")
            .description(self.description())
            .parameter(Parameter::builder("path").param_type(ParameterType::String).required(true)
                .description("Path to the file").build())
            .parameter(Parameter::builder("anchor_text").param_type(ParameterType::String).required(true)
                .description("Text to find — insertion happens AFTER this line").build())
            .parameter(Parameter::builder("content").param_type(ParameterType::String).required(true)
                .description("Text to insert after the anchor line").build())
            .build()
    }

    async fn execute(&self, args: Value) -> crate::Result<Value> {
        let path = args["path"].as_str()
            .ok_or_else(|| crate::PawanError::Tool("path is required".into()))?;
        let anchor = args["anchor_text"].as_str()
            .ok_or_else(|| crate::PawanError::Tool("anchor_text is required".into()))?;
        let insert_content = args["content"].as_str()
            .ok_or_else(|| crate::PawanError::Tool("content is required".into()))?;

        let full_path = self.resolve_path(path);
        validate_file_write(&full_path).map_err(|r| crate::PawanError::Tool(format!("Edit blocked: {} — {}", full_path.display(), r)))?;
        if !full_path.exists() {
            return Err(crate::PawanError::NotFound(format!("File not found: {}", full_path.display())));
        }

        let content = tokio::fs::read_to_string(&full_path).await.map_err(crate::PawanError::Io)?;
        let had_trailing_newline = content.ends_with('\n');
        let mut lines: Vec<String> = content.lines().map(|l| l.to_string()).collect();

        // Fuzzy anchor matching: exact → whitespace-normalized → case-insensitive
        let anchor_normalized: String = anchor.split_whitespace().collect::<Vec<_>>().join(" ");
        let found = lines.iter().position(|l| {
            if l.contains(anchor) { return true; }
            let norm: String = l.split_whitespace().collect::<Vec<_>>().join(" ");
            norm.contains(&anchor_normalized) || norm.to_lowercase().contains(&anchor_normalized.to_lowercase())
        });
        match found {
            Some(idx) => {
                let insert_lines: Vec<String> = insert_content.lines().map(|l| l.to_string()).collect();
                let insert_count = insert_lines.len();

                // Smart insertion: if anchor line opens a block ({), insert AFTER the block closes
                let anchor_line = &lines[idx];
                let insert_at = if anchor_line.trim_end().ends_with('{') {
                    // Find matching closing brace
                    let mut depth = 0i32;
                    let mut close_idx = idx;
                    for (i, line) in lines.iter().enumerate().skip(idx) {
                        for ch in line.chars() {
                            if ch == '{' { depth += 1; }
                            if ch == '}' { depth -= 1; }
                        }
                        if depth == 0 {
                            close_idx = i;
                            break;
                        }
                    }
                    close_idx + 1
                } else {
                    idx + 1
                };
                for (i, line) in insert_lines.into_iter().enumerate() {
                    lines.insert(insert_at + i, line);
                }
                let mut new_content = lines.join("\n");
                if had_trailing_newline { new_content.push('\n'); }
                let diff = generate_diff(&content, &new_content, path);
                tokio::fs::write(&full_path, &new_content).await.map_err(crate::PawanError::Io)?;
                let block_skipped = insert_at != idx + 1;
                Ok(json!({
                    "success": true,
                    "path": full_path.display().to_string(),
                    "anchor_line": idx + 1,
                    "inserted_after_line": insert_at,
                    "block_skipped": block_skipped,
                    "block_skip_note": if block_skipped { format!("Anchor line {} opens a block — inserted after closing '}}' at line {}", idx + 1, insert_at) } else { String::new() },
                    "lines_inserted": insert_count,
                    "anchor_matched": lines.get(idx).unwrap_or(&String::new()).trim(),
                    "diff": diff
                }))
            }
            None => Err(crate::PawanError::Tool(format!(
                "anchor_text {:?} not found in file", anchor
            ))),
        }
    }
}

/// Tool for appending content to the end of a file
pub struct AppendFileTool {
    workspace_root: PathBuf,
}

impl AppendFileTool {
    pub fn new(workspace_root: PathBuf) -> Self {
        Self { workspace_root }
    }

    fn resolve_path(&self, path: &str) -> PathBuf {
        normalize_path(&self.workspace_root, path)
    }
}

#[async_trait]
impl Tool for AppendFileTool {
    fn name(&self) -> &str {
        "append_file"
    }

    fn description(&self) -> &str {
        "Append content to the end of a file. Creates the file if it doesn't exist. \
         Use for adding new functions, tests, or sections without touching existing content. \
         Safer than write_file for large additions."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": { "type": "string", "description": "Path to the file" },
                "content": { "type": "string", "description": "Content to append" }
            },
            "required": ["path", "content"]
        })
    }

    fn thulp_definition(&self) -> thulp_core::ToolDefinition {
        use thulp_core::{Parameter, ParameterType};
        thulp_core::ToolDefinition::builder("append_file")
            .description(self.description())
            .parameter(Parameter::builder("path").param_type(ParameterType::String).required(true)
                .description("Path to the file").build())
            .parameter(Parameter::builder("content").param_type(ParameterType::String).required(true)
                .description("Content to append").build())
            .build()
    }

    async fn execute(&self, args: Value) -> crate::Result<Value> {
        let path = args["path"].as_str()
            .ok_or_else(|| crate::PawanError::Tool("path is required".into()))?;
        let append_content = args["content"].as_str()
            .ok_or_else(|| crate::PawanError::Tool("content is required".into()))?;

        let full_path = self.resolve_path(path);
        if let Some(parent) = full_path.parent() {
            tokio::fs::create_dir_all(parent).await.map_err(crate::PawanError::Io)?;
        }

        let existing = if full_path.exists() {
            tokio::fs::read_to_string(&full_path).await.map_err(crate::PawanError::Io)?
        } else {
            String::new()
        };

        let separator = if existing.is_empty() || existing.ends_with('\n') { "" } else { "\n" };
        let new_content = format!("{}{}{}\n", existing, separator, append_content);
        let appended_lines = append_content.lines().count();

        tokio::fs::write(&full_path, &new_content).await.map_err(crate::PawanError::Io)?;

        Ok(json!({
            "success": true,
            "path": full_path.display().to_string(),
            "lines_appended": appended_lines,
            "total_lines": new_content.lines().count()
        }))
    }
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

    // --- EditFileLinesTool anchor mode tests ---

    #[tokio::test]
    async fn test_edit_file_lines_anchor_mode_finds_and_replaces() {
        let temp_dir = TempDir::new().unwrap();
        let file_path = temp_dir.path().join("test.rs");
        std::fs::write(
            &file_path,
            "fn alpha() {}\nfn beta() {}\nfn gamma() {}\n",
        )
        .unwrap();

        let tool = EditFileLinesTool::new(temp_dir.path().to_path_buf());
        let result = tool
            .execute(json!({
                "path": "test.rs",
                "anchor_text": "fn beta",
                "anchor_count": 1,
                "new_content": "fn beta_renamed() {}"
            }))
            .await
            .unwrap();

        assert!(result["success"].as_bool().unwrap());
        assert_eq!(result["lines_replaced"], 1);
        let content = std::fs::read_to_string(&file_path).unwrap();
        assert!(content.contains("fn beta_renamed() {}"));
        assert!(content.contains("fn alpha() {}"));
        assert!(content.contains("fn gamma() {}"));
        assert!(!content.contains("fn beta() {}"));
    }

    #[tokio::test]
    async fn test_edit_file_lines_anchor_not_found_errors() {
        let temp_dir = TempDir::new().unwrap();
        let file_path = temp_dir.path().join("test.rs");
        std::fs::write(&file_path, "fn alpha() {}\n").unwrap();

        let tool = EditFileLinesTool::new(temp_dir.path().to_path_buf());
        let result = tool
            .execute(json!({
                "path": "test.rs",
                "anchor_text": "nonexistent_function_xyz",
                "new_content": "replacement"
            }))
            .await;

        assert!(result.is_err());
        let err_msg = format!("{}", result.unwrap_err());
        assert!(err_msg.contains("not found"), "got: {}", err_msg);
    }

    // --- InsertAfterTool tests (previously ZERO coverage) ---

    #[tokio::test]
    async fn test_insert_after_simple_line() {
        let temp_dir = TempDir::new().unwrap();
        let file_path = temp_dir.path().join("test.rs");
        std::fs::write(&file_path, "// header\nfn existing() {}\n").unwrap();

        let tool = InsertAfterTool::new(temp_dir.path().to_path_buf());
        let result = tool
            .execute(json!({
                "path": "test.rs",
                "anchor_text": "// header",
                "content": "// new comment"
            }))
            .await
            .unwrap();

        assert!(result["success"].as_bool().unwrap());
        assert_eq!(result["lines_inserted"], 1);
        assert_eq!(result["block_skipped"], false);
        let content = std::fs::read_to_string(&file_path).unwrap();
        assert_eq!(content, "// header\n// new comment\nfn existing() {}\n");
    }

    #[tokio::test]
    async fn test_insert_after_block_skip() {
        // Anchor line ends with '{' — insert should jump past the closing '}'
        let temp_dir = TempDir::new().unwrap();
        let file_path = temp_dir.path().join("test.rs");
        std::fs::write(
            &file_path,
            "fn first() {\n    println!(\"a\");\n}\nfn third() {}\n",
        )
        .unwrap();

        let tool = InsertAfterTool::new(temp_dir.path().to_path_buf());
        let result = tool
            .execute(json!({
                "path": "test.rs",
                "anchor_text": "fn first()",
                "content": "fn second() {}"
            }))
            .await
            .unwrap();

        assert!(result["success"].as_bool().unwrap());
        assert_eq!(result["block_skipped"], true);
        let content = std::fs::read_to_string(&file_path).unwrap();
        // fn second() must appear AFTER the closing brace of fn first()
        let first_close = content.find("}\nfn second()").expect("second should be inserted after first's '}'");
        assert!(first_close > content.find("println!").unwrap());
        // And before fn third()
        assert!(content.find("fn second()").unwrap() < content.find("fn third()").unwrap());
    }

    #[tokio::test]
    async fn test_insert_after_anchor_not_found() {
        let temp_dir = TempDir::new().unwrap();
        let file_path = temp_dir.path().join("test.rs");
        std::fs::write(&file_path, "fn alpha() {}\n").unwrap();

        let tool = InsertAfterTool::new(temp_dir.path().to_path_buf());
        let result = tool
            .execute(json!({
                "path": "test.rs",
                "anchor_text": "completely_missing_marker",
                "content": "new"
            }))
            .await;

        assert!(result.is_err());
    }

    // --- AppendFileTool tests (previously ZERO coverage) ---

    #[tokio::test]
    async fn test_append_file_creates_new_file() {
        let temp_dir = TempDir::new().unwrap();
        let tool = AppendFileTool::new(temp_dir.path().to_path_buf());

        let result = tool
            .execute(json!({
                "path": "new_file.md",
                "content": "# Hello\n\nFirst line"
            }))
            .await
            .unwrap();

        assert!(result["success"].as_bool().unwrap());
        assert_eq!(result["lines_appended"], 3);
        let created = temp_dir.path().join("new_file.md");
        assert!(created.exists());
        let content = std::fs::read_to_string(&created).unwrap();
        assert_eq!(content, "# Hello\n\nFirst line\n");
    }

    #[tokio::test]
    async fn test_append_file_adds_to_existing() {
        let temp_dir = TempDir::new().unwrap();
        let file_path = temp_dir.path().join("log.txt");
        std::fs::write(&file_path, "entry one\n").unwrap();

        let tool = AppendFileTool::new(temp_dir.path().to_path_buf());
        tool.execute(json!({
            "path": "log.txt",
            "content": "entry two"
        }))
        .await
        .unwrap();

        let content = std::fs::read_to_string(&file_path).unwrap();
        assert_eq!(content, "entry one\nentry two\n");
    }
}
