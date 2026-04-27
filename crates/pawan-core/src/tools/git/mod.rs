//! Git operation tools
//!
//! Tools for git operations: status, diff, add, commit, log, blame, branch.

use std::path::PathBuf;
use std::process::Stdio;
use tokio::io::AsyncReadExt;
use tokio::process::Command;

pub mod branch;
pub mod diff;
pub mod log;
pub mod staging;
pub mod status;

pub use branch::{GitBranchTool, GitCheckoutTool, GitStashTool};
pub use diff::GitDiffTool;
pub use log::{GitBlameTool, GitLogTool};
pub use staging::{GitAddTool, GitCommitTool};
pub use status::GitStatusTool;

/// Run a git command in a workspace directory
pub(crate) async fn run_git(
    workspace: &PathBuf,
    args: &[&str],
) -> crate::Result<(bool, String, String)> {
    let mut cmd = Command::new("git");
    cmd.args(args)
        .current_dir(workspace)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .stdin(Stdio::null());

    let mut child = cmd.spawn().map_err(crate::PawanError::Io)?;

    let mut stdout = String::new();
    let mut stderr = String::new();

    if let Some(mut handle) = child.stdout.take() {
        handle.read_to_string(&mut stdout).await.ok();
    }
    if let Some(mut handle) = child.stderr.take() {
        handle.read_to_string(&mut stderr).await.ok();
    }

    let status = child.wait().await.map_err(crate::PawanError::Io)?;
    Ok((status.success(), stdout, stderr))
}


#[cfg(test)]
mod tests {
    use super::*;
    use super::super::Tool;
    use tempfile::TempDir;

    #[tokio::test]
    async fn test_git_tool_schemas() {
        let tmp = TempDir::new().unwrap();
        // Verify all git tools have correct names and non-empty schemas
        let tools: Vec<(&str, Box<dyn Tool>)> = vec![
            ("git_status", Box::new(status::GitStatusTool::new(tmp.path().into()))),
            ("git_diff", Box::new(diff::GitDiffTool::new(tmp.path().into()))),
            ("git_add", Box::new(staging::GitAddTool::new(tmp.path().into()))),
            ("git_commit", Box::new(staging::GitCommitTool::new(tmp.path().into()))),
            ("git_log", Box::new(log::GitLogTool::new(tmp.path().into()))),
            ("git_blame", Box::new(log::GitBlameTool::new(tmp.path().into()))),
            ("git_branch", Box::new(branch::GitBranchTool::new(tmp.path().into()))),
            ("git_checkout", Box::new(branch::GitCheckoutTool::new(tmp.path().into()))),
            ("git_stash", Box::new(branch::GitStashTool::new(tmp.path().into()))),
        ];
        for (expected_name, tool) in &tools {
            assert_eq!(tool.name(), *expected_name, "Tool name mismatch");
            assert!(!tool.description().is_empty(), "Missing description for {}", expected_name);
            let schema = tool.parameters_schema();
            assert!(schema.is_object(), "Schema should be object for {}", expected_name);
        }
    }
}