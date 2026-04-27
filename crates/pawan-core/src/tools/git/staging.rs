use super::run_git;
use super::super::Tool;
use async_trait::async_trait;
use serde_json::{json, Value};
use std::path::PathBuf;

/// Tool for staging files
///
/// This tool adds files to the git staging area in preparation for commit.
///
/// # Fields
/// - `workspace_root`: The root directory of the workspace
pub struct GitAddTool {
    workspace_root: PathBuf,
}

impl GitAddTool {
    pub fn new(workspace_root: PathBuf) -> Self {
        Self { workspace_root }
    }
}

#[async_trait]
impl Tool for GitAddTool {
    fn name(&self) -> &str {
        "git_add"
    }

    fn description(&self) -> &str {
        "Stage files for commit. Can stage specific files or all changes."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "files": {
                    "type": "array",
                    "items": {"type": "string"},
                    "description": "List of files to stage. Use [\".\"] to stage all changes."
                },
                "all": {
                    "type": "boolean",
                    "description": "Stage all changes including untracked files (-A)"
                }
            },
            "required": []
        })
    }

    fn thulp_definition(&self) -> thulp_core::ToolDefinition {
        use thulp_core::{Parameter, ParameterType};
        thulp_core::ToolDefinition::builder("git_add")
            .description(self.description())
            .parameter(Parameter::builder("files").param_type(ParameterType::Array).required(false)
                .description("List of files to stage. Use [\".\"] to stage all changes.").build())
            .parameter(Parameter::builder("all").param_type(ParameterType::Boolean).required(false)
                .description("Stage all changes including untracked files (-A)").build())
            .build()
    }

    async fn execute(&self, args: Value) -> crate::Result<Value> {
        let all = args["all"].as_bool().unwrap_or(false);
        let files: Vec<&str> = args["files"]
            .as_array()
            .map(|arr| arr.iter().filter_map(|v| v.as_str()).collect())
            .unwrap_or_default();

        let mut git_args = vec!["add"];

        if all {
            git_args.push("-A");
        } else if files.is_empty() {
            return Err(crate::PawanError::Tool(
                "Either 'files' or 'all: true' must be specified".into(),
            ));
        } else {
            for f in &files {
                git_args.push(f);
            }
        }

        let (success, _, stderr) = run_git(&self.workspace_root, &git_args).await?;

        if !success {
            return Err(crate::PawanError::Git(format!(
                "git add failed: {}",
                stderr
            )));
        }

        // Get status after adding
        let (_, status_output, _) = run_git(&self.workspace_root, &["status", "-s"]).await?;
        let staged_count = status_output
            .lines()
            .filter(|l| l.starts_with('A') || l.starts_with('M') || l.starts_with('D'))
            .count();

        Ok(json!({
            "success": true,
            "staged_count": staged_count,
            "message": if all {
                "Staged all changes".to_string()
            } else {
                format!("Staged {} file(s)", files.len())
            }
        }))
    }
}

/// Tool for creating commits
///
/// This tool creates a new git commit with the staged changes.
///
/// # Fields
/// - `workspace_root`: The root directory of the workspace
pub struct GitCommitTool {
    workspace_root: PathBuf,
}

impl GitCommitTool {
    pub fn new(workspace_root: PathBuf) -> Self {
        Self { workspace_root }
    }
}

#[async_trait]
impl Tool for GitCommitTool {
    fn name(&self) -> &str {
        "git_commit"
    }

    fn description(&self) -> &str {
        "Create a git commit with the staged changes. Requires a commit message."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "message": {
                    "type": "string",
                    "description": "Commit message (required)"
                },
                "body": {
                    "type": "string",
                    "description": "Extended commit body (optional)"
                }
            },
            "required": ["message"]
        })
    }

    fn thulp_definition(&self) -> thulp_core::ToolDefinition {
        use thulp_core::{Parameter, ParameterType};
        thulp_core::ToolDefinition::builder("git_commit")
            .description(self.description())
            .parameter(Parameter::builder("message").param_type(ParameterType::String).required(true)
                .description("Commit message (required)").build())
            .parameter(Parameter::builder("body").param_type(ParameterType::String).required(false)
                .description("Extended commit body (optional)").build())
            .build()
    }

    async fn execute(&self, args: Value) -> crate::Result<Value> {
        let message = args["message"]
            .as_str()
            .ok_or_else(|| crate::PawanError::Tool("commit message is required".into()))?;

        let body = args["body"].as_str();

        // Check if there are staged changes
        let (_, staged, _) = run_git(&self.workspace_root, &["diff", "--cached", "--stat"]).await?;
        if staged.trim().is_empty() {
            return Err(crate::PawanError::Git(
                "No staged changes to commit. Use git_add first.".into(),
            ));
        }

        // Build commit message
        let full_message = if let Some(b) = body {
            format!("{}\n\n{}", message, b)
        } else {
            message.to_string()
        };

        let (success, stdout, stderr) =
            run_git(&self.workspace_root, &["commit", "-m", &full_message]).await?;

        if !success {
            return Err(crate::PawanError::Git(format!(
                "git commit failed: {}",
                stderr
            )));
        }

        // Get the commit hash
        let (_, hash_output, _) =
            run_git(&self.workspace_root, &["rev-parse", "--short", "HEAD"]).await?;
        let commit_hash = hash_output.trim().to_string();

        Ok(json!({
            "success": true,
            "commit_hash": commit_hash,
            "message": message,
            "output": stdout.trim()
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use tempfile::TempDir;
    use tokio::process::Command;

    async fn setup_git_repo() -> TempDir {
        let temp_dir = TempDir::new().unwrap();

        Command::new("git")
            .args(["init"])
            .current_dir(temp_dir.path())
            .output()
            .await
            .unwrap();

        Command::new("git")
            .args(["config", "user.email", "test@test.com"])
            .current_dir(temp_dir.path())
            .output()
            .await
            .unwrap();

        Command::new("git")
            .args(["config", "user.name", "Test User"])
            .current_dir(temp_dir.path())
            .output()
            .await
            .unwrap();

        temp_dir
    }

    #[tokio::test]
    async fn test_git_add_and_commit() {
        let temp_dir = setup_git_repo().await;

        // Create a file
        std::fs::write(temp_dir.path().join("test.txt"), "hello").unwrap();

        // Add the file
        let add_tool = GitAddTool::new(temp_dir.path().to_path_buf());
        let add_result = add_tool
            .execute(json!({
                "files": ["test.txt"]
            }))
            .await
            .unwrap();
        assert!(add_result["success"].as_bool().unwrap());

        // Commit
        let commit_tool = GitCommitTool::new(temp_dir.path().to_path_buf());
        let commit_result = commit_tool
            .execute(json!({
                "message": "Add test file"
            }))
            .await
            .unwrap();
        assert!(commit_result["success"].as_bool().unwrap());
        assert!(!commit_result["commit_hash"].as_str().unwrap().is_empty());
    }

    #[tokio::test]
    async fn test_git_commit_missing_message_errors() {
        let temp_dir = setup_git_repo().await;
        let tool = GitCommitTool::new(temp_dir.path().to_path_buf());
        // No "message" field
        let result = tool.execute(json!({})).await;
        assert!(result.is_err(), "commit without message must error");
    }

    #[tokio::test]
    async fn test_git_commit_multiline_message_preserved() {
        let temp_dir = setup_git_repo().await;
        std::fs::write(temp_dir.path().join("a.txt"), "content").unwrap();

        GitAddTool::new(temp_dir.path().to_path_buf())
            .execute(json!({ "files": ["a.txt"] }))
            .await
            .unwrap();

        // Commit with a message that has newlines, backticks, dollars,
        // and quotes — everything that could break shell escaping.
        let message = "feat: the subject line\n\nThis is the body.\nIt has `backticks`, $dollars, and \"quotes\".\n\nCo-Authored-By: Test <test@example.com>";
        let commit_result = GitCommitTool::new(temp_dir.path().to_path_buf())
            .execute(json!({ "message": message }))
            .await
            .unwrap();
        assert!(commit_result["success"].as_bool().unwrap());

        // Read the commit message back via git log
        use crate::tools::git::log::GitLogTool;
        let log_result = GitLogTool::new(temp_dir.path().to_path_buf())
            .execute(json!({ "count": 1 }))
            .await
            .unwrap();
        let log_str = format!("{}", log_result);
        assert!(
            log_str.contains("the subject line"),
            "log should contain subject line, got: {}",
            log_str
        );
    }

    #[tokio::test]
    async fn test_git_add_neither_files_nor_all_returns_error() {
        // GitAddTool requires either `files` (non-empty) or `all: true`.
        // Omitting both must return a specific error message.
        let temp_dir = setup_git_repo().await;
        let tool = GitAddTool::new(temp_dir.path().to_path_buf());
        let result = tool.execute(json!({})).await;
        assert!(result.is_err(), "git_add with no args must return Err");
        let err = format!("{}", result.unwrap_err());
        assert!(
            err.contains("files") && err.contains("all"),
            "error must mention both 'files' and 'all', got: {}",
            err
        );
    }

    #[tokio::test]
    async fn test_git_add_all_without_files_list_succeeds() {
        // all=true should work even when `files` is not specified at all.
        // This tests the early branch that skips the empty-files check.
        let temp_dir = setup_git_repo().await;
        std::fs::write(temp_dir.path().join("x.txt"), "a").unwrap();
        std::fs::write(temp_dir.path().join("y.txt"), "b").unwrap();

        let tool = GitAddTool::new(temp_dir.path().to_path_buf());
        let result = tool.execute(json!({ "all": true })).await.unwrap();
        assert!(result["success"].as_bool().unwrap());
        assert!(
            result["message"]
                .as_str()
                .unwrap()
                .contains("Staged all changes"),
            "all=true should report 'Staged all changes'"
        );
    }

    #[tokio::test]
    async fn test_git_add_empty_files_array_returns_error() {
        // files=[] with no all flag must ALSO error (empty array is falsy).
        let temp_dir = setup_git_repo().await;
        let tool = GitAddTool::new(temp_dir.path().to_path_buf());
        let result = tool.execute(json!({ "files": [] })).await;
        assert!(
            result.is_err(),
            "empty files array + no all flag must error"
        );
    }
}
