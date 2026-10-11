use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Permission level for a tool
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum ToolPermission {
    /// Always allow (default for most tools)
    Allow,
    /// Deny — tool is disabled
    Deny,
    /// Prompt — ask user before executing (TUI shows confirmation, headless denies)
    Prompt,
}

impl ToolPermission {
    /// Resolve permission for a tool name.
    /// Checks explicit config first, then falls back to default rules:
    /// - state-mutating tools (shell, file writes, git writes, structural
    ///   rewrite, index writers, MCP bridges' server-side effects): Prompt
    /// - Everything else: Allow
    ///
    /// The default for a mutating tool is Prompt, not Allow: with a
    /// callback wired the user is asked; with none wired the mutating-tool
    /// gate denies (fail closed) rather than auto-approving.
    pub fn resolve(name: &str, permissions: &HashMap<String, ToolPermission>) -> Self {
        if let Some(p) = permissions.get(name) {
            return p.clone();
        }
        // Default: mutating tools prompt, others allow
        match name {
            "bash" | "write_file" | "edit_file" | "edit_file_lines" | "insert_after"
            | "append_file" | "git_add" | "git_commit" | "git_checkout" | "git_stash"
            | "git_branch" | "ast_grep" | "sd" | "deagle_map" | "mise" => ToolPermission::Prompt,
            _ => ToolPermission::Allow,
        }
    }
}
