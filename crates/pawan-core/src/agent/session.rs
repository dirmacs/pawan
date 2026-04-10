//! Session persistence — save and resume conversations

use crate::agent::Message;
use crate::{PawanError, Result};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// A saved conversation session
#[derive(Debug, Serialize, Deserialize)]
pub struct Session {
    /// Unique session ID
    pub id: String,
    /// Model used for this session
    pub model: String,
    /// When the session was created
    pub created_at: String,
    /// When the session was last updated
    pub updated_at: String,
    /// Conversation messages
    pub messages: Vec<Message>,
    /// Total tokens used in this session
    #[serde(default)]
    pub total_tokens: u64,
    /// Number of iterations completed
    #[serde(default)]
    pub iteration_count: u32,
}

impl Session {
    /// Create a new session
    pub fn new(model: &str) -> Self {
        let id = uuid::Uuid::new_v4().to_string()[..8].to_string();
        let now = chrono::Utc::now().to_rfc3339();
        Self {
            id,
            model: model.to_string(),
            created_at: now.clone(),
            updated_at: now,
            messages: Vec::new(),
            total_tokens: 0,
            iteration_count: 0,
        }
    }

    /// Get the sessions directory (~/.pawan/sessions/)
    pub fn sessions_dir() -> Result<PathBuf> {
        let home = std::env::var("HOME").unwrap_or_else(|_| "/root".to_string());
        let dir = PathBuf::from(home).join(".pawan").join("sessions");
        if !dir.exists() {
            std::fs::create_dir_all(&dir)
                .map_err(|e| PawanError::Config(format!("Failed to create sessions dir: {}", e)))?;
        }
        Ok(dir)
    }

    /// Save session to disk
    pub fn save(&mut self) -> Result<PathBuf> {
        self.updated_at = chrono::Utc::now().to_rfc3339();
        let dir = Self::sessions_dir()?;
        let path = dir.join(format!("{}.json", self.id));
        let json = serde_json::to_string_pretty(self)
            .map_err(|e| PawanError::Config(format!("Failed to serialize session: {}", e)))?;
        std::fs::write(&path, json)
            .map_err(|e| PawanError::Config(format!("Failed to write session: {}", e)))?;
        Ok(path)
    }

    /// Load a session from disk by ID
    pub fn load(id: &str) -> Result<Self> {
        let dir = Self::sessions_dir()?;
        let path = dir.join(format!("{}.json", id));
        if !path.exists() {
            return Err(PawanError::NotFound(format!("Session not found: {}", id)));
        }
        let content = std::fs::read_to_string(&path)
            .map_err(|e| PawanError::Config(format!("Failed to read session: {}", e)))?;
        serde_json::from_str(&content)
            .map_err(|e| PawanError::Config(format!("Failed to parse session: {}", e)))
    }

    /// List all saved sessions (sorted by updated_at, newest first)
    pub fn list() -> Result<Vec<SessionSummary>> {
        let dir = Self::sessions_dir()?;
        let mut sessions = Vec::new();

        if let Ok(entries) = std::fs::read_dir(&dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().is_some_and(|ext| ext == "json") {
                    if let Ok(content) = std::fs::read_to_string(&path) {
                        if let Ok(session) = serde_json::from_str::<Session>(&content) {
                            sessions.push(SessionSummary {
                                id: session.id,
                                model: session.model,
                                created_at: session.created_at,
                                updated_at: session.updated_at,
                                message_count: session.messages.len(),
                            });
                        }
                    }
                }
            }
        }

        sessions.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
        Ok(sessions)
    }
}

/// Summary of a saved session (for listing)
#[derive(Debug, Serialize, Deserialize)]
pub struct SessionSummary {
    pub id: String,
    pub model: String,
    pub created_at: String,
    pub updated_at: String,
    pub message_count: usize,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::Role;

    #[test]
    fn session_new_generates_8_char_id() {
        let s = Session::new("test-model");
        assert_eq!(s.id.len(), 8, "session id must be exactly 8 chars");
        assert_eq!(s.model, "test-model");
        assert!(s.messages.is_empty());
        assert_eq!(s.total_tokens, 0);
        assert_eq!(s.iteration_count, 0);
    }

    #[test]
    fn session_new_produces_distinct_ids() {
        // UUID prefix uniqueness — two fresh sessions in a row must differ.
        // (At 8 hex chars = 32 bits, birthday paradox says ~65k sessions
        // before 50% collision chance, so two in a row is safe.)
        let a = Session::new("m");
        let b = Session::new("m");
        assert_ne!(a.id, b.id, "successive Session::new() must produce distinct ids");
    }

    #[test]
    fn session_new_timestamps_parse_as_rfc3339() {
        let s = Session::new("m");
        // Both timestamps must be valid RFC3339 and equal at creation.
        assert_eq!(s.created_at, s.updated_at, "at creation created_at == updated_at");
        chrono::DateTime::parse_from_rfc3339(&s.created_at)
            .expect("created_at must parse as RFC3339");
        chrono::DateTime::parse_from_rfc3339(&s.updated_at)
            .expect("updated_at must parse as RFC3339");
    }

    #[test]
    fn session_serde_roundtrip_preserves_all_fields() {
        let mut original = Session::new("qwen-test");
        original.total_tokens = 12345;
        original.iteration_count = 7;
        original.messages.push(Message {
            role: Role::User,
            content: "hello".into(),
            tool_calls: vec![],
            tool_result: None,
        });
        let json = serde_json::to_string(&original).unwrap();
        let restored: Session = serde_json::from_str(&json).unwrap();
        assert_eq!(restored.id, original.id);
        assert_eq!(restored.model, original.model);
        assert_eq!(restored.created_at, original.created_at);
        assert_eq!(restored.updated_at, original.updated_at);
        assert_eq!(restored.total_tokens, 12345);
        assert_eq!(restored.iteration_count, 7);
        assert_eq!(restored.messages.len(), 1);
    }

    #[test]
    fn session_deserialize_tolerates_missing_token_fields() {
        // Old sessions written before total_tokens / iteration_count existed
        // must still load — they're marked #[serde(default)] so missing
        // fields deserialize to 0. Regression guard.
        let json = r#"{
            "id": "abcd1234",
            "model": "old-model",
            "created_at": "2026-01-01T00:00:00Z",
            "updated_at": "2026-01-01T00:00:00Z",
            "messages": []
        }"#;
        let session: Session = serde_json::from_str(json).unwrap();
        assert_eq!(session.id, "abcd1234");
        assert_eq!(session.total_tokens, 0, "missing total_tokens ⇒ default 0");
        assert_eq!(session.iteration_count, 0, "missing iteration_count ⇒ default 0");
    }

    #[test]
    fn session_summary_serde_roundtrip() {
        let summary = SessionSummary {
            id: "abcdef12".into(),
            model: "qwen3.5".into(),
            created_at: "2026-04-10T12:00:00Z".into(),
            updated_at: "2026-04-10T13:00:00Z".into(),
            message_count: 42,
        };
        let json = serde_json::to_string(&summary).unwrap();
        assert!(json.contains("\"id\":\"abcdef12\""));
        assert!(json.contains("\"message_count\":42"));
        let restored: SessionSummary = serde_json::from_str(&json).unwrap();
        assert_eq!(restored.id, "abcdef12");
        assert_eq!(restored.message_count, 42);
    }
}
