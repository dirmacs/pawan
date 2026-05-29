//! Agent session and conversation history management.
//!
//! Re-exports persistence types from [`session_store`] so `crate::agent::session::*`
//! remains stable.

pub use super::session_store::*;

use super::{
    fence_external_system_messages_for_resume, Message, PawanAgent, Role,
};
use crate::config::PawanConfig;
use crate::tools::ToolDefinition;
use crate::Result;

impl PawanAgent {
    pub fn history(&self) -> &[Message] {
        &self.history
    }

    /// Save current conversation as a session, returns session ID
    pub fn save_session(&self) -> Result<String> {
        let mut session = Session::new(&self.config.model);
        session.messages = self.history.clone();
        session.total_tokens = self.context_tokens_estimate as u64;
        session.save()?;
        Ok(session.id)
    }

    /// Resume a saved session by ID
    pub fn resume_session(&mut self, session_id: &str) -> Result<()> {
        let session = Session::load(session_id)?;
        self.history = session.messages;
        self.context_tokens_estimate = session.total_tokens as usize;
        // Adopt the loaded session's id so eruka writes cluster under the
        // same key as the on-disk session.
        self.session_id = session_id.to_string();
        fence_external_system_messages_for_resume(&mut self.history);
        Ok(())
    }

    /// Archive the current conversation to Eruka's context store. Safe to
    /// call from any async context; returns Ok even when eruka is disabled
    /// or unreachable so callers can fire-and-forget after save_session().
    pub async fn archive_to_eruka(&self) -> Result<()> {
        let Some(eruka) = &self.eruka else {
            return Ok(());
        };
        let mut session = Session::new(&self.config.model);
        session.id = self.session_id.clone();
        session.messages = self.history.clone();
        session.total_tokens = self.context_tokens_estimate as u64;
        eruka.archive_session(&session).await
    }

    /// Build a compact snapshot of the current history for on_pre_compress.
    /// Keeps message role + first 200 chars per entry so the eruka write
    /// stays bounded even with huge histories.
    pub(crate) fn history_snapshot_for_eruka(history: &[Message]) -> String {
        let mut out = String::with_capacity(2048);
        for msg in history {
            let prefix = match msg.role {
                Role::User => "U: ",
                Role::Assistant => "A: ",
                Role::Tool => "T: ",
                Role::System => "S: ",
            };
            let body: String = msg.content.chars().take(200).collect();
            out.push_str(prefix);
            out.push_str(&body);
            out.push('\n');
            if out.len() > 4000 {
                break;
            }
        }
        out
    }

    /// Get the configuration
    pub fn config(&self) -> &PawanConfig {
        &self.config
    }

    /// Clear the conversation history
    pub fn clear_history(&mut self) {
        self.history.clear();
    }

    /// Prune conversation history to reduce context size.
    /// Uses importance scoring (inspired by claude-code-rust's consolidation engine):
    /// - Tool results with errors: high importance (learning from failures)
    /// - User messages: medium importance (intent context)
    /// - Successful tool results: low importance (can be re-derived)
    ///
    /// Keeps system prompt + last 4 messages, summarizes the rest.
    pub(crate) fn prune_history(&mut self) {
        let len = self.history.len();
        if len <= 5 {
            return; // Nothing to prune
        }

        let keep_end = 4;
        let start = 1; // Skip system prompt at index 0
        let end = len - keep_end;
        let pruned_count = end - start;

        // Score messages by importance for summary prioritization
        let mut scored: Vec<(f32, &Message)> = self.history[start..end]
            .iter()
            .map(|msg| {
                let score = Self::message_importance(msg);
                (score, msg)
            })
            .collect();
        scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));

        // Build summary from highest-importance messages first (UTF-8 safe)
        let mut summary = String::with_capacity(2048);
        for (score, msg) in &scored {
            let prefix = match msg.role {
                Role::User => "User: ",
                Role::Assistant => "Assistant: ",
                Role::Tool => {
                    if *score > 0.7 {
                        "Tool error: "
                    } else {
                        "Tool: "
                    }
                }
                Role::System => "System: ",
            };
            let chunk: String = msg.content.chars().take(200).collect();
            summary.push_str(prefix);
            summary.push_str(&chunk);
            summary.push('\n');
            if summary.len() > 2000 {
                let safe_end = summary
                    .char_indices()
                    .take_while(|(i, _)| *i <= 2000)
                    .last()
                    .map(|(i, c)| i + c.len_utf8())
                    .unwrap_or(0);
                summary.truncate(safe_end);
                break;
            }
        }

        let summary_msg = Message {
            role: Role::System,
            content: format!(
                "Previous conversation summary (pruned {} messages, importance-ranked): {}",
                pruned_count, summary
            ),
            tool_calls: vec![],
            tool_result: None,
        };

        self.history.drain(start..end);
        self.history.insert(start, summary_msg);

        tracing::info!(
            pruned = pruned_count,
            context_estimate = self.context_tokens_estimate,
            "Pruned messages from history (importance-ranked)"
        );
    }

    /// Score a message's importance for pruning decisions (0.0-1.0).
    /// Higher = more important = kept in summary.
    pub(crate) fn message_importance(msg: &Message) -> f32 {
        match msg.role {
            Role::User => 0.6,   // User intent is moderately important
            Role::System => 0.3, // System messages are usually ephemeral
            Role::Assistant => {
                if msg.content.contains("error") || msg.content.contains("Error") {
                    0.8
                } else {
                    0.4
                }
            }
            Role::Tool => {
                if let Some(ref result) = msg.tool_result {
                    if !result.success {
                        0.9
                    }
                    // Failed tools are very important (learning)
                    else {
                        0.2
                    } // Successful tools can be re-derived
                } else {
                    0.3
                }
            }
        }
    }

    /// Add a message to history
    pub fn add_message(&mut self, message: Message) {
        self.history.push(message);
    }

    /// Switch the LLM model at runtime. Recreates the backend with the new model.
    pub fn switch_model(&mut self, model: &str) -> Result<()> {
        self.config.model = model.to_string();
        let system_prompt = self.config.get_system_prompt_checked()?;
        self.backend = Self::create_backend(&self.config, &system_prompt);
        tracing::info!(model = model, "Model switched at runtime");
        Ok(())
    }

    /// Get the current model name
    pub fn model_name(&self) -> &str {
        &self.config.model
    }

    /// Stable session id for this agent (Eruka sync, persistence keys)
    pub fn session_id(&self) -> &str {
        &self.session_id
    }

    /// Get tool definitions for the LLM
    pub fn get_tool_definitions(&self) -> Vec<ToolDefinition> {
        self.tools.get_definitions()
    }
}
