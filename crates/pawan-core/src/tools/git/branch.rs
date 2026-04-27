use super::run_git;
use super::super::Tool;
use async_trait::async_trait;
use serde_json::{json, Value};
use std::path::PathBuf;

/// Tool for listing and managing branches
pub struct GitBranchTool {
    workspace_root: PathBuf,
}

impl GitBranchTool {
    pub fn new(workspace_root: PathBuf) -> Self {
        Self { workspace_root }
    }
}

#[async_trait]
impl Tool for GitBranchTool {
    fn name(&self) -> &str {
        "git_branch"
    }

    fn description(&self) -> &str {
        "List branches or get current branch name. Shows local and optionally remote branches."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "all": {
                    "type": "boolean",
                    "description": "Show both local and remote branches (default: false)"
                }
            },
            "required": []
        })
    }

    fn thulp_definition(&self) -> thulp_core::ToolDefinition {
        use thulp_core::{Parameter, ParameterType};
        thulp_core::ToolDefinition::builder("git_branch")
            .description(self.description())
            .parameter(Parameter::builder("all").param_type(ParameterType::Boolean).required(false)
                .description("Show both local and remote branches (default: false)").build())
            .build()
    }

    async fn execute(&self, args: Value) -> crate::Result<Value> {
        let all = args["all"].as_bool().unwrap_or(false);

        // Get current branch
        let (_, current, _) = run_git(&self.workspace_root, &["branch", "--show-current"]).await?;
        let current_branch = current.trim().to_string();

        // List branches
        let mut git_args = vec!["branch", "--format=%(refname:short)"];
        if all {
            git_args.push("-a");
        }

        let (success, stdout, stderr) = run_git(&self.workspace_root, &git_args).await?;

        if !success {
            return Err(crate::PawanError::Git(format!(
                "git branch failed: {}",
                stderr
            )));
        }

        let branches: Vec<&str> = stdout
            .lines()
            .map(|l| l.trim())
            .filter(|l| !l.is_empty())
            .collect();

        Ok(json!({
            "current": current_branch,
            "branches": branches,
            "count": branches.len(),
            "success": true
        }))
    }
}

/// Tool for git checkout (switch branches or restore files)
pub struct GitCheckoutTool {
    workspace_root: PathBuf,
}

impl GitCheckoutTool {
    pub fn new(workspace_root: PathBuf) -> Self {
        Self { workspace_root }
    }
}

#[async_trait]
impl Tool for GitCheckoutTool {
    fn name(&self) -> &str {
        "git_checkout"
    }

    fn description(&self) -> &str {
        "Switch branches or restore working tree files. Can create new branches with create=true."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "target": {
                    "type": "string",
                    "description": "Branch name, commit, or file path to checkout"
                },
                "create": {
                    "type": "boolean",
                    "description": "Create a new branch (git checkout -b)"
                },
                "files": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Specific files to restore (git checkout -- <files>)"
                }
            },
            "required": ["target"]
        })
    }

    fn thulp_definition(&self) -> thulp_core::ToolDefinition {
        use thulp_core::{Parameter, ParameterType};
        thulp_core::ToolDefinition::builder("git_checkout")
            .description(self.description())
            .parameter(Parameter::builder("target").param_type(ParameterType::String).required(true)
                .description("Branch name, commit, or file path to checkout").build())
            .parameter(Parameter::builder("create").param_type(ParameterType::Boolean).required(false)
                .description("Create a new branch (git checkout -b)").build())
            .parameter(Parameter::builder("files").param_type(ParameterType::Array).required(false)
                .description("Specific files to restore (git checkout -- <files>)").build())
            .build()
    }

    async fn execute(&self, args: Value) -> crate::Result<Value> {
        let target = args["target"]
            .as_str()
            .ok_or_else(|| crate::PawanError::Tool("target is required".into()))?;
        let create = args["create"].as_bool().unwrap_or(false);
        let files: Vec<&str> = args["files"]
            .as_array()
            .map(|a| a.iter().filter_map(|v| v.as_str()).collect())
            .unwrap_or_default();

        let mut git_args: Vec<&str> = vec!["checkout"];

        if create {
            git_args.push("-b");
        }

        git_args.push(target);

        if !files.is_empty() {
            git_args.push("--");
            git_args.extend(files.iter());
        }

        let (success, stdout, stderr) = run_git(&self.workspace_root, &git_args).await?;

        if !success {
            return Err(crate::PawanError::Git(format!(
                "git checkout failed: {}",
                stderr
            )));
        }

        Ok(json!({
            "success": true,
            "target": target,
            "created": create,
            "output": format!("{}{}", stdout, stderr).trim().to_string()
        }))
    }
}

/// Tool for git stash operations
pub struct GitStashTool {
    workspace_root: PathBuf,
}

impl GitStashTool {
    pub fn new(workspace_root: PathBuf) -> Self {
        Self { workspace_root }
    }
}

#[async_trait]
impl Tool for GitStashTool {
    fn name(&self) -> &str {
        "git_stash"
    }

    fn description(&self) -> &str {
        "Stash or restore uncommitted changes. Actions: push (default), pop, list, drop."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["push", "pop", "list", "drop", "show"],
                    "description": "Stash action (default: push)"
                },
                "message": {
                    "type": "string",
                    "description": "Message for stash push"
                },
                "index": {
                    "type": "integer",
                    "description": "Stash index for pop/drop/show (default: 0)"
                }
            },
            "required": []
        })
    }

    fn thulp_definition(&self) -> thulp_core::ToolDefinition {
        use thulp_core::{Parameter, ParameterType};
        thulp_core::ToolDefinition::builder("git_stash")
            .description(self.description())
            .parameter(Parameter::builder("action").param_type(ParameterType::String).required(false)
                .description("Stash action (default: push)").build())
            .parameter(Parameter::builder("message").param_type(ParameterType::String).required(false)
                .description("Message for stash push").build())
            .parameter(Parameter::builder("index").param_type(ParameterType::Integer).required(false)
                .description("Stash index for pop/drop/show (default: 0)").build())
            .build()
    }

    async fn execute(&self, args: Value) -> crate::Result<Value> {
        let action = args["action"].as_str().unwrap_or("push");
        let message = args["message"].as_str();
        let index = args["index"].as_u64().unwrap_or(0);

        let git_args: Vec<String> = match action {
            "push" => {
                let mut a = vec!["stash".to_string(), "push".to_string()];
                if let Some(msg) = message {
                    a.push("-m".to_string());
                    a.push(msg.to_string());
                }
                a
            }
            "pop" => vec!["stash".to_string(), "pop".to_string(), format!("stash@{{{}}}", index)],
            "list" => vec!["stash".to_string(), "list".to_string()],
            "drop" => vec!["stash".to_string(), "drop".to_string(), format!("stash@{{{}}}", index)],
            "show" => vec!["stash".to_string(), "show".to_string(), "-p".to_string(), format!("stash@{{{}}}", index)],
            _ => return Err(crate::PawanError::Tool(format!("Unknown stash action: {}", action))),
        };

        let git_args_ref: Vec<&str> = git_args.iter().map(|s| s.as_str()).collect();
        let (success, stdout, stderr) = run_git(&self.workspace_root, &git_args_ref).await?;

        if !success {
            return Err(crate::PawanError::Git(format!(
                "git stash {} failed: {}",
                action, stderr
            )));
        }

        Ok(json!({
            "success": true,
            "action": action,
            "output": stdout.trim().to_string()
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::git::staging::{GitAddTool, GitCommitTool};
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
    async fn test_git_branch_list() {
        let temp_dir = setup_git_repo().await;
        std::fs::write(temp_dir.path().join("f.txt"), "init").unwrap();
        Command::new("git").args(["add", "."]).current_dir(temp_dir.path()).output().await.unwrap();
        Command::new("git").args(["commit", "-m", "init"]).current_dir(temp_dir.path()).output().await.unwrap();

        let tool = GitBranchTool::new(temp_dir.path().into());
        let result = tool.execute(json!({})).await.unwrap();
        assert!(result["success"].as_bool().unwrap());
        let branches = result["branches"].as_array().unwrap();
        assert!(!branches.is_empty(), "Should have at least one branch");
        assert!(result["current"].as_str().is_some());
    }

    #[tokio::test]
    async fn test_git_checkout_create_branch() {
        let temp_dir = setup_git_repo().await;
        std::fs::write(temp_dir.path().join("f.txt"), "init").unwrap();
        Command::new("git").args(["add", "."]).current_dir(temp_dir.path()).output().await.unwrap();
        Command::new("git").args(["commit", "-m", "init"]).current_dir(temp_dir.path()).output().await.unwrap();

        let tool = GitCheckoutTool::new(temp_dir.path().into());
        let result = tool.execute(json!({"target": "feature-test", "create": true})).await.unwrap();
        assert!(result["success"].as_bool().unwrap());

        // Verify we're on the new branch
        let branch_tool = GitBranchTool::new(temp_dir.path().into());
        let branches = branch_tool.execute(json!({})).await.unwrap();
        assert_eq!(branches["current"].as_str().unwrap(), "feature-test");
    }

    #[tokio::test]
    async fn test_git_stash_on_clean_repo() {
        let temp_dir = setup_git_repo().await;
        let tool = GitStashTool::new(temp_dir.path().into());
        // List stashes on empty repo
        let result = tool.execute(json!({"action": "list"})).await.unwrap();
        assert!(result["success"].as_bool().unwrap());
    }

    #[tokio::test]
    async fn test_git_stash_on_dirty_repo_saves_changes() {
        let temp_dir = setup_git_repo().await;
        // First commit a base file
        std::fs::write(temp_dir.path().join("base.txt"), "v1").unwrap();
        GitAddTool::new(temp_dir.path().to_path_buf())
            .execute(json!({ "files": ["base.txt"] }))
            .await
            .unwrap();
        GitCommitTool::new(temp_dir.path().to_path_buf())
            .execute(json!({ "message": "base" }))
            .await
            .unwrap();

        // Now modify it so there's something to stash
        std::fs::write(temp_dir.path().join("base.txt"), "v2-dirty").unwrap();

        let stash_tool = GitStashTool::new(temp_dir.path().to_path_buf());
        let result = stash_tool
            .execute(json!({ "action": "push", "message": "test stash" }))
            .await
            .unwrap();
        assert!(result["success"].as_bool().unwrap());

        // Working tree should be clean again (stash popped the change)
        let content = std::fs::read_to_string(temp_dir.path().join("base.txt")).unwrap();
        assert_eq!(content, "v1", "stash push should revert working tree");
    }

    #[tokio::test]
    async fn test_git_checkout_nonexistent_branch_without_create_errors() {
        // Checkout to a non-existent branch WITHOUT create=true must fail,
        // not silently create it. This pins the "safety" contract of the tool.
        let temp_dir = setup_git_repo().await;
        std::fs::write(temp_dir.path().join("init.txt"), "init").unwrap();
        Command::new("git")
            .args(["add", "."])
            .current_dir(temp_dir.path())
            .output()
            .await
            .unwrap();
        Command::new("git")
            .args(["commit", "-m", "init"])
            .current_dir(temp_dir.path())
            .output()
            .await
            .unwrap();

        let tool = GitCheckoutTool::new(temp_dir.path().to_path_buf());
        let result = tool
            .execute(json!({
                "target": "nonexistent-branch-xyz-abc-9999",
                "create": false
            }))
            .await;
        assert!(
            result.is_err(),
            "checkout to nonexistent branch without create must error"
        );
    }
}
