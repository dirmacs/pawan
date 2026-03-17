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
