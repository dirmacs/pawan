//! File read/write tools

use super::Tool;
use async_trait::async_trait;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

/// Normalize a path relative to the workspace root.
///
/// Handles the double-prefix bug where the model passes an absolute path
/// like "/opt/pawan/grind/opt/pawan/grind/foo.rs" — it joined the workspace
/// root with an absolute path instead of a relative one. We detect the
/// workspace root appearing twice and collapse to the second occurrence.
///
/// # Parameters
/// - `workspace_root`: The root directory of the workspace
/// - `path`: The path to normalize (can be relative or absolute)
///
/// # Returns
/// The normalized path as a PathBuf
pub fn normalize_path(workspace_root: &Path, path: &str) -> PathBuf {
    let p = PathBuf::from(path);
    if p.is_absolute() {
        let ws = workspace_root.to_string_lossy();
        let ps = p.to_string_lossy();
        // If path starts with workspace_root, check if ws appears again in the remainder
        if ps.starts_with(&*ws) {
            let tail = &ps[ws.len()..];
            if let Some(idx) = tail.find(&*ws) {
                let corrected = &tail[idx..];
                tracing::warn!(
                    original = %ps, corrected = %corrected,
                    "Path normalization: double workspace prefix detected"
                );
                return PathBuf::from(corrected.to_string());
            }
        }
        p
    } else {
        workspace_root.join(p)
    }
}

/// Tool for reading file contents
pub struct ReadFileTool {
    workspace_root: PathBuf,
}

impl ReadFileTool {
    pub fn new(workspace_root: PathBuf) -> Self {
        Self { workspace_root }
    }

    fn resolve_path(&self, path: &str) -> PathBuf {
        normalize_path(&self.workspace_root, path)
    }
}

#[async_trait]
impl Tool for ReadFileTool {
    fn name(&self) -> &str {
        "read_file"
    }

    fn description(&self) -> &str {
        "Read the contents of a file. Returns the file content with line numbers."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Path to the file to read (relative to workspace root or absolute)"
                },
                "offset": {
                    "type": "integer",
                    "description": "Line number to start reading from (0-based, optional)"
                },
                "limit": {
                    "type": "integer",
                    "description": "Maximum number of lines to read (optional, defaults to 2000)"
                }
            },
            "required": ["path"]
        })
    }

    async fn execute(&self, args: Value) -> crate::Result<Value> {
        let path = args["path"]
            .as_str()
            .ok_or_else(|| crate::PawanError::Tool("path is required".into()))?;

        let offset = args["offset"].as_u64().unwrap_or(0) as usize;
        let limit = args["limit"].as_u64().unwrap_or(200) as usize;

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

        let lines: Vec<&str> = content.lines().collect();
        let total_lines = lines.len();

        let selected_lines: Vec<String> = lines
            .into_iter()
            .skip(offset)
            .take(limit)
            .enumerate()
            .map(|(i, line)| {
                let line_num = offset + i + 1;
                // Truncate very long lines
                let display_line = if line.len() > 2000 {
                    format!("{}...[truncated]", &line[..2000])
                } else {
                    line.to_string()
                };
                format!("{:>6}\t{}", line_num, display_line)
            })
            .collect();

        let output = selected_lines.join("\n");

        let warning = if total_lines > 300 && selected_lines.len() == total_lines {
            Some(format!(
                "Large file ({} lines). Consider using offset/limit to read specific sections, \
                 or use anchor_text in edit_file_lines to avoid line-number math.",
                total_lines
            ))
        } else {
            None
        };

        Ok(json!({
            "content": output,
            "path": full_path.display().to_string(),
            "total_lines": total_lines,
            "lines_shown": selected_lines.len(),
            "offset": offset,
            "warning": warning
        }))
    }
}

/// Tool for writing file contents
pub struct WriteFileTool {
    workspace_root: PathBuf,
}

impl WriteFileTool {
    pub fn new(workspace_root: PathBuf) -> Self {
        Self { workspace_root }
    }

    fn resolve_path(&self, path: &str) -> PathBuf {
        normalize_path(&self.workspace_root, path)
    }
}

#[async_trait]
impl Tool for WriteFileTool {
    fn name(&self) -> &str {
        "write_file"
    }

    fn description(&self) -> &str {
        "Write content to a file. Creates parent directories if needed. Overwrites existing content."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Path to the file to write (relative to workspace root or absolute)"
                },
                "content": {
                    "type": "string",
                    "description": "Content to write to the file"
                }
            },
            "required": ["path", "content"]
        })
    }

    async fn execute(&self, args: Value) -> crate::Result<Value> {
        let path = args["path"]
            .as_str()
            .ok_or_else(|| crate::PawanError::Tool("path is required".into()))?;

        let content = args["content"]
            .as_str()
            .ok_or_else(|| crate::PawanError::Tool("content is required".into()))?;

        let full_path = self.resolve_path(path);

        // Create parent directories if needed
        if let Some(parent) = full_path.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(crate::PawanError::Io)?;
        }

        // Write the file
        tokio::fs::write(&full_path, content)
            .await
            .map_err(crate::PawanError::Io)?;

        // Verify written size matches expected
        let written_size = tokio::fs::metadata(&full_path)
            .await
            .map(|m| m.len() as usize)
            .unwrap_or(0);
        let line_count = content.lines().count();
        let size_mismatch = written_size != content.len();

        Ok(json!({
            "success": true,
            "path": full_path.display().to_string(),
            "bytes_written": content.len(),
            "bytes_on_disk": written_size,
            "size_verified": !size_mismatch,
            "lines": line_count
        }))
    }
}

/// Tool for listing directory contents
pub struct ListDirectoryTool {
    workspace_root: PathBuf,
}

impl ListDirectoryTool {
    pub fn new(workspace_root: PathBuf) -> Self {
        Self { workspace_root }
    }

    fn resolve_path(&self, path: &str) -> PathBuf {
        normalize_path(&self.workspace_root, path)
    }
}

#[async_trait]
impl Tool for ListDirectoryTool {
    fn name(&self) -> &str {
        "list_directory"
    }

    fn description(&self) -> &str {
        "List the contents of a directory."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Path to the directory to list (relative to workspace root or absolute)"
                },
                "recursive": {
                    "type": "boolean",
                    "description": "Whether to list recursively (default: false)"
                },
                "max_depth": {
                    "type": "integer",
                    "description": "Maximum depth for recursive listing (default: 3)"
                }
            },
            "required": ["path"]
        })
    }

    async fn execute(&self, args: Value) -> crate::Result<Value> {
        let path = args["path"]
            .as_str()
            .ok_or_else(|| crate::PawanError::Tool("path is required".into()))?;

        let recursive = args["recursive"].as_bool().unwrap_or(false);
        let max_depth = args["max_depth"].as_u64().unwrap_or(3) as usize;

        let full_path = self.resolve_path(path);

        if !full_path.exists() {
            return Err(crate::PawanError::NotFound(format!(
                "Directory not found: {}",
                full_path.display()
            )));
        }

        if !full_path.is_dir() {
            return Err(crate::PawanError::Tool(format!(
                "Not a directory: {}",
                full_path.display()
            )));
        }

        let mut entries = Vec::new();

        if recursive {
            for entry in walkdir::WalkDir::new(&full_path)
                .max_depth(max_depth)
                .into_iter()
                .filter_map(|e| e.ok())
            {
                let path = entry.path();
                let relative = path.strip_prefix(&full_path).unwrap_or(path);
                let is_dir = entry.file_type().is_dir();
                let size = if is_dir {
                    0
                } else {
                    entry.metadata().map(|m| m.len()).unwrap_or(0)
                };

                entries.push(json!({
                    "path": relative.display().to_string(),
                    "is_dir": is_dir,
                    "size": size
                }));
            }
        } else {
            let mut read_dir = tokio::fs::read_dir(&full_path)
                .await
                .map_err(crate::PawanError::Io)?;

            while let Some(entry) = read_dir.next_entry().await.map_err(crate::PawanError::Io)? {
                let path = entry.path();
                let name = entry.file_name().to_string_lossy().to_string();
                let metadata = entry.metadata().await.ok();
                let is_dir = metadata.as_ref().map(|m| m.is_dir()).unwrap_or(false);
                let size = metadata.map(|m| m.len()).unwrap_or(0);

                entries.push(json!({
                    "name": name,
                    "path": path.display().to_string(),
                    "is_dir": is_dir,
                    "size": size
                }));
            }
        }

        Ok(json!({
            "path": full_path.display().to_string(),
            "entries": entries,
            "count": entries.len()
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[tokio::test]
    async fn test_read_file() {
        let temp_dir = TempDir::new().unwrap();
        let file_path = temp_dir.path().join("test.txt");
        std::fs::write(&file_path, "line 1\nline 2\nline 3").unwrap();

        let tool = ReadFileTool::new(temp_dir.path().to_path_buf());
        let result = tool.execute(json!({"path": "test.txt"})).await.unwrap();

        assert_eq!(result["total_lines"], 3);
        assert!(result["content"].as_str().unwrap().contains("line 1"));
    }

    #[tokio::test]
    async fn test_write_file() {
        let temp_dir = TempDir::new().unwrap();

        let tool = WriteFileTool::new(temp_dir.path().to_path_buf());
        let result = tool
            .execute(json!({
                "path": "new_file.txt",
                "content": "hello\nworld"
            }))
            .await
            .unwrap();

        assert!(result["success"].as_bool().unwrap());
        assert_eq!(result["lines"], 2);

        let content = std::fs::read_to_string(temp_dir.path().join("new_file.txt")).unwrap();
        assert_eq!(content, "hello\nworld");
    }

    #[tokio::test]
    async fn test_list_directory() {
        let temp_dir = TempDir::new().unwrap();
        std::fs::write(temp_dir.path().join("file1.txt"), "content").unwrap();
        std::fs::write(temp_dir.path().join("file2.txt"), "content").unwrap();
        std::fs::create_dir(temp_dir.path().join("subdir")).unwrap();

        let tool = ListDirectoryTool::new(temp_dir.path().to_path_buf());
        let result = tool.execute(json!({"path": "."})).await.unwrap();

        assert_eq!(result["count"], 3);
    }

    #[test]
    fn test_normalize_path_double_prefix() {
        let ws = PathBuf::from("/opt/pawan/grind");
        // Model passes absolute path with workspace root repeated
        let bad = "/opt/pawan/grind/opt/pawan/grind/leftist_heap/src/lib.rs";
        let result = normalize_path(&ws, bad);
        assert_eq!(result, PathBuf::from("/opt/pawan/grind/leftist_heap/src/lib.rs"));
    }

    #[test]
    fn test_normalize_path_normal_absolute() {
        let ws = PathBuf::from("/opt/pawan/grind");
        let normal = "/opt/pawan/grind/trie/src/lib.rs";
        let result = normalize_path(&ws, normal);
        assert_eq!(result, PathBuf::from("/opt/pawan/grind/trie/src/lib.rs"));
    }

    #[test]
    fn test_normalize_path_relative() {
        let ws = PathBuf::from("/opt/pawan/grind");
        let rel = "trie/src/lib.rs";
        let result = normalize_path(&ws, rel);
        assert_eq!(result, PathBuf::from("/opt/pawan/grind/trie/src/lib.rs"));
    }

    #[test]
    fn test_normalize_path_unrelated_absolute() {
        let ws = PathBuf::from("/opt/pawan/grind");
        let other = "/tmp/foo/bar.rs";
        let result = normalize_path(&ws, other);
        assert_eq!(result, PathBuf::from("/tmp/foo/bar.rs"));
    }
}
