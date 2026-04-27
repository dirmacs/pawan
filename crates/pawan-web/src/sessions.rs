use serde::Serialize;
use std::fs;
use std::path::PathBuf;

#[derive(Debug, Serialize)]
/// Summary information about a chat session
///
/// This struct represents basic metadata about a saved chat session,
/// used for listing sessions in the UI.
pub struct SessionSummary {
    pub id: String,
    pub created_at: String,
    pub message_count: usize,
    pub size_bytes: u64,
}

#[derive(Debug, Serialize)]
/// Detailed information about a chat session
///
/// This struct contains the full content of a saved chat session,
/// including all messages and their metadata.
pub struct SessionDetail {
    pub id: String,
    pub messages: serde_json::Value,
}

fn sessions_dir() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/root".into());
    PathBuf::from(home).join(".pawan").join("sessions")
}

/// List all saved chat sessions
///
/// Returns a list of all saved chat sessions with their metadata.
///
/// # Returns
/// * `Ok(Vec<SessionSummary>)` - List of session summaries sorted by creation date (newest first)
/// * `Err(String)` - Error message if session directory cannot be read
pub fn list_sessions() -> Result<Vec<SessionSummary>, String> {
    let dir = sessions_dir();
    if !dir.exists() {
        return Ok(vec![]);
    }

    let mut sessions = Vec::new();
    let entries = fs::read_dir(&dir).map_err(|e| e.to_string())?;

    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }

        let id = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("unknown")
            .to_string();

        let metadata = fs::metadata(&path).map_err(|e| e.to_string())?;
        let size_bytes = metadata.len();

        let created_at = metadata
            .created()
            .or_else(|_| metadata.modified())
            .map(|t| {
                let dt: chrono::DateTime<chrono::Utc> = t.into();
                dt.to_rfc3339()
            })
            .unwrap_or_default();

        // Count messages by parsing
        let message_count = fs::read_to_string(&path)
            .ok()
            .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
            .and_then(|v| {
                v.get("messages")
                    .and_then(|m| m.as_array())
                    .map(|a| a.len())
            })
            .unwrap_or(0);

        sessions.push(SessionSummary {
            id,
            created_at,
            message_count,
            size_bytes,
        });
    }

    sessions.sort_by(|a, b| b.created_at.cmp(&a.created_at));
    Ok(sessions)
}

/// Get a specific chat session by ID
///
/// Retrieves the full content of a saved chat session.
///
/// # Arguments
/// * `id` - The session ID to retrieve
///
/// # Returns
/// * `Ok(SessionDetail)` - The session content
/// * `Err(String)` - Error message if session is not found or cannot be read
pub fn get_session(id: &str) -> Result<SessionDetail, String> {
    let path = sessions_dir().join(format!("{}.json", id));
    if !path.exists() {
        return Err(format!("session '{}' not found", id));
    }

    let content = fs::read_to_string(&path).map_err(|e| e.to_string())?;
    let messages: serde_json::Value = serde_json::from_str(&content).map_err(|e| e.to_string())?;

    Ok(SessionDetail {
        id: id.to_string(),
        messages,
    })
}

/// Delete a chat session by ID
///
/// Permanently deletes a saved chat session.
///
/// # Arguments
/// * `id` - The session ID to delete
///
/// # Returns
/// * `Ok(())` - Session successfully deleted
/// * `Err(String)` - Error message if session is not found or cannot be deleted
pub fn delete_session(id: &str) -> Result<(), String> {
    let path = sessions_dir().join(format!("{}.json", id));
    if !path.exists() {
        return Err(format!("session '{}' not found", id));
    }
    fs::remove_file(&path).map_err(|e| e.to_string())
}
#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn test_sessions_dir_uses_home_env() {
        let dir = sessions_dir();
        assert!(dir.ends_with(".pawan/sessions"));
    }

    #[test]
    fn test_list_sessions_empty_directory() {
        let tmp = TempDir::new().unwrap();
        let sessions_path = tmp.path().join(".pawan").join("sessions");
        fs::create_dir_all(&sessions_path).unwrap();

        // Temporarily override HOME env var
        std::env::set_var("HOME", tmp.path());
        let sessions = list_sessions().unwrap();
        std::env::remove_var("HOME");

        assert!(sessions.is_empty());
    }

    #[test]
    fn test_list_sessions_nonexistent_directory() {
        // Temporarily override HOME to a non-existent path
        std::env::set_var("HOME", "/nonexistent/path/that/does/not/exist");
        let sessions = list_sessions().unwrap();
        std::env::remove_var("HOME");

        assert!(sessions.is_empty());
    }

    #[test]
    fn test_get_session_not_found() {
        let tmp = TempDir::new().unwrap();
        std::env::set_var("HOME", tmp.path());

        let result = get_session("nonexistent");
        std::env::remove_var("HOME");

        assert!(result.is_err());
        assert!(result.unwrap_err().contains("not found"));
    }

    #[test]
    fn test_delete_session_not_found() {
        let tmp = TempDir::new().unwrap();
        std::env::set_var("HOME", tmp.path());

        let result = delete_session("nonexistent");
        std::env::remove_var("HOME");

        assert!(result.is_err());
        assert!(result.unwrap_err().contains("not found"));
    }

    #[test]
    fn test_delete_session_success() {
        let tmp = TempDir::new().unwrap();
        let sessions_path = tmp.path().join(".pawan").join("sessions");
        fs::create_dir_all(&sessions_path).unwrap();

        let session_path = sessions_path.join("test.json");
        fs::write(&session_path, r#"{"messages":[]}"#).unwrap();

        std::env::set_var("HOME", tmp.path());
        let result = delete_session("test");
        std::env::remove_var("HOME");

        assert!(result.is_ok());
        assert!(!session_path.exists());
    }

    #[test]
    fn test_get_session_success() {
        let tmp = TempDir::new().unwrap();
        let sessions_path = tmp.path().join(".pawan").join("sessions");
        fs::create_dir_all(&sessions_path).unwrap();

        let session_path = sessions_path.join("test.json");
        let content = r#"{"messages":[{"role":"user","content":"hello"}]}"#;
        fs::write(&session_path, content).unwrap();

        std::env::set_var("HOME", tmp.path());
        let result = get_session("test");
        std::env::remove_var("HOME");

        assert!(result.is_ok());
        let session = result.unwrap();
        assert_eq!(session.id, "test");
        assert!(session.messages.is_object());
    }
}
