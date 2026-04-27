//! Session-scoped memory boundaries and key/content sanitization.

use crate::memory::{now_rfc3339, Memory, MemoryStore};
use crate::{PawanError, Result};
use std::cmp::Ordering;
use std::collections::HashSet;

/// Maximum key length in Unicode scalar values (after sanitization).
pub const MAX_KEY_CHARS: usize = 256;

/// Maximum serialized memory content size (bytes).
pub const MAX_CONTENT_BYTES: usize = 1024 * 1024;

/// A memory store that is scoped to a specific session.
/// Prevents memory from one session leaking into another.
pub struct SessionScopedMemory {
    store: MemoryStore,
    session_id: String,
}

impl SessionScopedMemory {
    pub fn new(store: MemoryStore, session_id: String) -> Self {
        Self { store, session_id }
    }

    fn require_session(&self) -> Result<()> {
        if self.session_id.is_empty() {
            return Err(PawanError::Config(
                "SessionScopedMemory requires a non-empty session_id".to_string(),
            ));
        }
        Ok(())
    }

    /// Save a memory tagged with this session.
    pub fn save(&self, memory: &Memory) -> Result<()> {
        self.require_session()?;

        let mut key = sanitize_key(&memory.key);
        validate_key(&key)?;
        key = self.disambiguate_key(key)?;

        let now = now_rfc3339();
        let content = sanitize_content(&memory.content);

        let (created_at, relevance_score) = match self.store.load(&key) {
            Ok(existing) if existing.source_session == self.session_id => (
                existing.created_at,
                memory.relevance_score.max(existing.relevance_score),
            ),
            Err(PawanError::NotFound(_)) => (now.clone(), memory.relevance_score),
            Ok(_) => {
                return Err(PawanError::Tool(
                    "Memory key conflict after disambiguation; refusing to clobber a foreign session"
                        .to_string(),
                ));
            }
            Err(e) => return Err(e),
        };

        let to_store = Memory {
            key,
            content,
            source_session: self.session_id.clone(),
            created_at,
            updated_at: now,
            relevance_score,
        };

        self.store.save(&to_store)
    }

    /// Only return memories from this session (or shared cross-session knowledge).
    pub fn get_relevant(&self, query: &str, limit: usize) -> Result<Vec<Memory>> {
        self.require_session()?;
        if limit == 0 {
            return Ok(vec![]);
        }

        // Pull a larger candidate pool, then apply the session fence.
        let pool = limit.saturating_mul(8).clamp(32, 2000);
        let mut hits: Vec<Memory> = self
            .store
            .search(query, pool)?
            .into_iter()
            .filter(|m| m.source_session == self.session_id || m.is_shared())
            .collect();

        let mut seen: HashSet<String> = hits.iter().map(|m| m.key.clone()).collect();
        if let Ok(keys) = self.store.list() {
            for k in keys {
                if seen.contains(&k) {
                    continue;
                }
                if let Ok(m) = self.store.load(&k) {
                    if m.is_shared() {
                        seen.insert(m.key.clone());
                        hits.push(m);
                    }
                }
            }
        }

        hits.sort_by(|a, b| {
            let s = b
                .relevance_score
                .partial_cmp(&a.relevance_score)
                .unwrap_or(Ordering::Equal);
            if s != Ordering::Equal {
                return s;
            }
            b.updated_at.cmp(&a.updated_at)
        });
        hits.truncate(limit);
        Ok(hits)
    }

    /// Remove session-local memories; shared memories are retained for other sessions.
    pub fn cleanup_session(&self) -> Result<()> {
        self.require_session()?;
        if !self.store.base_path.exists() {
            return Ok(());
        }

        for entry in std::fs::read_dir(&self.store.base_path)? {
            let entry = entry?;
            let path = entry.path();
            if path.extension().and_then(|s| s.to_str()) != Some("json") {
                continue;
            }
            let bytes = match std::fs::read(&path) {
                Ok(b) => b,
                Err(_) => continue,
            };
            let mem: Memory = match serde_json::from_slice(&bytes) {
                Ok(m) => m,
                Err(_) => continue,
            };
            if mem.source_session == self.session_id && !mem.is_shared() {
                self.store.delete(&mem.key)?;
            }
        }
        Ok(())
    }

    fn disambiguate_key(&self, base: String) -> Result<String> {
        let original = base.clone();
        let mut candidate = base;
        let mut n = 0u32;

        loop {
            match self.store.load(&candidate) {
                Ok(existing) if existing.source_session == self.session_id => {
                    return Ok(candidate);
                }
                Ok(_other) => {
                    n += 1;
                    let suffix = format!("__{n}");
                    let max_base = MAX_KEY_CHARS.saturating_sub(suffix.chars().count());
                    if max_base == 0 {
                        return Err(PawanError::Tool(
                            "Could not reserve space for a disambiguation suffix on the memory key"
                                .to_string(),
                        ));
                    }
                    let truncated = truncate_to_max_chars(&original, max_base);
                    candidate = format!("{truncated}{suffix}");
                }
                Err(PawanError::NotFound(_)) => return Ok(candidate),
                Err(e) => return Err(e),
            }
        }
    }
}

/// Sanitize a string to prevent injection in memory keys: keep alnum, dash, underscore, dot.
pub fn sanitize_key(s: &str) -> String {
    s.chars()
        .filter(|ch| ch.is_ascii_alphanumeric() || *ch == '-' || *ch == '_' || *ch == '.')
        .collect()
}

/// Sanitize a memory content string: strip NULs and cap size at 1MB (byte length).
pub fn sanitize_content(s: &str) -> String {
    let no_nul: String = s.chars().filter(|&c| c != '\0').collect();
    truncate_to_max_bytes(&no_nul, MAX_CONTENT_BYTES)
}

/// Validate that a memory key is safe for filesystem use.
pub fn validate_key(key: &str) -> Result<()> {
    if key.is_empty() {
        return Err(PawanError::Tool(
            "Memory key is empty (or became empty after sanitization)".to_string(),
        ));
    }
    if key.chars().count() > MAX_KEY_CHARS {
        return Err(PawanError::Tool(format!(
            "Memory key exceeds {MAX_KEY_CHARS} characters"
        )));
    }
    if !key
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' || ch == '.')
    {
        return Err(PawanError::Tool(
            "Memory key contains disallowed characters (allowed: A-Z, a-z, 0-9, -, _, .)"
                .to_string(),
        ));
    }
    Ok(())
}

fn truncate_to_max_bytes(s: &str, max: usize) -> String {
    if s.len() <= max {
        return s.to_string();
    }
    let mut end = max;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    s[..end].to_string()
}

fn truncate_to_max_chars(s: &str, max_chars: usize) -> String {
    if s.chars().count() <= max_chars {
        return s.to_string();
    }
    s.chars().take(max_chars).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn sanitize_strips_unsafe_key_chars() {
        assert_eq!(sanitize_key("a/b@x#y"), "abxy");
        assert_eq!(sanitize_key("arch.module-name"), "arch.module-name");
    }

    #[test]
    fn validate_key_rejects_bad_keys() {
        assert!(validate_key("").is_err());
        assert!(validate_key("bad/key").is_err());
        let long: String = "a".repeat(MAX_KEY_CHARS + 1);
        assert!(validate_key(&long).is_err());
    }

    #[test]
    fn sanitize_content_strips_nul_and_truncates() {
        let s = "a\0b".repeat(MAX_CONTENT_BYTES);
        let out = sanitize_content(&s);
        assert!(!out.contains('\0'));
        assert!(out.len() <= MAX_CONTENT_BYTES);
    }

    #[test]
    fn session_fence_filters_foreign_session() {
        let dir = TempDir::new().unwrap();
        let store = MemoryStore::new(dir.path().join("memories"));

        let mem_a = Memory {
            key: "note.a".to_string(),
            content: "local debug for session A".to_string(),
            source_session: "sess-a".to_string(),
            created_at: now_rfc3339(),
            updated_at: now_rfc3339(),
            relevance_score: 1.0,
        };
        let mem_b = Memory {
            key: "note.b".to_string(),
            content: "Architecture decision: use modules".to_string(),
            source_session: "sess-b".to_string(),
            created_at: now_rfc3339(),
            updated_at: now_rfc3339(),
            relevance_score: 1.0,
        };
        let mem_c = Memory {
            key: "note.c".to_string(),
            content: "Private session B debug scratchpad".to_string(),
            source_session: "sess-b".to_string(),
            created_at: now_rfc3339(),
            updated_at: now_rfc3339(),
            relevance_score: 1.0,
        };
        store.save(&mem_a).unwrap();
        store.save(&mem_b).unwrap();
        store.save(&mem_c).unwrap();

        let scoped = SessionScopedMemory::new(store, "sess-a".to_string());
        let found = scoped.get_relevant("debug", 10).unwrap();
        let keys: Vec<_> = found.iter().map(|m| m.key.as_str()).collect();
        assert!(keys.contains(&"note.a"));
        assert!(keys.contains(&"note.b"));
        assert!(!keys.contains(&"note.c"));
    }

    #[test]
    fn test_session_scoped_memory_requires_non_empty_session_id() {
        let dir = TempDir::new().unwrap();
        let store = MemoryStore::new(dir.path().join("memories"));
        let scoped = SessionScopedMemory::new(store, String::new());
        let m = Memory {
            key: "k".to_string(),
            content: "c".to_string(),
            source_session: String::new(),
            created_at: now_rfc3339(),
            updated_at: now_rfc3339(),
            relevance_score: 0.1,
        };
        assert!(scoped.save(&m).is_err());
    }

    #[test]
    fn test_get_relevant_empty_query_returns_empty() {
        let dir = TempDir::new().unwrap();
        let store = MemoryStore::new(dir.path().join("memories"));
        let scoped = SessionScopedMemory::new(store, "s".to_string());
        let out = scoped.get_relevant("   ", 10).unwrap();
        assert!(out.is_empty());
    }

    #[test]
    fn test_sanitize_and_validate_key_edge_cases() {
        assert_eq!(sanitize_key(""), "");
        assert_eq!(sanitize_key("a@b"), "ab");
        assert!(validate_key("valid.key-1_").is_ok());
        let empty_content = sanitize_content("");
        assert!(empty_content.is_empty());
        let big = "x".repeat(MAX_CONTENT_BYTES + 10_000);
        let capped = sanitize_content(&big);
        assert!(capped.len() <= MAX_CONTENT_BYTES);
    }
}
