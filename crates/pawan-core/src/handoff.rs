//! Handoff prompt generation for session context transfer
//!
//! This module provides functionality to generate focused handoff prompts
//! that preserve essential context while stripping noise from conversations.

use crate::agent::{Message, Role};
use std::collections::HashSet;

/// Configuration for handoff prompt generation
#[derive(Debug, Clone)]
pub struct HandoffConfig {
    /// Maximum number of constraints to include
    pub max_constraints: usize,
    /// Maximum number of tasks to include
    pub max_tasks: usize,
    /// Maximum number of recent messages to include
    pub max_recent_messages: usize,
    /// Maximum length for message previews
    pub max_preview_length: usize,
}

impl Default for HandoffConfig {
    fn default() -> Self {
        Self {
            max_constraints: 10,
            max_tasks: 15,
            max_recent_messages: 3,
            max_preview_length: 200,
        }
    }
}

/// Extracted context from a session for handoff
#[derive(Debug, Clone)]
pub struct HandoffContext {
    /// File paths referenced in the conversation
    pub file_paths: Vec<String>,
    /// Constraints and requirements
    pub constraints: Vec<String>,
    /// Key tasks and action items
    pub tasks: Vec<String>,
    /// Recent messages for context
    pub recent_messages: Vec<(Role, String)>,
    /// Session tags
    pub tags: Vec<String>,
    /// Session notes
    pub notes: String,
}

/// Session metadata for handoff
#[derive(Debug, Clone)]
pub struct HandoffMetadata {
    /// Model used
    pub model: String,
    /// Total message count
    pub message_count: usize,
    /// Tool calls made
    pub tool_calls: usize,
    /// Files edited
    pub files_edited: usize,
}

/// Generate a handoff prompt from conversation messages
///
/// This function extracts key information from messages and generates
/// a structured handoff prompt that preserves essential context while
/// removing noise and redundant information.
///
/// # Arguments
///
/// * `messages` - The conversation messages
/// * `model` - The model name used
/// * `tool_calls` - Number of tool calls made
/// * `files_edited` - Number of files edited
/// * `tags` - Session tags
/// * `notes` - Session notes
/// * `config` - Optional configuration for handoff generation
///
/// # Returns
///
/// A structured handoff prompt string
pub fn generate_handoff_prompt(
    messages: &[Message],
    model: &str,
    tool_calls: usize,
    files_edited: usize,
    tags: &[String],
    notes: &str,
    config: Option<HandoffConfig>,
) -> String {
    let config = config.unwrap_or_default();

    if messages.is_empty() {
        return "No conversation context available.".to_string();
    }

    let context = extract_context(messages, &config);
    let metadata = HandoffMetadata {
        model: model.to_string(),
        message_count: messages.len(),
        tool_calls,
        files_edited,
    };

    build_handoff_prompt(&context, &metadata, tags, notes, &config)
}

/// Extract key context from messages
fn extract_context(messages: &[Message], config: &HandoffConfig) -> HandoffContext {
    let mut file_paths: HashSet<String> = HashSet::new();
    let mut constraints = Vec::new();
    let mut tasks = Vec::new();
    let mut seen_messages: HashSet<String> = HashSet::new();

    for msg in messages {
        let content = &msg.content;

        // Skip duplicate messages (noise reduction)
        let content_hash = format!("{:?}:{}", msg.role, content);
        if seen_messages.contains(&content_hash) {
            continue;
        }
        seen_messages.insert(content_hash);

        // Extract file paths
        extract_file_paths(content, &mut file_paths);

        // Extract constraints
        extract_constraints(content, &mut constraints);

        // Extract tasks
        extract_tasks(content, &mut tasks);
    }

    // Collect recent messages (in reverse, then reverse back)
    let recent_messages: Vec<_> = messages
        .iter()
        .rev()
        .take(config.max_recent_messages)
        .filter_map(|msg| {
            let content = &msg.content;
            if content.is_empty() {
                return None;
            }
            let preview = if content.len() > config.max_preview_length {
                format!("{}...", &content[..config.max_preview_length])
            } else {
                content.clone()
            };
            Some((msg.role.clone(), preview))
        })
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();

    HandoffContext {
        file_paths: file_paths.into_iter().collect(),
        constraints,
        tasks,
        recent_messages,
        tags: Vec::new(),
        notes: String::new(),
    }
}
/// Extract file paths from content
fn extract_file_paths(content: &str, file_paths: &mut HashSet<String>) {
    for line in content.lines() {
        // Match file paths with common extensions
        let extensions = [".rs", ".ts", ".js", ".py", ".go", ".java", ".md", ".toml", ".json"];
        
        for word in line.split_whitespace() {
            let word = word.trim_matches(['\"', '\'', '(', ')', ',', ':', '[', ']']);
            
            // Check if word ends with a known extension
            if extensions.iter().any(|ext| word.ends_with(ext)) {
                file_paths.insert(word.to_string());
                continue;
            }

            // Check for path-like patterns (src/, lib/, test/, etc.)
            if word.contains('/') && (word.contains("src") || word.contains("lib") || word.contains("test")) {
                file_paths.insert(word.to_string());
            }
        }
    }
}

/// Extract constraints from content
fn extract_constraints(content: &str, constraints: &mut Vec<String>) {
    for line in content.lines() {
        let line = line.trim();
        
        // Look for constraint keywords
        let is_constraint = line.contains("MUST") 
            || line.contains("MUST NOT")
            || line.contains("SHOULD")
            || line.contains("SHOULD NOT")
            || line.contains("REQUIRED")
            || line.contains("constraint")
            || line.contains("requirement");

        if is_constraint && !line.is_empty() {
            constraints.push(line.to_string());
        }
    }
}

/// Extract tasks from content
fn extract_tasks(content: &str, tasks: &mut Vec<String>) {
    for line in content.lines() {
        let line = line.trim();
        
        // Look for task indicators
        let is_task = line.starts_with("-")
            || line.starts_with("*")
            || line.starts_with("+")
            || line.contains("TODO")
            || line.contains("FIXME")
            || line.contains("implement")
            || line.contains("fix")
            || line.contains("add")
            || line.contains("create")
            || line.contains("update")
            || line.contains("remove")
            || line.contains("delete");

        if is_task && !line.is_empty() {
            tasks.push(line.to_string());
        }
    }
}

/// Build the handoff prompt from extracted context
fn build_handoff_prompt(
    context: &HandoffContext,
    metadata: &HandoffMetadata,
    tags: &[String],
    notes: &str,
    config: &HandoffConfig,
) -> String {
    let mut parts = Vec::new();

    // Header
    parts.push("# Session Handoff".to_string());
    parts.push(String::new());

    // Metadata
    parts.push("## Session Summary".to_string());
    parts.push(format!("- **Model:** {}", metadata.model));
    parts.push(format!("- **Messages:** {}", metadata.message_count));
    parts.push(format!("- **Tool calls:** {}", metadata.tool_calls));
    parts.push(format!("- **Files edited:** {}", metadata.files_edited));
    parts.push(String::new());

    // Tags
    if !tags.is_empty() {
        parts.push("## Tags".to_string());
        for tag in tags {
            parts.push(format!("- {}", tag));
        }
        parts.push(String::new());
    }

    // Notes
    if !notes.is_empty() {
        parts.push("## Notes".to_string());
        parts.push(notes.to_string());
        parts.push(String::new());
    }

    // File paths
    if !context.file_paths.is_empty() {
        parts.push("## Files Referenced".to_string());
        let mut paths = context.file_paths.clone();
        paths.sort();
        paths.dedup();
        for path in paths {
            parts.push(format!("- {}", path));
        }
        parts.push(String::new());
    }

    // Constraints
    if !context.constraints.is_empty() {
        parts.push("## Constraints & Requirements".to_string());
        for constraint in context.constraints.iter().take(config.max_constraints) {
            parts.push(format!("- {}", constraint));
        }
        if context.constraints.len() > config.max_constraints {
            parts.push(format!("- ... and {} more", context.constraints.len() - config.max_constraints));
        }
        parts.push(String::new());
    }

    // Tasks
    if !context.tasks.is_empty() {
        parts.push("## Key Tasks & Action Items".to_string());
        for task in context.tasks.iter().take(config.max_tasks) {
            // Remove leading dash/star/plus if present to avoid double formatting
            let task = task.trim_start_matches(['-', '*', '+']).trim();
            parts.push(format!("- {}", task));
        }
        if context.tasks.len() > config.max_tasks {
            parts.push(format!("- ... and {} more", context.tasks.len() - config.max_tasks));
        }
        parts.push(String::new());
    }

    // Recent context
    if !context.recent_messages.is_empty() {
        parts.push("## Recent Context".to_string());
        for (role, content) in &context.recent_messages {
            let role_name = match role {
                Role::User => "User",
                Role::Assistant => "Assistant",
                Role::System => "System",
                Role::Tool => "Tool",
            };
            parts.push(format!("**{}:** {}", role_name, content));
        }
        parts.push(String::new());
    }

    // Footer
    parts.push("---".to_string());
    parts.push("*Handoff generated for context transfer*".to_string());

    parts.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_generate_handoff_prompt_empty() {
        let messages = vec![];
        let prompt = generate_handoff_prompt(&messages, "test-model", 0, 0, &[], "", None);
        assert!(prompt.contains("No conversation context available"));
    }

    #[test]
    fn test_generate_handoff_prompt_with_content() {
        let messages = vec![
            Message {
                role: Role::User,
                content: "Fix src/main.rs".to_string(),
                tool_calls: vec![],
                tool_result: None,
            },
            Message {
                role: Role::Assistant,
                content: "I'll fix it".to_string(),
                tool_calls: vec![],
                tool_result: None,
            },
        ];
        
        let prompt = generate_handoff_prompt(&messages, "test-model", 3, 1, &[], "", None);
        
        assert!(prompt.contains("Session Handoff"));
        assert!(prompt.contains("Model:"));
        assert!(prompt.contains("Messages:"));
        assert!(prompt.contains("Tool calls:"));
        assert!(prompt.contains("Files edited:"));
    }

    #[test]
    fn test_extract_file_paths() {
        let messages = vec![
            Message {
                role: Role::User,
                content: "Edit src/main.rs and lib/helper.ts".to_string(),
                tool_calls: vec![],
                tool_result: None,
            },
        ];
        
        let prompt = generate_handoff_prompt(&messages, "test-model", 0, 0, &[], "", None);
        
        assert!(prompt.contains("Files Referenced"));
        assert!(prompt.contains("src/main.rs"));
        assert!(prompt.contains("lib/helper.ts"));
    }

    #[test]
    fn test_extract_constraints() {
        let messages = vec![
            Message {
                role: Role::User,
                content: "MUST use async functions\nMUST NOT break existing tests".to_string(),
                tool_calls: vec![],
                tool_result: None,
            },
        ];
        
        let prompt = generate_handoff_prompt(&messages, "test-model", 0, 0, &[], "", None);
        
        assert!(prompt.contains("Constraints"));
        assert!(prompt.contains("MUST"));
    }

    #[test]
    fn test_extract_tasks() {
        let messages = vec![
            Message {
                role: Role::User,
                content: "- Implement feature X\n- Fix bug Y\n* Add tests".to_string(),
                tool_calls: vec![],
                tool_result: None,
            },
        ];
        
        let prompt = generate_handoff_prompt(&messages, "test-model", 0, 0, &[], "", None);
        
        assert!(prompt.contains("Key Tasks"));
        assert!(prompt.contains("Implement feature X") || prompt.contains("feature X"));
    }

    #[test]
    fn test_recent_context() {
        let messages = vec![
            Message {
                role: Role::User,
                content: "First message".to_string(),
                tool_calls: vec![],
                tool_result: None,
            },
            Message {
                role: Role::Assistant,
                content: "First response".to_string(),
                tool_calls: vec![],
                tool_result: None,
            },
            Message {
                role: Role::User,
                content: "Second message".to_string(),
                tool_calls: vec![],
                tool_result: None,
            },
            Message {
                role: Role::Assistant,
                content: "Second response".to_string(),
                tool_calls: vec![],
                tool_result: None,
            },
        ];
        
        let prompt = generate_handoff_prompt(&messages, "test-model", 0, 0, &[], "", None);
        
        assert!(prompt.contains("Recent Context"));
        assert!(prompt.contains("User") || prompt.contains("Assistant"));
    }

    #[test]
    fn test_deduplication() {
        let messages = vec![
            Message {
                role: Role::User,
                content: "Fix the bug".to_string(),
                tool_calls: vec![],
                tool_result: None,
            },
            Message {
                role: Role::User,
                content: "Fix the bug".to_string(), // Duplicate
                tool_calls: vec![],
                tool_result: None,
            },
        ];
        
        let prompt = generate_handoff_prompt(&messages, "test-model", 0, 0, &[], "", None);
        
        // Should show total message count (2), not deduplicated count
        assert!(prompt.contains("**Messages:** 2"));
    }

    #[test]
    fn test_custom_config() {
        let messages = vec![
            Message {
                role: Role::User,
                content: "- Task 1\n- Task 2\n- Task 3\n- Task 4\n- Task 5".to_string(),
                tool_calls: vec![],
                tool_result: None,
            },
        ];
        
        let config = HandoffConfig {
            max_tasks: 2,
            ..Default::default()
        };
        
        let prompt = generate_handoff_prompt(&messages, "test-model", 0, 0, &[], "", Some(config));
        
        assert!(prompt.contains("Key Tasks"));
        // Should limit to 2 tasks
        assert!(prompt.contains("- Task 1"));
        assert!(prompt.contains("- Task 2"));
        // Task 3 should not be in the output (limited to 2)
        assert!(!prompt.contains("- Task 3"));
    }
}
