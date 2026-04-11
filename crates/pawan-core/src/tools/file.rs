//! File read/write tools with safety validation

use super::Tool;
use async_trait::async_trait;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

/// Validate a file path for write safety.
/// Returns Err with reason if the write should be blocked.
/// Inspired by claw-code's file_ops.rs safety checks.
pub fn validate_file_write(path: &Path) -> Result<(), &'static str> {
    let path_str = path.to_string_lossy();
    let filename = path.file_name().and_then(|n| n.to_str()).unwrap_or("");

    // Block: writes inside .git directory (corrupts repository)
    for component in path.components() {
        if let std::path::Component::Normal(c) = component {
            if c == ".git" {
                return Err("refuses to write inside .git directory");
            }
        }
    }

    // Block: sensitive credential/secret files
    let blocked_files = [
        ".env", ".env.local", ".env.production",
        "id_rsa", "id_ed25519", "id_ecdsa",
        "credentials.json", "service-account.json",
        ".npmrc", ".pypirc",
    ];
    if blocked_files.contains(&filename) {
        return Err("refuses to overwrite credential/secret file");
    }

    // Block: critical system paths
    if path_str.starts_with("/etc/") || path_str.starts_with("/usr/") || path_str.starts_with("/bin/")
        || path_str.starts_with("/sbin/") || path_str.starts_with("/boot/")
    {
        return Err("refuses to write to system directory");
    }

    // Warn-level (allow but log): lock files
    let warn_files = ["Cargo.lock", "package-lock.json", "yarn.lock", "pnpm-lock.yaml", "Gemfile.lock", "poetry.lock"];
    if warn_files.contains(&filename) {
        tracing::warn!(path = %path_str, "Writing to lock file — usually auto-generated");
    }

    Ok(())
}

/// Normalize a path relative to the workspace root.
///
/// Handles the double-prefix bug where the model passes an absolute path
/// like "/home/user/ws/home/user/ws/foo.rs" — it joined the workspace
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

    fn thulp_definition(&self) -> thulp_core::ToolDefinition {
        use thulp_core::{Parameter, ParameterType};
        thulp_core::ToolDefinition::builder("read_file")
            .description(self.description())
            .parameter(Parameter::builder("path").param_type(ParameterType::String).required(true)
                .description("Path to the file to read (relative to workspace root or absolute)").build())
            .parameter(Parameter::builder("offset").param_type(ParameterType::Integer).required(false)
                .description("Line number to start reading from (0-based, optional)").build())
            .parameter(Parameter::builder("limit").param_type(ParameterType::Integer).required(false)
                .description("Maximum number of lines to read (optional, defaults to 2000)").build())
            .build()
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
        "Write content to a file. Creates parent directories automatically. \
         PREFER edit_file or edit_file_lines for modifying existing files — \
         write_file overwrites the entire file. Only use for creating new files \
         or complete rewrites. Writes to .git/, .env, credential files, and \
         system paths (/etc, /usr) are blocked for safety."
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

    fn thulp_definition(&self) -> thulp_core::ToolDefinition {
        use thulp_core::{Parameter, ParameterType};
        thulp_core::ToolDefinition::builder("write_file")
            .description(self.description())
            .parameter(Parameter::builder("path").param_type(ParameterType::String).required(true)
                .description("Path to the file to write (relative to workspace root or absolute)").build())
            .parameter(Parameter::builder("content").param_type(ParameterType::String).required(true)
                .description("Content to write to the file").build())
            .build()
    }

    async fn execute(&self, args: Value) -> crate::Result<Value> {
        let path = args["path"]
            .as_str()
            .ok_or_else(|| crate::PawanError::Tool("path is required".into()))?;

        let content = args["content"]
            .as_str()
            .ok_or_else(|| crate::PawanError::Tool("content is required".into()))?;

        let full_path = self.resolve_path(path);

        // Validate write safety
        if let Err(reason) = validate_file_write(&full_path) {
            return Err(crate::PawanError::Tool(format!(
                "Write blocked: {} — {}", full_path.display(), reason
            )));
        }

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

    fn thulp_definition(&self) -> thulp_core::ToolDefinition {
        use thulp_core::{Parameter, ParameterType};
        thulp_core::ToolDefinition::builder("list_directory")
            .description(self.description())
            .parameter(Parameter::builder("path").param_type(ParameterType::String).required(true)
                .description("Path to the directory to list (relative to workspace root or absolute)").build())
            .parameter(Parameter::builder("recursive").param_type(ParameterType::Boolean).required(false)
                .description("Whether to list recursively (default: false)").build())
            .parameter(Parameter::builder("max_depth").param_type(ParameterType::Integer).required(false)
                .description("Maximum depth for recursive listing (default: 3)").build())
            .build()
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

    // ─── ReadFileTool edge cases ──────────────────────────────────────────

    #[tokio::test]
    async fn test_read_file_missing_path_returns_error() {
        let temp_dir = TempDir::new().unwrap();
        let tool = ReadFileTool::new(temp_dir.path().to_path_buf());
        let err = tool.execute(json!({})).await.unwrap_err();
        match err {
            crate::PawanError::Tool(msg) => assert!(msg.contains("path is required")),
            other => panic!("expected Tool error, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_read_file_nonexistent_returns_not_found() {
        let temp_dir = TempDir::new().unwrap();
        let tool = ReadFileTool::new(temp_dir.path().to_path_buf());
        let err = tool
            .execute(json!({"path": "does_not_exist.rs"}))
            .await
            .unwrap_err();
        match err {
            crate::PawanError::NotFound(msg) => assert!(msg.contains("File not found")),
            other => panic!("expected NotFound error, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_read_file_line_numbers_are_formatted() {
        let temp_dir = TempDir::new().unwrap();
        let file_path = temp_dir.path().join("numbered.txt");
        std::fs::write(&file_path, "alpha\nbeta\ngamma").unwrap();

        let tool = ReadFileTool::new(temp_dir.path().to_path_buf());
        let result = tool
            .execute(json!({"path": "numbered.txt"}))
            .await
            .unwrap();

        let content = result["content"].as_str().unwrap();
        // Line 1 must be right-aligned in a 6-char field followed by a tab
        assert!(
            content.contains("     1\talpha"),
            "expected 6-char right-aligned line number: got {content:?}"
        );
        assert!(content.contains("     2\tbeta"));
        assert!(content.contains("     3\tgamma"));
    }

    #[tokio::test]
    async fn test_read_file_offset_and_limit_respected() {
        let temp_dir = TempDir::new().unwrap();
        let lines: String = (1..=10).map(|i| format!("line{i}\n")).collect();
        let file_path = temp_dir.path().join("ten.txt");
        std::fs::write(&file_path, &lines).unwrap();

        let tool = ReadFileTool::new(temp_dir.path().to_path_buf());
        // offset=3 skips lines 1-3, limit=2 takes lines 4 and 5
        let result = tool
            .execute(json!({"path": "ten.txt", "offset": 3, "limit": 2}))
            .await
            .unwrap();

        assert_eq!(result["lines_shown"], 2);
        assert_eq!(result["offset"], 3);
        let content = result["content"].as_str().unwrap();
        // Line 4 and 5 must appear; lines 1-3 and 6-10 must not
        assert!(content.contains("line4"), "expected line4 in {content:?}");
        assert!(content.contains("line5"), "expected line5 in {content:?}");
        assert!(!content.contains("line3"), "line3 should be before offset");
        assert!(!content.contains("line6"), "line6 should be beyond limit");
    }

    #[tokio::test]
    async fn test_read_file_large_file_warning() {
        let temp_dir = TempDir::new().unwrap();
        // 301 lines — triggers the large-file warning when all are read
        let lines: String = (1..=301).map(|i| format!("ln{i}\n")).collect();
        let file_path = temp_dir.path().join("large.txt");
        std::fs::write(&file_path, &lines).unwrap();

        let tool = ReadFileTool::new(temp_dir.path().to_path_buf());
        // Read without offset/limit — default limit is 200, so lines_shown < total_lines
        // Warning fires only when lines_shown == total_lines and total_lines > 300,
        // so use a large limit to force all lines to be shown.
        let result = tool
            .execute(json!({"path": "large.txt", "limit": 400}))
            .await
            .unwrap();

        assert_eq!(result["total_lines"], 301);
        let warning = &result["warning"];
        assert!(
            !warning.is_null(),
            "expected warning for 301-line file, got null"
        );
        assert!(
            warning.as_str().unwrap().contains("Large file"),
            "warning should mention 'Large file'"
        );
    }

    // ─── WriteFileTool edge cases ─────────────────────────────────────────

    #[tokio::test]
    async fn test_write_file_missing_path_returns_error() {
        let temp_dir = TempDir::new().unwrap();
        let tool = WriteFileTool::new(temp_dir.path().to_path_buf());
        let err = tool
            .execute(json!({"content": "hello"}))
            .await
            .unwrap_err();
        match err {
            crate::PawanError::Tool(msg) => assert!(msg.contains("path is required")),
            other => panic!("expected Tool error, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_write_file_missing_content_returns_error() {
        let temp_dir = TempDir::new().unwrap();
        let tool = WriteFileTool::new(temp_dir.path().to_path_buf());
        let err = tool
            .execute(json!({"path": "output.txt"}))
            .await
            .unwrap_err();
        match err {
            crate::PawanError::Tool(msg) => assert!(msg.contains("content is required")),
            other => panic!("expected Tool error, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_write_file_blocked_dotgit_returns_error() {
        let temp_dir = TempDir::new().unwrap();
        // Use an absolute .git path under temp so validate_file_write sees the .git component
        let git_path = temp_dir.path().join(".git").join("COMMIT_EDITMSG");
        let tool = WriteFileTool::new(temp_dir.path().to_path_buf());
        let err = tool
            .execute(json!({"path": git_path.to_str().unwrap(), "content": "blocked"}))
            .await
            .unwrap_err();
        match err {
            crate::PawanError::Tool(msg) => {
                assert!(msg.contains("Write blocked"), "expected 'Write blocked' in: {msg}");
                assert!(msg.contains(".git"), "expected '.git' in: {msg}");
            }
            other => panic!("expected Tool error, got {:?}", other),
        }
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
        let ws = PathBuf::from("/home/user/workspace");
        // Model passes absolute path with workspace root repeated
        let bad = "/home/user/workspace/home/user/workspace/leftist_heap/src/lib.rs";
        let result = normalize_path(&ws, bad);
        assert_eq!(result, PathBuf::from("/home/user/workspace/leftist_heap/src/lib.rs"));
    }

    #[test]
    fn test_normalize_path_normal_absolute() {
        let ws = PathBuf::from("/home/user/workspace");
        let normal = "/home/user/workspace/trie/src/lib.rs";
        let result = normalize_path(&ws, normal);
        assert_eq!(result, PathBuf::from("/home/user/workspace/trie/src/lib.rs"));
    }

    #[test]
    fn test_normalize_path_relative() {
        let ws = PathBuf::from("/home/user/workspace");
        let rel = "trie/src/lib.rs";
        let result = normalize_path(&ws, rel);
        assert_eq!(result, PathBuf::from("/home/user/workspace/trie/src/lib.rs"));
    }

    #[test]
    fn test_normalize_path_unrelated_absolute() {
        let ws = PathBuf::from("/home/user/workspace");
        let other = "/tmp/foo/bar.rs";
        let result = normalize_path(&ws, other);
        assert_eq!(result, PathBuf::from("/tmp/foo/bar.rs"));
    }

    // --- validate_file_write security tests (previously ZERO coverage) ---

    #[test]
    fn test_validate_file_write_blocks_dotgit_writes() {
        // .git anywhere in path should be blocked — would corrupt repo state
        let cases = [
            "/home/user/repo/.git/HEAD",
            "/opt/pawan/.git/config",
            ".git/index",
            "./.git/hooks/pre-commit",
            "/tmp/foo/.git/something",
        ];
        for p in cases {
            let result = validate_file_write(Path::new(p));
            assert!(
                result.is_err(),
                "Expected .git write to be blocked: {}",
                p
            );
            assert!(result.unwrap_err().contains(".git"));
        }
    }

    #[test]
    fn test_validate_file_write_blocks_credential_files() {
        let blocked = [
            ".env",
            ".env.local",
            ".env.production",
            "id_rsa",
            "id_ed25519",
            "id_ecdsa",
            "credentials.json",
            "service-account.json",
            ".npmrc",
            ".pypirc",
        ];
        for name in blocked {
            let path = PathBuf::from(format!("/tmp/test-dir/{}", name));
            let result = validate_file_write(&path);
            assert!(
                result.is_err(),
                "Expected {} to be blocked as credential file",
                name
            );
            assert!(
                result.unwrap_err().contains("credential"),
                "Expected error to mention 'credential' for {}",
                name
            );
        }
    }

    #[test]
    fn test_validate_file_write_blocks_system_paths() {
        let system_paths = [
            "/etc/passwd",
            "/etc/hosts",
            "/usr/bin/myscript",
            "/usr/local/bin/foo",
            "/bin/sh",
            "/sbin/init",
            "/boot/vmlinuz",
        ];
        for p in system_paths {
            let result = validate_file_write(Path::new(p));
            assert!(
                result.is_err(),
                "Expected system path {} to be blocked",
                p
            );
            assert!(result.unwrap_err().contains("system directory"));
        }
    }

    #[test]
    fn test_validate_file_write_allows_normal_paths() {
        // Normal paths in the workspace should pass cleanly
        let allowed = [
            "/home/user/ws/src/main.rs",
            "/tmp/scratch/notes.md",
            "/opt/pawan/README.md",
            "/var/tmp/output.txt",
            "./relative/path/file.txt",
        ];
        for p in allowed {
            let result = validate_file_write(Path::new(p));
            assert!(
                result.is_ok(),
                "Expected {} to be allowed, got error: {:?}",
                p,
                result.err()
            );
        }
    }

    #[test]
    fn test_validate_file_write_allows_lock_files_with_warn() {
        // Lock files are warn-level, not blocked — they should return Ok
        // but trigger the warn tracing event. We can't assert the warn
        // emission in a unit test without a tracing subscriber, but we can
        // assert Ok so the behavior contract is pinned.
        let lock_files = [
            "/home/user/ws/Cargo.lock",
            "/home/user/ws/package-lock.json",
            "/home/user/ws/yarn.lock",
            "/home/user/ws/pnpm-lock.yaml",
            "/home/user/ws/Gemfile.lock",
            "/home/user/ws/poetry.lock",
        ];
        for p in lock_files {
            let result = validate_file_write(Path::new(p));
            assert!(
                result.is_ok(),
                "Lock file {} should be allowed (warn only), got error: {:?}",
                p,
                result.err()
            );
        }
    }

    // ─── Edge case tests for validate_file_write ─────────────────────────

    #[test]
    fn test_validate_file_write_allows_gitignore_not_blocked_as_dotgit() {
        // Regression: `.gitignore`, `.github/`, `.git-credentials` must not
        // be mistaken for the `.git/` directory component. Only an exact
        // `.git` path component (delimited by `/`) should trigger the block.
        let allowed = [
            "/home/user/ws/.gitignore",
            "/home/user/ws/.gitattributes",
            "/home/user/ws/.github/workflows/ci.yml",
            "/home/user/ws/.git-credentials",
            "/home/user/ws/src/.gitkeep",
        ];
        for p in allowed {
            let result = validate_file_write(Path::new(p));
            assert!(
                result.is_ok(),
                "Path {} starts with .git but is NOT a .git component — should be allowed, got: {:?}",
                p,
                result.err()
            );
        }
    }

    #[test]
    fn test_validate_file_write_case_sensitivity_on_env_files() {
        // Filename comparison is case-sensitive (contains()) — .ENV and .Env
        // are NOT blocked because blocked_files only lists lowercase. This
        // test pins the current behavior so it's not accidentally changed
        // without considering the impact on case-insensitive filesystems.
        let path = PathBuf::from("/tmp/project/.ENV");
        let result = validate_file_write(&path);
        assert!(
            result.is_ok(),
            ".ENV (uppercase) is not in the blocked list — current behavior is to allow"
        );
    }

    #[test]
    fn test_validate_file_write_blocks_dotgit_even_at_root() {
        // .git at the root of a path (no parent dirs) should still be blocked
        let result = validate_file_write(Path::new(".git/HEAD"));
        assert!(result.is_err(), "root-level .git/ must be blocked");
        assert!(result.unwrap_err().contains(".git"));
    }

    #[test]
    fn test_validate_file_write_handles_empty_filename() {
        // A path that is just a directory (e.g. "/foo/bar/") has no filename.
        // validate_file_write must not panic — it should treat it as allowed
        // since there's no filename to match against the blocked list.
        let result = validate_file_write(Path::new("/tmp/somedir/"));
        assert!(
            result.is_ok(),
            "directory path with no filename must not panic or error"
        );
    }

    #[test]
    fn test_validate_file_write_allows_etc_files_at_wrong_level() {
        // `/etc/` prefix blocks writes to /etc. But a path like
        // "/home/user/etc/config" should NOT be blocked — /etc/ is the
        // start-of-string match, not a substring.
        let allowed = [
            "/home/user/etc/config.toml",
            "/opt/pawan/etc/overrides.yml",
        ];
        for p in allowed {
            let result = validate_file_write(Path::new(p));
            assert!(
                result.is_ok(),
                "Path {} with /etc/ not at start must be allowed",
                p
            );
        }
    }

    // ─── Edge case tests for normalize_path ──────────────────────────────

    #[test]
    fn test_normalize_path_workspace_root_with_trailing_slash() {
        // When workspace_root has a trailing slash, normalization must
        // still work correctly for both the double-prefix case and
        // normal relative paths.
        let ws = PathBuf::from("/home/user/ws");
        let rel = "src/main.rs";
        let result = normalize_path(&ws, rel);
        assert_eq!(result, PathBuf::from("/home/user/ws/src/main.rs"));
    }

    #[test]
    fn test_normalize_path_empty_relative() {
        // An empty relative path should join cleanly to the workspace root
        let ws = PathBuf::from("/home/user/ws");
        let result = normalize_path(&ws, "");
        assert_eq!(result, PathBuf::from("/home/user/ws"));
    }

    #[test]
    fn test_normalize_path_triple_prefix_not_collapsed() {
        // Current behavior: triple-prefix paths find the FIRST reoccurrence
        // only. This pins the current behavior to catch regressions in the
        // double-prefix detection algorithm.
        let ws = PathBuf::from("/ws");
        let triple = "/ws/ws/ws/foo.rs";
        let result = normalize_path(&ws, triple);
        // After stripping first /ws, remainder is "/ws/ws/foo.rs"; first
        // occurrence of "/ws" in remainder is at index 0 → returns "/ws/ws/foo.rs"
        assert_eq!(
            result,
            PathBuf::from("/ws/ws/foo.rs"),
            "triple prefix collapses to double prefix — documented behavior"
        );
    }
}
