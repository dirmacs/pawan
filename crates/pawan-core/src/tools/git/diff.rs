use super::run_git;
use super::super::Tool;
use async_trait::async_trait;
use serde_json::{json, Value};
use std::path::PathBuf;

/// Tool for getting git diff
///
/// This tool shows the differences between files in the working directory
/// and the git index, or between commits.
///
/// # Fields
/// - `workspace_root`: The root directory of the workspace
pub struct GitDiffTool {
    workspace_root: PathBuf,
}

impl GitDiffTool {
    pub fn new(workspace_root: PathBuf) -> Self {
        Self { workspace_root }
    }
}

#[async_trait]
impl Tool for GitDiffTool {
    fn name(&self) -> &str {
        "git_diff"
    }

    fn description(&self) -> &str {
        "Show git diff for staged or unstaged changes. Can diff against a specific commit or branch."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "staged": {
                    "type": "boolean",
                    "description": "Show staged changes only (--cached). Default: false (shows unstaged)"
                },
                "file": {
                    "type": "string",
                    "description": "Specific file to diff (optional)"
                },
                "base": {
                    "type": "string",
                    "description": "Base commit/branch to diff against (e.g., 'main', 'HEAD~3')"
                },
                "stat": {
                    "type": "boolean",
                    "description": "Show diffstat summary instead of full diff"
                }
            },
            "required": []
        })
    }

    fn thulp_definition(&self) -> thulp_core::ToolDefinition {
        use thulp_core::{Parameter, ParameterType};
        thulp_core::ToolDefinition::builder("git_diff")
            .description(self.description())
            .parameter(Parameter::builder("staged").param_type(ParameterType::Boolean).required(false)
                .description("Show staged changes only (--cached). Default: false (shows unstaged)").build())
            .parameter(Parameter::builder("file").param_type(ParameterType::String).required(false)
                .description("Specific file to diff (optional)").build())
            .parameter(Parameter::builder("base").param_type(ParameterType::String).required(false)
                .description("Base commit/branch to diff against (e.g., 'main', 'HEAD~3')").build())
            .parameter(Parameter::builder("stat").param_type(ParameterType::Boolean).required(false)
                .description("Show diffstat summary instead of full diff").build())
            .build()
    }

    async fn execute(&self, args: Value) -> crate::Result<Value> {
        let staged = args["staged"].as_bool().unwrap_or(false);
        let file = args["file"].as_str();
        let base = args["base"].as_str();
        let stat = args["stat"].as_bool().unwrap_or(false);

        let mut git_args = vec!["diff"];

        if staged {
            git_args.push("--cached");
        }

        if stat {
            git_args.push("--stat");
        }

        if let Some(b) = base {
            git_args.push(b);
        }

        if let Some(f) = file {
            git_args.push("--");
            git_args.push(f);
        }

        let (success, stdout, stderr) = run_git(&self.workspace_root, &git_args).await?;

        if !success {
            return Err(crate::PawanError::Git(format!(
                "git diff failed: {}",
                stderr
            )));
        }

        // Truncate if too large
        let max_size = 100_000;
        let truncated = stdout.len() > max_size;
        let diff = if truncated {
            format!(
                "{}...\n[truncated, {} bytes total]",
                &stdout[..max_size],
                stdout.len()
            )
        } else {
            stdout
        };

        Ok(json!({
            "diff": diff,
            "truncated": truncated,
            "has_changes": !diff.trim().is_empty(),
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
    async fn test_git_diff_no_changes() {
        let temp_dir = setup_git_repo().await;

        let tool = GitDiffTool::new(temp_dir.path().to_path_buf());
        let result = tool.execute(json!({})).await.unwrap();

        assert!(result["success"].as_bool().unwrap());
        assert!(!result["has_changes"].as_bool().unwrap());
    }

    #[tokio::test]
    async fn test_git_diff_schema() {
        let temp_dir = setup_git_repo().await;
        let tool = GitDiffTool::new(temp_dir.path().to_path_buf());
        let schema = tool.parameters_schema();
        let obj = schema.as_object().unwrap();
        let props = obj.get("properties").unwrap().as_object().unwrap();
        assert!(props.contains_key("staged"));
        assert!(props.contains_key("file"));
        assert!(props.contains_key("base"));
        assert!(props.contains_key("stat"));
    }

    #[tokio::test]
    async fn test_git_diff_with_changes() {
        let temp_dir = setup_git_repo().await;
        // Create, add, commit a file
        std::fs::write(temp_dir.path().join("f.txt"), "original").unwrap();
        Command::new("git").args(["add", "."]).current_dir(temp_dir.path()).output().await.unwrap();
        Command::new("git").args(["commit", "-m", "init"]).current_dir(temp_dir.path()).output().await.unwrap();
        // Modify the file
        std::fs::write(temp_dir.path().join("f.txt"), "modified").unwrap();

        let tool = GitDiffTool::new(temp_dir.path().into());
        let result = tool.execute(json!({})).await.unwrap();
        assert!(result["success"].as_bool().unwrap());
        assert!(result["has_changes"].as_bool().unwrap());
        assert!(result["diff"].as_str().unwrap().contains("modified"));
    }
}
