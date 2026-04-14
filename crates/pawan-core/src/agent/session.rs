//! Session persistence — save and resume conversations

use crate::agent::{Message, Role};
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
    /// User-defined tags for this session
    #[serde(default)]
    pub tags: Vec<String>,
}

impl Session {
    /// Create a new session
    pub fn new(model: &str) -> Self {
        Self::new_with_tags(model, Vec::new())
    }

    /// Create a new session with tags
    pub fn new_with_tags(model: &str, tags: Vec<String>) -> Self {
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
            tags,
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

    /// Add a tag to the session (validates and prevents duplicates)
    pub fn add_tag(&mut self, tag: &str) -> Result<()> {
        let sanitized = Self::sanitize_tag(tag)?;
        if self.tags.contains(&sanitized) {
            return Err(PawanError::Config(format!("Tag already exists: {}", sanitized)));
        }
        self.tags.push(sanitized);
        Ok(())
    }

    /// Remove a tag from the session
    pub fn remove_tag(&mut self, tag: &str) -> Result<()> {
        let sanitized = Self::sanitize_tag(tag)?;
        if let Some(pos) = self.tags.iter().position(|t| t == &sanitized) {
            self.tags.remove(pos);
            Ok(())
        } else {
            Err(PawanError::NotFound(format!("Tag not found: {}", sanitized)))
        }
    }

    /// Clear all tags from the session
    pub fn clear_tags(&mut self) {
        self.tags.clear();
    }

    /// Check if session has a specific tag
    pub fn has_tag(&self, tag: &str) -> bool {
        match Self::sanitize_tag(tag) {
            Ok(sanitized) => self.tags.contains(&sanitized),
            Err(_) => false,
        }
    }

    /// Sanitize and validate a tag name
    fn sanitize_tag(tag: &str) -> Result<String> {
        let trimmed = tag.trim();
        if trimmed.is_empty() {
            return Err(PawanError::Config("Tag name cannot be empty".to_string()));
        }
        if trimmed.len() > 50 {
            return Err(PawanError::Config("Tag name too long (max 50 characters)".to_string()));
        }
        // Allow alphanumeric, hyphen, underscore, and space
        let sanitized: String = trimmed
            .chars()
            .filter(|c| c.is_alphanumeric() || *c == '-' || *c == '_' || *c == ' ')
            .collect();
        if sanitized.is_empty() {
            return Err(PawanError::Config("Tag contains invalid characters".to_string()));
        }
        Ok(sanitized)
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
                                tags: session.tags,
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
    /// User-defined tags for this session
    #[serde(default)]
    pub tags: Vec<String>,
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
            tags: Vec::new(),
        };
        let json = serde_json::to_string(&summary).unwrap();
        assert!(json.contains("\"id\":\"abcdef12\""));
        assert!(json.contains("\"message_count\":42"));
        let restored: SessionSummary = serde_json::from_str(&json).unwrap();
        assert_eq!(restored.id, "abcdef12");
        assert_eq!(restored.message_count, 42);
    }

    // ─── I/O path tests ───────────────────────────────────────────────────

    #[test]
    fn test_load_nonexistent_id_returns_not_found() {
        // Use an ID that is guaranteed not to exist on disk.
        let err = Session::load("__test_nonexistent_id_zzz__").unwrap_err();
        match err {
            crate::PawanError::NotFound(msg) => {
                assert!(msg.contains("Session not found"), "unexpected: {msg}")
            }
            other => panic!("expected NotFound, got {:?}", other),
        }
    }

    #[test]
    fn test_save_and_load_roundtrip() {
        let mut session = Session::new("roundtrip-model");
        session.total_tokens = 999;
        session.iteration_count = 3;
        session.messages.push(Message {
            role: Role::User,
            content: "save-load test".into(),
            tool_calls: vec![],
            tool_result: None,
        });
        let id = session.id.clone();

        let path = session.save().expect("save must succeed");
        assert!(path.exists(), "saved file must exist at {:?}", path);

        let loaded = Session::load(&id).expect("load by id must succeed");
        assert_eq!(loaded.id, id);
        assert_eq!(loaded.model, "roundtrip-model");
        assert_eq!(loaded.total_tokens, 999);
        assert_eq!(loaded.iteration_count, 3);
        assert_eq!(loaded.messages.len(), 1);

        // cleanup
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn test_save_updates_updated_at() {
        let mut session = Session::new("timestamp-model");
        let original_updated = session.updated_at.clone();
        // Small sleep to ensure clock advances
        std::thread::sleep(std::time::Duration::from_millis(10));
        let path = session.save().expect("save must succeed");
        // updated_at must be >= created_at (may be equal if sub-ms precision)
        let updated = chrono::DateTime::parse_from_rfc3339(&session.updated_at)
            .expect("updated_at must be valid RFC3339");
        let orig = chrono::DateTime::parse_from_rfc3339(&original_updated)
            .expect("original_updated must be valid RFC3339");
        assert!(
            updated >= orig,
            "updated_at after save must be >= created_at"
        );
        // cleanup
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn test_list_includes_saved_session() {
        let mut session = Session::new("list-test-model");
        let id = session.id.clone();
        let path = session.save().expect("save must succeed");

        let summaries = Session::list().expect("list must succeed");
        let found = summaries.iter().any(|s| s.id == id);
        assert!(found, "newly saved session must appear in list()");

        // cleanup
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn test_list_sorted_newest_first() {
        // Create two sessions and force different updated_at values.
        let mut older = Session::new("older-model");
        older.updated_at = "2020-01-01T00:00:00Z".to_string();
        let path_older = older.save().expect("save older");

        let mut newer = Session::new("newer-model");
        newer.updated_at = "2030-01-01T00:00:00Z".to_string();
        let path_newer = newer.save().expect("save newer");

        let summaries = Session::list().expect("list must succeed");

        // Find positions of our two sessions in the sorted list
        let pos_older = summaries.iter().position(|s| s.id == older.id);
        let pos_newer = summaries.iter().position(|s| s.id == newer.id);

        if let (Some(po), Some(pn)) = (pos_older, pos_newer) {
            assert!(
                pn < po,
                "newer session (pos {pn}) must appear before older (pos {po}) in list"
            );
        }

        // cleanup
        let _ = std::fs::remove_file(&path_older);
        let _ = std::fs::remove_file(&path_newer);
    }
}

// ========== Session Search and Pruning ==========

/// Search result for a session
#[derive(Debug, Serialize, Deserialize)]
pub struct SearchResult {
    pub id: String,
    pub model: String,
    pub updated_at: String,
    pub message_count: usize,
    #[serde(default)]
    pub tags: Vec<String>,
    pub matches: Vec<MessageMatch>,
}

/// A matching message within a search result
#[derive(Debug, Serialize, Deserialize)]
pub struct MessageMatch {
    pub message_index: usize,
    pub role: Role,
    pub preview: String,
}

/// Retention policy for session cleanup
#[derive(Debug, Clone, Default)]
pub struct RetentionPolicy {
    /// Maximum age in days (None = no limit)
    pub max_age_days: Option<u32>,
    /// Maximum number of sessions to keep (None = no limit)
    pub max_sessions: Option<usize>,
    /// Tags to always keep
    pub keep_tags: Vec<String>,
}

/// Search sessions by content query
pub fn search_sessions(query: &str) -> Result<Vec<SearchResult>> {
    let dir = Session::sessions_dir()?;
    let query_lower = query.to_lowercase();
    let mut results = Vec::new();
    
    if let Ok(entries) = std::fs::read_dir(&dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().is_some_and(|ext| ext == "json") {
                if let Ok(content) = std::fs::read_to_string(&path) {
                    if let Ok(session) = serde_json::from_str::<Session>(&content) {
                        let mut matches = Vec::new();
                        for (i, msg) in session.messages.iter().enumerate() {
                            if msg.content.to_lowercase().contains(&query_lower) {
                                let preview = msg.content.chars().take(100).collect::<String>();
                                matches.push(MessageMatch {
                                    message_index: i,
                                    role: msg.role.clone(),
                                    preview,
                                });
                            }
                        }
                        if !matches.is_empty() {
                            results.push(SearchResult {
                                id: session.id,
                                model: session.model,
                                updated_at: session.updated_at,
                                message_count: session.messages.len(),
                                tags: session.tags,
                                matches,
                            });
                        }
                    }
                }
            }
        }
    }
    
    results.sort_by(|a, b| b.matches.len().cmp(&a.matches.len()));
    Ok(results)
}

/// Prune sessions based on retention policy
pub fn prune_sessions(policy: &RetentionPolicy) -> Result<usize> {
    let dir = Session::sessions_dir()?;
    let mut sessions_data: Vec<(std::path::PathBuf, Session)> = Vec::new();
    
    if let Ok(entries) = std::fs::read_dir(&dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().is_some_and(|ext| ext == "json") {
                if let Ok(content) = std::fs::read_to_string(&path) {
                    if let Ok(session) = serde_json::from_str::<Session>(&content) {
                        sessions_data.push((path, session));
                    }
                }
            }
        }
    }
    
    sessions_data.sort_by(|a, b| b.1.updated_at.cmp(&a.1.updated_at));
    
    let mut deleted = 0usize;
    let now = chrono::Utc::now();
    
    // Use enumerate to get index without moving the vector
    for (i, (path, session)) in sessions_data.into_iter().enumerate() {
        // Skip sessions with protected tags
        let has_protected = session.tags.iter()
            .any(|t| policy.keep_tags.iter().any(|kt| kt == t));
        if has_protected {
            continue;
        }
        
        let mut should_delete = false;
        
        // Check age limit
        if let Some(max_days) = policy.max_age_days {
            if let Ok(st) = chrono::DateTime::parse_from_rfc3339(&session.updated_at) {
                let age = (now - st.with_timezone(&chrono::Utc)).num_days() as u32;
                if age > max_days {
                    should_delete = true;
                }
            }
        }
        
        // Check max sessions limit (use index from enumerate)
        if !should_delete {
            if let Some(max_sess) = policy.max_sessions {
                if i >= max_sess {
                    should_delete = true;
                }
            }
        }
        
        if should_delete {
            std::fs::remove_file(&path)
                .map_err(|e| PawanError::Config(format!("Delete failed: {}", e)))?;
            deleted += 1;
        }
    }
    
    Ok(deleted)
}
