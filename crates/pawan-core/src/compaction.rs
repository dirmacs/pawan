//! Structured conversation compaction for context overflow handling
//!
//! When conversation history exceeds context limits, this module provides
//! tools to compact the history while preserving key information like:
//! - User's original intent and requirements
//! - Important decisions made
//! - Code changes and their rationale
//! - Error messages and debugging information

use crate::agent::{Message, Role};
use serde::{Deserialize, Serialize};

/// Compaction strategy for preserving different types of information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompactionStrategy {
    /// Keep the most recent N messages (default: 10)
    pub keep_recent: usize,
    /// Keep messages with specific keywords (e.g., "error", "fix", "bug")
    pub keep_keywords: Vec<String>,
    /// Keep tool call results (default: true)
    pub keep_tool_results: bool,
    /// Keep system messages (default: true)
    pub keep_system: bool,
}

impl Default for CompactionStrategy {
    fn default() -> Self {
        Self {
            keep_recent: 10,
            keep_keywords: vec![
                "error".to_string(),
                "fix".to_string(),
                "bug".to_string(),
                "issue".to_string(),
                "problem".to_string(),
                "solution".to_string(),
                "important".to_string(),
                "note".to_string(),
                "warning".to_string(),
            ],
            keep_tool_results: true,
            keep_system: true,
        }
    }
}

/// Compaction result with statistics
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompactionResult {
    /// The compacted messages
    pub messages: Vec<Message>,
    /// Number of messages before compaction
    pub original_count: usize,
    /// Number of messages after compaction
    pub compacted_count: usize,
    /// Estimated tokens saved
    pub tokens_saved: usize,
}

/// Build a compaction prompt for the LLM
pub fn build_compaction_prompt(messages: &[Message], strategy: &CompactionStrategy) -> String {
    let mut prompt = String::from(
        r#"You are tasked with compacting a conversation history while preserving all essential information.

Your goal is to create a concise summary that captures:
1. The user's original intent and requirements
2. Important decisions made during the conversation
3. Code changes and their rationale
4. Error messages and debugging information
5. Any warnings or important notes

Format your response as a structured summary with clear sections.

--- ORIGINAL CONVERSATION ---
"#,
    );

    // Add messages to the prompt
    for (i, msg) in messages.iter().enumerate() {
        let role = match msg.role {
            Role::System => "SYSTEM",
            Role::User => "USER",
            Role::Assistant => "ASSISTANT",
            Role::Tool => "TOOL",
        };
        prompt.push_str(&format!("\n[{}]: {}\n", role, msg.content));

        // Add tool call information if present
        if !msg.tool_calls.is_empty() {
            prompt.push_str("  Tool calls:\n");
            for tc in &msg.tool_calls {
                prompt.push_str(&format!("    - {}: {}\n", tc.name, tc.arguments));
            }
        }

        // Add tool result if present
        if let Some(ref result) = msg.tool_result {
            prompt.push_str(&format!("  Tool result: {}\n", result.content));
        }
    }

    prompt.push_str(
        r#"
--- END ORIGINAL CONVERSATION ---

Please provide a compacted summary of this conversation that preserves all essential information.
"#,
    );

    prompt
}

/// Compact messages based on the given strategy
pub fn compact_messages(messages: Vec<Message>, strategy: &CompactionStrategy) -> CompactionResult {
    let original_count = messages.len();
    let mut compacted = Vec::new();

    // Always keep system messages if enabled
    if strategy.keep_system {
        compacted.extend(
            messages
                .iter()
                .filter(|m| m.role == Role::System)
                .cloned(),
        );
    }

    // Keep messages with keywords
    for msg in &messages {
        let content_lower = msg.content.to_lowercase();
        if strategy
            .keep_keywords
            .iter()
            .any(|kw| content_lower.contains(&kw.to_lowercase()))
        {
            if !compacted.iter().any(|m| m.content == msg.content) {
                compacted.push(msg.clone());
            }
        }
    }

    // Keep tool results if enabled
    if strategy.keep_tool_results {
        for msg in &messages {
            if msg.tool_result.is_some() && !msg.tool_calls.is_empty() {
                if !compacted.iter().any(|m| m.content == msg.content) {
                    compacted.push(msg.clone());
                }
            }
        }
    }

    // Keep the most recent messages
    let recent_start = if messages.len() > strategy.keep_recent {
        messages.len() - strategy.keep_recent
    } else {
        0
    };

    for msg in &messages[recent_start..] {
        if !compacted.iter().any(|m| m.content == msg.content) {
            compacted.push(msg.clone());
        }
    }

    // Sort by original order (approximate)
    compacted.sort_by_key(|m| {
        messages
            .iter()
            .position(|orig| orig.content == m.content)
            .unwrap_or(usize::MAX)
    });

    let compacted_count = compacted.len();
    let tokens_saved = estimate_tokens_saved(original_count, compacted_count);

    CompactionResult {
        messages: compacted,
        original_count,
        compacted_count,
        tokens_saved,
    }
}

/// Estimate tokens saved by compaction (rough approximation)
fn estimate_tokens_saved(original: usize, compacted: usize) -> usize {
    // Assume average of 4 tokens per message
    let avg_tokens_per_message = 4;
    (original - compacted) * avg_tokens_per_message
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_compaction_strategy_default() {
        let strategy = CompactionStrategy::default();
        assert_eq!(strategy.keep_recent, 10);
        assert!(strategy.keep_keywords.contains(&"error".to_string()));
        assert!(strategy.keep_tool_results);
        assert!(strategy.keep_system);
    }

    #[test]
    fn test_build_compaction_prompt() {
        let messages = vec![
            Message {
                role: Role::User,
                content: "Fix the bug in main.rs".to_string(),
                tool_calls: vec![],
                tool_result: None,
            },
            Message {
                role: Role::Assistant,
                content: "I'll read the file first.".to_string(),
                tool_calls: vec![],
                tool_result: None,
            },
        ];

        let prompt = build_compaction_prompt(&messages, &CompactionStrategy::default());
        assert!(prompt.contains("Fix the bug in main.rs"));
        assert!(prompt.contains("I'll read the file first."));
        assert!(prompt.contains("ORIGINAL CONVERSATION"));
    }

    #[test]
    fn test_compact_messages() {
        let messages = vec![
            Message {
                role: Role::System,
                content: "You are a coding agent.".to_string(),
                tool_calls: vec![],
                tool_result: None,
            },
            Message {
                role: Role::User,
                content: "Fix the error".to_string(),
                tool_calls: vec![],
                tool_result: None,
            },
            Message {
                role: Role::Assistant,
                content: "I'll help.".to_string(),
                tool_calls: vec![],
                tool_result: None,
            },
        ];

        let strategy = CompactionStrategy::default();
        let result = compact_messages(messages, &strategy);

        assert_eq!(result.original_count, 3);
        assert!(result.compacted_count > 0);
        assert!(result.tokens_saved > 0);
    }

    #[test]
    fn test_compaction_preserves_system_messages() {
        let messages = vec![
            Message {
                role: Role::System,
                content: "System prompt".to_string(),
                tool_calls: vec![],
                tool_result: None,
            },
            Message {
                role: Role::User,
                content: "User message".to_string(),
                tool_calls: vec![],
                tool_result: None,
            },
        ];

        let strategy = CompactionStrategy {
            keep_system: true,
            ..Default::default()
        };

        let result = compact_messages(messages, &strategy);
        assert!(result
            .messages
            .iter()
            .any(|m| m.role == Role::System && m.content == "System prompt"));
    }

    #[test]
    fn test_compaction_preserves_keyword_messages() {
        let messages = vec![
            Message {
                role: Role::User,
                content: "Fix the error".to_string(),
                tool_calls: vec![],
                tool_result: None,
            },
            Message {
                role: Role::User,
                content: "Regular message".to_string(),
                tool_calls: vec![],
                tool_result: None,
            },
        ];

        let strategy = CompactionStrategy {
            keep_keywords: vec!["error".to_string()],
            ..Default::default()
        };

        let result = compact_messages(messages, &strategy);
        assert!(result
            .messages
            .iter()
            .any(|m| m.content == "Fix the error"));
    }
}
