use super::super::Tool;
use super::run_git;
use async_trait::async_trait;
use serde_json::{json, Value};
use std::path::PathBuf;

/// Tool for checking git status
///
/// This tool provides information about the current git repository status,
/// including modified files, untracked files, and branch information.
///
/// # Fields
/// - `workspace_root`: The root directory of the workspace
pub struct GitStatusTool {
    workspace_root: PathBuf,
}

impl GitStatusTool {
    pub fn new(workspace_root: PathBuf) -> Self {
        Self { workspace_root }
    }
}

#[async_trait]
impl Tool for GitStatusTool {
    fn name(&self) -> &str {
        "git_status"
    }

    fn description(&self) -> &str {
        "Get the current git status showing staged, unstaged, and untracked files."
    }

    fn mutating(&self) -> bool {
        false // Git status is read-only
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "short": {
                    "type": "boolean",
                    "description": "Use short format output (default: false)"
                }
            },
            "required": []
        })
    }

    fn thulp_definition(&self) -> thulp_core::ToolDefinition {
        use thulp_core::{Parameter, ParameterType};
        thulp_core::ToolDefinition::builder("git_status")
            .description(self.description())
            .parameter(
                Parameter::builder("short")
                    .param_type(ParameterType::Boolean)
                    .required(false)
                    .description("Use short format output (default: false)")
                    .build(),
            )
            .build()
    }

    async fn execute(&self, args: Value) -> crate::Result<Value> {
        let short = args["short"].as_bool().unwrap_or(false);

        let mut git_args = vec!["status"];
        if short {
            git_args.push("-s");
        }

        let (success, stdout, stderr) = run_git(&self.workspace_root, &git_args).await?;

        if !success {
            return Err(crate::PawanError::Git(format!(
                "git status failed: {}",
                stderr
            )));
        }

        // Also get branch info
        let (_, branch_output, _) =
            run_git(&self.workspace_root, &["branch", "--show-current"]).await?;
        let branch = branch_output.trim().to_string();

        // Check if repo is clean
        let (_, porcelain, _) = run_git(&self.workspace_root, &["status", "--porcelain"]).await?;
        let is_clean = porcelain.trim().is_empty();

        Ok(json!({
            "status": stdout.trim(),
            "branch": branch,
            "is_clean": is_clean,
            "success": true
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
    async fn test_git_status_empty_repo() {
        let temp_dir = setup_git_repo().await;

        let tool = GitStatusTool::new(temp_dir.path().to_path_buf());
        let result = tool.execute(json!({})).await.unwrap();

        assert!(result["success"].as_bool().unwrap());
        assert!(result["is_clean"].as_bool().unwrap());
    }

    #[tokio::test]
    async fn test_git_status_with_untracked() {
        let temp_dir = setup_git_repo().await;

        // Create an untracked file
        std::fs::write(temp_dir.path().join("test.txt"), "hello").unwrap();

        let tool = GitStatusTool::new(temp_dir.path().to_path_buf());
        let result = tool.execute(json!({})).await.unwrap();

        assert!(result["success"].as_bool().unwrap());
        assert!(!result["is_clean"].as_bool().unwrap());
    }

    #[tokio::test]
    async fn test_git_status_tool_exists() {
        let temp_dir = setup_git_repo().await;
        let tool = GitStatusTool::new(temp_dir.path().to_path_buf());
        assert_eq!(tool.name(), "git_status");
    }

    #[tokio::test]
    async fn test_git_status_detects_modified_file() {
        // GitStatusTool should report modified files that were previously committed
        let temp_dir = setup_git_repo().await;
        std::fs::write(temp_dir.path().join("tracked.txt"), "v1").unwrap();
        Command::new("git")
            .args(["add", "."])
            .current_dir(temp_dir.path())
            .output()
            .await
            .unwrap();
        Command::new("git")
            .args(["commit", "-m", "init tracked"])
            .current_dir(temp_dir.path())
            .output()
            .await
            .unwrap();

        // Modify the tracked file
        std::fs::write(temp_dir.path().join("tracked.txt"), "v2").unwrap();

        let tool = GitStatusTool::new(temp_dir.path().to_path_buf());
        let result = tool.execute(json!({})).await.unwrap();
        // Verify the status includes the modified file
        let serialized = result.to_string();
        assert!(
            serialized.contains("tracked.txt"),
            "status must mention modified tracked.txt, got: {}",
            serialized
        );
    }
}
