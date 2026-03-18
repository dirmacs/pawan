use serde::Serialize;
use std::fs;
use std::path::PathBuf;

#[derive(Debug, Serialize)]
pub struct SessionSummary {
    pub id: String,
    pub created_at: String,
    pub message_count: usize,
    pub size_bytes: u64,
}

#[derive(Debug, Serialize)]
pub struct SessionDetail {
    pub id: String,
    pub messages: serde_json::Value,
}

fn sessions_dir() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/root".into());
    PathBuf::from(home).join(".pawan").join("sessions")
}

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
            .and_then(|v| v.get("messages").and_then(|m| m.as_array()).map(|a| a.len()))
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

pub fn get_session(id: &str) -> Result<SessionDetail, String> {
    let path = sessions_dir().join(format!("{}.json", id));
    if !path.exists() {
        return Err(format!("session '{}' not found", id));
    }

    let content = fs::read_to_string(&path).map_err(|e| e.to_string())?;
    let messages: serde_json::Value =
        serde_json::from_str(&content).map_err(|e| e.to_string())?;

    Ok(SessionDetail {
        id: id.to_string(),
        messages,
    })
}

pub fn delete_session(id: &str) -> Result<(), String> {
    let path = sessions_dir().join(format!("{}.json", id));
    if !path.exists() {
        return Err(format!("session '{}' not found", id));
    }
    fs::remove_file(&path).map_err(|e| e.to_string())
}
