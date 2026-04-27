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

/// Build a structured compaction prompt for the LLM
///
/// This prompt instructs the LLM to create a structured summary that preserves
/// essential information while reducing token count. The output format is
/// designed to be machine-readable and easily parsed.
pub fn build_compaction_prompt(messages: &[Message], _strategy: &CompactionStrategy) -> String {
    let mut prompt = String::from(
        r#"# Structured Conversation Compaction

You are tasked with compacting a conversation history while preserving all essential information.

## Your Goal

Create a concise, structured summary that captures:
1. **User's Original Intent** - What the user wanted to accomplish
2. **Important Decisions** - Key decisions made during the conversation
3. **Code Changes** - What was changed and why
4. **Error Messages** - Any errors encountered and their solutions
5. **Debugging Information** - Important debugging steps and findings
6. **Warnings and Notes** - Any warnings or important notes

## Output Format

Your response MUST follow this exact structure:

```
# Conversation Summary

## User Intent
[Describe what the user wanted to accomplish in 1-2 sentences]

## Key Decisions
- [Decision 1]
- [Decision 2]
- [Decision 3]

## Code Changes
### File: [filename]
- **Change**: [description of change]
- **Rationale**: [why this change was made]
- **Impact**: [what this affects]

### File: [filename]
- **Change**: [description of change]
- **Rationale**: [why this change was made]
- **Impact**: [what this affects]

## Errors and Solutions
### Error: [error description]
- **Location**: [where the error occurred]
- **Solution**: [how it was fixed]
- **Prevention**: [how to prevent this in the future]

## Debugging Steps
1. [Step 1]
2. [Step 2]
3. [Step 3]

## Warnings and Notes
- [Warning or note 1]
- [Warning or note 2]

## Current State
[Describe the current state of the work in 1-2 sentences]

## Next Steps
1. [Next step 1]
2. [Next step 2]
3. [Next step 3]
```

## Guidelines

- Be concise but complete
- Preserve all technical details (function names, file paths, error messages)
- Use bullet points for lists
- Keep each section focused and clear
- If a section has no relevant information, write "None"
- Maintain chronological order where relevant
- Include specific values (numbers, strings, paths) when important

## Original Conversation

"#,
    );

    // Add messages to the prompt with clear section markers
    for (i, msg) in messages.iter().enumerate() {
        let role = match msg.role {
            Role::System => "SYSTEM",
            Role::User => "USER",
            Role::Assistant => "ASSISTANT",
            Role::Tool => "TOOL",
        };
        prompt.push_str(&format!(
            "\n### Message {} [{}]\n\n{}\n",
            i + 1,
            role,
            msg.content
        ));

        // Add tool call information if present
        if !msg.tool_calls.is_empty() {
            prompt.push_str("\n**Tool Calls:**\n");
            for tc in &msg.tool_calls {
                prompt.push_str(&format!("- `{}`: {}\n", tc.name, tc.arguments));
            }
        }

        // Add tool result if present
        if let Some(ref result) = msg.tool_result {
            prompt.push_str(&format!("\n**Tool Result:**\n{}\n", result.content));
        }
    }

    prompt.push_str(
        r#"

--- End of Original Conversation ---

Please provide a structured summary following the exact format specified above.
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
        compacted.extend(messages.iter().filter(|m| m.role == Role::System).cloned());
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

/// Parse a structured compaction summary
///
/// This function parses the structured output from the LLM and extracts
/// the different sections into a structured format.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParsedCompactionSummary {
    /// User's original intent
    pub user_intent: String,
    /// Key decisions made
    pub key_decisions: Vec<String>,
    /// Code changes made
    pub code_changes: Vec<CodeChange>,
    /// Errors encountered and their solutions
    pub errors_and_solutions: Vec<ErrorSolution>,
    /// Debugging steps taken
    pub debugging_steps: Vec<String>,
    /// Warnings and notes
    pub warnings_and_notes: Vec<String>,
    /// Current state of the work
    pub current_state: String,
    /// Next steps to take
    pub next_steps: Vec<String>,
}

/// A code change with metadata
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CodeChange {
    /// File that was changed
    pub file: String,
    /// Description of the change
    pub change: String,
    /// Rationale for the change
    pub rationale: String,
    /// Impact of the change
    pub impact: String,
}

/// An error and its solution
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ErrorSolution {
    /// Error description
    pub error: String,
    /// Location of the error
    pub location: String,
    /// How it was fixed
    pub solution: String,
    /// How to prevent this in the future
    pub prevention: String,
}

/// Parse a structured compaction summary from LLM output
///
/// This is a simple parser that extracts sections from the structured format.
/// It's designed to be robust to minor variations in formatting.
pub fn parse_compaction_summary(summary: &str) -> Result<ParsedCompactionSummary, String> {
    let mut parsed = ParsedCompactionSummary {
        user_intent: String::new(),
        key_decisions: Vec::new(),
        code_changes: Vec::new(),
        errors_and_solutions: Vec::new(),
        debugging_steps: Vec::new(),
        warnings_and_notes: Vec::new(),
        current_state: String::new(),
        next_steps: Vec::new(),
    };

    let lines: Vec<&str> = summary.lines().collect();
    let mut current_section: Option<String> = None;
    let mut current_code_change: Option<CodeChange> = None;
    let mut current_error: Option<ErrorSolution> = None;

    for line in lines {
        let line = line.trim();

        // Detect section headers
        if line.starts_with("## ") {
            current_section = Some(line[3..].to_string());
            continue;
        }

        // Detect subsection headers (###)
        if line.starts_with("### ") {
            let subsection = line[4..].to_string();

            // If we were building a code change, save it
            if let Some(code_change) = current_code_change.take() {
                if !code_change.file.is_empty() {
                    parsed.code_changes.push(code_change);
                }
            }

            // If we were building an error solution, save it
            if let Some(error) = current_error.take() {
                if !error.error.is_empty() {
                    parsed.errors_and_solutions.push(error);
                }
            }

            // Start new code change or error
            if subsection.starts_with("File: ") {
                current_code_change = Some(CodeChange {
                    file: subsection[6..].to_string(),
                    change: String::new(),
                    rationale: String::new(),
                    impact: String::new(),
                });
            } else if subsection.starts_with("Error: ") {
                current_error = Some(ErrorSolution {
                    error: subsection[7..].to_string(),
                    location: String::new(),
                    solution: String::new(),
                    prevention: String::new(),
                });
            }

            continue;
        }

        // Process content based on current section
        match current_section.as_deref() {
            Some("User Intent") => {
                parsed.user_intent.push_str(line);
                parsed.user_intent.push(' ');
            }
            Some("Key Decisions") => {
                if line.starts_with("- ") {
                    parsed.key_decisions.push(line[2..].to_string());
                }
            }
            Some("Code Changes") => {
                if let Some(ref mut code_change) = current_code_change {
                    if line.starts_with("- **Change**: ") {
                        code_change.change = line[12..].to_string();
                    } else if line.starts_with("- **Rationale**: ") {
                        code_change.rationale = line[14..].to_string();
                    } else if line.starts_with("- **Impact**: ") {
                        code_change.impact = line[11..].to_string();
                    }
                }
            }
            Some("Errors and Solutions") => {
                if let Some(ref mut error) = current_error {
                    if line.starts_with("- **Location**: ") {
                        error.location = line[14..].to_string();
                    } else if line.starts_with("- **Solution**: ") {
                        error.solution = line[14..].to_string();
                    } else if line.starts_with("- **Prevention**: ") {
                        error.prevention = line[15..].to_string();
                    }
                }
            }
            Some("Debugging Steps") => {
                if line.starts_with("1. ") || line.starts_with("2. ") || line.starts_with("3. ") {
                    parsed.debugging_steps.push(line[3..].to_string());
                }
            }
            Some("Warnings and Notes") => {
                if line.starts_with("- ") {
                    parsed.warnings_and_notes.push(line[2..].to_string());
                }
            }
            Some("Current State") => {
                parsed.current_state.push_str(line);
                parsed.current_state.push(' ');
            }
            Some("Next Steps") => {
                if line.starts_with("1. ") || line.starts_with("2. ") || line.starts_with("3. ") {
                    parsed.next_steps.push(line[3..].to_string());
                }
            }
            _ => {}
        }
    }

    // Save any remaining code change or error
    if let Some(code_change) = current_code_change {
        if !code_change.file.is_empty() {
            parsed.code_changes.push(code_change);
        }
    }
    if let Some(error) = current_error {
        if !error.error.is_empty() {
            parsed.errors_and_solutions.push(error);
        }
    }

    // Trim whitespace
    parsed.user_intent = parsed.user_intent.trim().to_string();
    parsed.current_state = parsed.current_state.trim().to_string();

    Ok(parsed)
}

/// Convert a parsed compaction summary back to a message
///
/// This function converts the structured summary back into a message
/// that can be added to the conversation history.
pub fn summary_to_message(summary: &ParsedCompactionSummary) -> Message {
    let mut content = String::from("# Conversation Summary\n\n");

    content.push_str("## User Intent\n");
    content.push_str(&summary.user_intent);
    content.push_str("\n\n");

    if !summary.key_decisions.is_empty() {
        content.push_str("## Key Decisions\n");
        for decision in &summary.key_decisions {
            content.push_str("- ");
            content.push_str(decision);
            content.push_str("\n");
        }
        content.push_str("\n");
    }

    if !summary.code_changes.is_empty() {
        content.push_str("## Code Changes\n");
        for change in &summary.code_changes {
            content.push_str(&format!("### File: {}\n", change.file));
            content.push_str(&format!("- **Change**: {}\n", change.change));
            content.push_str(&format!("- **Rationale**: {}\n", change.rationale));
            content.push_str(&format!("- **Impact**: {}\n", change.impact));
            content.push_str("\n");
        }
    }

    if !summary.errors_and_solutions.is_empty() {
        content.push_str("## Errors and Solutions\n");
        for error in &summary.errors_and_solutions {
            content.push_str(&format!("### Error: {}\n", error.error));
            content.push_str(&format!("- **Location**: {}\n", error.location));
            content.push_str(&format!("- **Solution**: {}\n", error.solution));
            content.push_str(&format!("- **Prevention**: {}\n", error.prevention));
            content.push_str("\n");
        }
    }

    if !summary.debugging_steps.is_empty() {
        content.push_str("## Debugging Steps\n");
        for (i, step) in summary.debugging_steps.iter().enumerate() {
            content.push_str(&format!("{}. {}\n", i + 1, step));
        }
        content.push_str("\n");
    }

    if !summary.warnings_and_notes.is_empty() {
        content.push_str("## Warnings and Notes\n");
        for warning in &summary.warnings_and_notes {
            content.push_str("- ");
            content.push_str(warning);
            content.push_str("\n");
        }
        content.push_str("\n");
    }

    content.push_str("## Current State\n");
    content.push_str(&summary.current_state);
    content.push_str("\n\n");

    if !summary.next_steps.is_empty() {
        content.push_str("## Next Steps\n");
        for (i, step) in summary.next_steps.iter().enumerate() {
            content.push_str(&format!("{}. {}\n", i + 1, step));
        }
    }

    Message {
        role: Role::System,
        content,
        tool_calls: vec![],
        tool_result: None,
    }
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
        assert!(prompt.contains("Original Conversation"));
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

        // Use a strategy that will actually drop some messages
        let strategy = CompactionStrategy {
            keep_recent: 1,
            keep_keywords: vec![],
            keep_tool_results: false,
            keep_system: false,
        };
        let result = compact_messages(messages, &strategy);

        assert_eq!(result.original_count, 3);
        // With keep_recent=1, only the most recent message (Assistant) is kept
        // System and User are dropped because keep_system=false and keep_keywords=[]
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
        assert!(result.messages.iter().any(|m| m.content == "Fix the error"));
    }
}
