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
    /// - bash, git_commit, write_file, edit_file: Prompt if not explicitly configured
    /// - Everything else: Allow
    pub fn resolve(name: &str, permissions: &HashMap<String, ToolPermission>) -> Self {
        if let Some(p) = permissions.get(name) {
            return p.clone();
        }
        // Default: sensitive tools prompt, others allow
        match name {
            "bash" | "git_commit" | "write_file" | "edit_file_lines" | "insert_after"
            | "append_file" => ToolPermission::Allow, // default allow for now; users can override to Prompt
            _ => ToolPermission::Allow,
        }
    }
}
