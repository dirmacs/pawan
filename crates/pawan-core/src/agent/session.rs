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
    /// User notes/description for this session
    #[serde(default)]
    pub notes: String,
}

impl Session {
    /// Create a new session
    pub fn new(model: &str) -> Self {
        Self::new_with_tags(model, Vec::new())
    }

    /// Create a new session with a specific ID (e.g. for updates)
    pub fn new_with_id(id: String, model: &str, tags: Vec<String>) -> Self {
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
            notes: String::new(),
        }
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
            notes: String::new(),
        }
    }

    /// Create a new session with notes
    pub fn new_with_notes(model: &str, notes: String) -> Self {
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
            tags: Vec::new(),
            notes,
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

    /// Import a session from a JSON file
    pub fn from_json_file(path: impl AsRef<std::path::Path>) -> Result<Self> {
        let content = std::fs::read_to_string(path)
            .map_err(|e| PawanError::Config(format!("Failed to read session file: {}", e)))?;
        let mut session: Session = serde_json::from_str(&content)
            .map_err(|e| PawanError::Config(format!("Failed to parse session JSON: {}", e)))?;
        
        // Assign a new ID to ensure it doesn't collide with existing sessions
        // and clearly mark it as a new import in this system.
        session.id = uuid::Uuid::new_v4().to_string()[..8].to_string();
        session.updated_at = chrono::Utc::now().to_rfc3339();
        
        Ok(session)
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
                                notes: session.notes,
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
    /// User notes for this session
    #[serde(default)]
    pub notes: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::Role;
    use serial_test::serial;

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
            notes: String::new(),
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

    #[serial(pawan_session_tests)]
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

    #[serial(pawan_session_tests)]
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

    #[serial(pawan_session_tests)]
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

    #[serial(pawan_session_tests)]
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
    /// Context before the match (for better preview)
    #[serde(default)]
    pub context_before: String,
    /// Context after the match (for better preview)
    #[serde(default)]
    pub context_after: String,
    /// The actual matched text
    #[serde(default)]
    pub matched_text: String,
}

/// Search options for filtering sessions
#[derive(Debug, Clone, Default)]
pub struct SearchOptions {
    /// Filter by role (None = all roles)
    pub role_filter: Option<Role>,
    /// Filter by date range (start date in RFC3339 format)
    pub date_from: Option<String>,
    /// Filter by date range (end date in RFC3339 format)
    pub date_to: Option<String>,
    /// Maximum number of results per session
    pub max_matches_per_session: Option<usize>,
    /// Context window size (characters before/after match)
    pub context_window: usize,
}

impl SearchOptions {
    /// Create new search options with defaults
    pub fn new() -> Self {
        Self {
            role_filter: None,
            date_from: None,
            date_to: None,
            max_matches_per_session: Some(5),
            context_window: 50,
        }
    }

    /// Set role filter
    pub fn with_role(mut self, role: Role) -> Self {
        self.role_filter = Some(role);
        self
    }

    /// Set date range filter
    pub fn with_date_range(mut self, from: Option<String>, to: Option<String>) -> Self {
        self.date_from = from;
        self.date_to = to;
        self
    }

    /// Set max matches per session
    pub fn with_max_matches(mut self, max: usize) -> Self {
        self.max_matches_per_session = Some(max);
        self
    }

    /// Set context window size
    pub fn with_context_window(mut self, window: usize) -> Self {
        self.context_window = window;
        self
    }
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

/// Search sessions by content query with options
pub fn search_sessions_with_options(query: &str, options: &SearchOptions) -> Result<Vec<SearchResult>> {
    let dir = Session::sessions_dir()?;
    let query_lower = query.to_lowercase();
    let mut results = Vec::new();
    
    if let Ok(entries) = std::fs::read_dir(&dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().is_some_and(|ext| ext == "json") {
                if let Ok(content) = std::fs::read_to_string(&path) {
                    if let Ok(session) = serde_json::from_str::<Session>(&content) {
                        // Apply date filter if specified
                        if let (Some(from), Some(to)) = (&options.date_from, &options.date_to) {
                            if let Ok(updated) = chrono::DateTime::parse_from_rfc3339(&session.updated_at) {
                                let updated_utc = updated.with_timezone(&chrono::Utc);
                                if let (Ok(from_dt), Ok(to_dt)) = (
                                    chrono::DateTime::parse_from_rfc3339(from),
                                    chrono::DateTime::parse_from_rfc3339(to)
                                ) {
                                    let from_utc = from_dt.with_timezone(&chrono::Utc);
                                    let to_utc = to_dt.with_timezone(&chrono::Utc);
                                    if updated_utc < from_utc || updated_utc > to_utc {
                                        continue; // Skip sessions outside date range
                                    }
                                }
                            }
                        }
                        
                        let mut matches = Vec::new();
                        for (i, msg) in session.messages.iter().enumerate() {
                            // Apply role filter if specified
                            if let Some(ref role_filter) = options.role_filter {
                                if &msg.role != role_filter {
                                    continue;
                                }
                            }
                            
                            if msg.content.to_lowercase().contains(&query_lower) {
                                // Find the match position for context extraction
                                let content_lower = msg.content.to_lowercase();
                                if let Some(pos) = content_lower.find(&query_lower) {
                                    let start = if pos >= options.context_window {
                                        pos - options.context_window
                                    } else {
                                        0
                                    };
                                    let end = std::cmp::min(
                                        pos + query.len() + options.context_window,
                                        msg.content.len()
                                    );
                                    
                                    let context_before = msg.content[start..pos].to_string();
                                    let matched_text = msg.content[pos..pos + query.len()].to_string();
                                    let context_after = msg.content[pos + query.len()..end].to_string();
                                    
                                    // Create preview with context
                                    let preview = format!(
                                        "{}{}{}",
                                        if start > 0 { "..." } else { "" },
                                        &msg.content[start..end],
                                        if end < msg.content.len() { "..." } else { "" }
                                    );
                                    
                                    matches.push(MessageMatch {
                                        message_index: i,
                                        role: msg.role.clone(),
                                        preview,
                                        context_before,
                                        context_after,
                                        matched_text,
                                    });
                                }
                            }
                        }
                        
                        if !matches.is_empty() {
                            // Limit matches per session if specified
                            let limited_matches = if let Some(max) = options.max_matches_per_session {
                                matches.into_iter().take(max).collect()
                            } else {
                                matches
                            };
                            
                            results.push(SearchResult {
                                id: session.id,
                                model: session.model,
                                updated_at: session.updated_at,
                                message_count: session.messages.len(),
                                tags: session.tags,
                                matches: limited_matches,
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

/// Search sessions by content query (legacy function for backwards compatibility)
pub fn search_sessions(query: &str) -> Result<Vec<SearchResult>> {
    search_sessions_with_options(query, &SearchOptions::new())
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

#[cfg(test)]
mod search_prune_tests {
    use super::*;
    use crate::agent::{Message, Role};
    use serial_test::serial;

    #[test]
    fn test_role_serialization_is_lowercase() {
        assert_eq!(serde_json::to_string(&Role::User).unwrap(), "\"user\"");
        assert_eq!(serde_json::to_string(&Role::Assistant).unwrap(), "\"assistant\"");
        assert_eq!(serde_json::to_string(&Role::System).unwrap(), "\"system\"");
        assert_eq!(serde_json::to_string(&Role::Tool).unwrap(), "\"tool\"");
    }

    #[test]
    #[serial]
    fn test_search_sessions_logic() {
        let tmp = tempfile::tempdir().unwrap();
        let prev_home = std::env::var("HOME").ok();
        std::env::set_var("HOME", tmp.path());

        // Create 2 sessions
        let mut s1 = Session::new("m1");
        s1.messages.push(Message {
            role: Role::User,
            content: "hello world".into(),
            tool_calls: vec![],
            tool_result: None,
        });
        s1.save().unwrap();

        let mut s2 = Session::new("m2");
        s2.messages.push(Message {
            role: Role::User,
            content: "goodbye world".into(),
            tool_calls: vec![],
            tool_result: None,
        });
        s2.save().unwrap();

        // Search for "hello"
        let results = search_sessions("hello").unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].id, s1.id);
        assert_eq!(results[0].matches.len(), 1);
        assert_eq!(results[0].matches[0].preview, "hello world");

        // Search for "world" (both)
        let results = search_sessions("world").unwrap();
        assert_eq!(results.len(), 2);

        // Restore HOME
        if let Some(h) = prev_home { std::env::set_var("HOME", h); } else { std::env::remove_var("HOME"); }
    }

    #[test]
    #[serial]
    fn test_prune_sessions_logic() {
        let tmp = tempfile::tempdir().unwrap();
        let prev_home = std::env::var("HOME").ok();
        std::env::set_var("HOME", tmp.path());

        // Create 5 sessions with different timestamps manually
        let dir = Session::sessions_dir().unwrap();
        for i in 0..5 {
            let mut s = Session::new("m");
            s.id = format!("sess{}", i);
            s.updated_at = format!("2026-04-1{}T12:00:00Z", i);
            let path = dir.join(format!("{}.json", s.id));
            let json = serde_json::to_string_pretty(&s).unwrap();
            std::fs::write(&path, json).unwrap();
        }

        // Policy: keep 2 most recent
        let policy = RetentionPolicy {
            max_age_days: None,
            max_sessions: Some(2),
            keep_tags: vec![],
        };
        let deleted = prune_sessions(&policy).unwrap();
        assert_eq!(deleted, 3);

        let list = Session::list().unwrap();
        assert_eq!(list.len(), 2);
        // Should be sess4 and sess3 (newest)
        assert!(list.iter().any(|s| s.id == "sess4"));
        assert!(list.iter().any(|s| s.id == "sess3"));

        // Restore HOME
        if let Some(h) = prev_home { std::env::set_var("HOME", h); } else { std::env::remove_var("HOME"); }
    }

    #[test]
    #[serial]
    fn test_prune_sessions_age_and_tags() {
        let tmp = tempfile::tempdir().unwrap();
        let prev_home = std::env::var("HOME").ok();
        std::env::set_var("HOME", tmp.path());

        let dir = Session::sessions_dir().unwrap();

        // 1. Old session (30 days ago)
        let mut s1 = Session::new("m");
        s1.id = "old".into();
        s1.updated_at = "2020-01-01T00:00:00Z".into();
        let path1 = dir.join(format!("{}.json", s1.id));
        std::fs::write(&path1, serde_json::to_string_pretty(&s1).unwrap()).unwrap();

        // 2. Old but protected by tag
        let mut s2 = Session::new_with_tags("m", vec!["keep".into()]);
        s2.id = "protected".into();
        s2.updated_at = "2020-01-01T00:00:00Z".into();
        let path2 = dir.join(format!("{}.json", s2.id));
        std::fs::write(&path2, serde_json::to_string_pretty(&s2).unwrap()).unwrap();

        // 3. New session
        let mut s3 = Session::new("m");
        s3.id = "new".into();
        s3.save().unwrap(); // save() is fine for 'new' session

        let policy = RetentionPolicy {
            max_age_days: Some(7),
            max_sessions: None,
            keep_tags: vec!["keep".into()],
        };
        let deleted = prune_sessions(&policy).unwrap();
        assert_eq!(deleted, 1); // Only 'old' deleted

        let list = Session::list().unwrap();
        assert_eq!(list.len(), 2);
        assert!(list.iter().any(|s| s.id == "protected"));
        assert!(list.iter().any(|s| s.id == "new"));

        // Restore HOME
        if let Some(h) = prev_home { std::env::set_var("HOME", h); } else { std::env::remove_var("HOME"); }
    }

    #[test]
    #[serial]
    fn test_search_sessions_no_results() {
        let tmp = tempfile::tempdir().unwrap();
        let prev_home = std::env::var("HOME").ok();
        std::env::set_var("HOME", tmp.path());

        let results = search_sessions("anything").unwrap();
        assert!(results.is_empty());

        if let Some(h) = prev_home { std::env::set_var("HOME", h); } else { std::env::remove_var("HOME"); }
    }

    #[test]
    #[serial]
    fn test_prune_sessions_zero_limits() {
        let tmp = tempfile::tempdir().unwrap();
        let prev_home = std::env::var("HOME").ok();
        std::env::set_var("HOME", tmp.path());

        let mut s = Session::new("m");
        s.save().unwrap();

        // Policy: keep 0 sessions
        let policy = RetentionPolicy {
            max_age_days: None,
            max_sessions: Some(0),
            keep_tags: vec![],
        };
        let deleted = prune_sessions(&policy).unwrap();
        assert_eq!(deleted, 1);

        if let Some(h) = prev_home { std::env::set_var("HOME", h); } else { std::env::remove_var("HOME"); }
    }

    #[test]
    fn test_search_options_builder() {
        let options = SearchOptions::new()
            .with_role(Role::User)
            .with_date_range(Some("2026-01-01T00:00:00Z".to_string()), Some("2026-12-31T23:59:59Z".to_string()))
            .with_max_matches(10)
            .with_context_window(100);
        
        assert_eq!(options.role_filter, Some(Role::User));
        assert_eq!(options.date_from, Some("2026-01-01T00:00:00Z".to_string()));
        assert_eq!(options.date_to, Some("2026-12-31T23:59:59Z".to_string()));
        assert_eq!(options.max_matches_per_session, Some(10));
        assert_eq!(options.context_window, 100);
    }

    #[test]
    #[serial]
    fn test_search_sessions_with_role_filter() {
        let tmp = tempfile::tempdir().unwrap();
        let prev_home = std::env::var("HOME").ok();
        std::env::set_var("HOME", tmp.path());

        // Create sessions with different roles
        let mut s1 = Session::new("m1");
        s1.messages.push(Message {
            role: Role::User,
            content: "hello world".into(),
            tool_calls: vec![],
            tool_result: None,
        });
        s1.messages.push(Message {
            role: Role::Assistant,
            content: "hello there".into(),
            tool_calls: vec![],
            tool_result: None,
        });
        s1.save().unwrap();

        // Search for "hello" with user role filter
        let options = SearchOptions::new().with_role(Role::User);
        let results = search_sessions_with_options("hello", &options).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].matches.len(), 1);
        assert_eq!(results[0].matches[0].role, Role::User);

        // Search for "hello" with assistant role filter
        let options = SearchOptions::new().with_role(Role::Assistant);
        let results = search_sessions_with_options("hello", &options).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].matches.len(), 1);
        assert_eq!(results[0].matches[0].role, Role::Assistant);

        if let Some(h) = prev_home { std::env::set_var("HOME", h); } else { std::env::remove_var("HOME"); }
    }

    #[test]
    #[serial]
    fn test_search_sessions_context_extraction() {
        let tmp = tempfile::tempdir().unwrap();
        let prev_home = std::env::var("HOME").ok();
        std::env::set_var("HOME", tmp.path());

        // Create session with long message
        let mut s1 = Session::new("m1");
        s1.messages.push(Message {
            role: Role::User,
            content: "This is a long message with the word hello in the middle of the text".into(),
            tool_calls: vec![],
            tool_result: None,
        });
        s1.save().unwrap();

        // Search with context window
        let options = SearchOptions::new().with_context_window(10);
        let results = search_sessions_with_options("hello", &options).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].matches.len(), 1);
        
        let match_result = &results[0].matches[0];
        assert!(!match_result.context_before.is_empty());
        assert!(!match_result.context_after.is_empty());
        assert_eq!(match_result.matched_text, "hello");
        assert!(match_result.preview.contains("hello"));

        if let Some(h) = prev_home { std::env::set_var("HOME", h); } else { std::env::remove_var("HOME"); }
    }

    #[test]
    #[serial]
    fn test_search_sessions_max_matches_limit() {
        let tmp = tempfile::tempdir().unwrap();
        let prev_home = std::env::var("HOME").ok();
        std::env::set_var("HOME", tmp.path());

        // Create session with multiple matches
        let mut s1 = Session::new("m1");
        for i in 0..10 {
            s1.messages.push(Message {
                role: Role::User,
                content: format!("Message {} with hello text", i),
                tool_calls: vec![],
                tool_result: None,
            });
        }
        s1.save().unwrap();

        // Search with max matches limit
        let options = SearchOptions::new().with_max_matches(3);
        let results = search_sessions_with_options("hello", &options).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].matches.len(), 3); // Limited to 3

        if let Some(h) = prev_home { std::env::set_var("HOME", h); } else { std::env::remove_var("HOME"); }
    }

    #[test]
    #[serial]
    fn test_search_sessions_case_insensitive() {
        let tmp = tempfile::tempdir().unwrap();
        let prev_home = std::env::var("HOME").ok();
        std::env::set_var("HOME", tmp.path());

        // Create session with mixed case
        let mut s1 = Session::new("m1");
        s1.messages.push(Message {
            role: Role::User,
            content: "HeLLo WoRLd".into(),
            tool_calls: vec![],
            tool_result: None,
        });
        s1.save().unwrap();

        // Search with lowercase query
        let results = search_sessions("hello").unwrap();
        assert_eq!(results.len(), 1);

        // Search with uppercase query
        let results = search_sessions("HELLO").unwrap();
        assert_eq!(results.len(), 1);

        if let Some(h) = prev_home { std::env::set_var("HOME", h); } else { std::env::remove_var("HOME"); }
    }

    #[test]
    fn test_session_new_with_tags() {
        let session = Session::new_with_tags("test-model", vec!["tag1".to_string(), "tag2".to_string()]);
        assert_eq!(session.tags, vec!["tag1".to_string(), "tag2".to_string()]);
        assert_eq!(session.model, "test-model");
    }

    #[test]
    fn test_session_new_with_notes() {
        let session = Session::new_with_notes("test-model", "Test notes".to_string());
        assert_eq!(session.notes, "Test notes");
        assert_eq!(session.model, "test-model");
    }

    #[test]
    fn test_session_add_tag() {
        let mut session = Session::new("test-model");
        session.add_tag("tag1").unwrap();
        assert!(session.tags.contains(&"tag1".to_string()));
        assert_eq!(session.tags.len(), 1);
    }

    #[test]
    fn test_session_remove_tag() {
        let mut session = Session::new_with_tags("test-model", vec!["tag1".to_string(), "tag2".to_string()]);
        session.remove_tag("tag1").unwrap();
        assert!(!session.tags.contains(&"tag1".to_string()));
        assert!(session.tags.contains(&"tag2".to_string()));
        assert_eq!(session.tags.len(), 1);
    }

    #[test]
    fn test_session_clear_tags() {
        let mut session = Session::new_with_tags("test-model", vec!["tag1".to_string(), "tag2".to_string()]);
        session.clear_tags();
        assert!(session.tags.is_empty());
    }

    #[test]
    fn test_session_has_tag() {
        let session = Session::new_with_tags("test-model", vec!["tag1".to_string(), "tag2".to_string()]);
        assert!(session.has_tag("tag1"));
        assert!(session.has_tag("tag2"));
        assert!(!session.has_tag("tag3"));
    }

    #[serial(pawan_session_tests)]
    #[test]
    fn test_session_save_and_load() {
        let mut session = Session::new("test-model");
        session.messages.push(Message {
            role: Role::User,
            content: "Test message".to_string(),
            tool_calls: vec![],
            tool_result: None,
        });
        session.add_tag("test-tag").unwrap();
        session.notes = "Test notes".to_string();

        let id = session.id.clone();
        let path = session.save().expect("save must succeed");

        let loaded = Session::load(&id).expect("load must succeed");
        assert_eq!(loaded.id, id);
        assert_eq!(loaded.model, "test-model");
        assert_eq!(loaded.messages.len(), 1);
        assert_eq!(loaded.messages[0].content, "Test message");
        assert!(loaded.tags.contains(&"test-tag".to_string()));
        assert_eq!(loaded.notes, "Test notes");

        // cleanup
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn test_session_new_with_id() {
        let session = Session::new_with_id(
            "custom-id".to_string(),
            "test-model",
            vec!["tag1".to_string()]
        );
        assert_eq!(session.id, "custom-id");
        assert_eq!(session.model, "test-model");
        assert_eq!(session.tags, vec!["tag1".to_string()]);
    }
}
