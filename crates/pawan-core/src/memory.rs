//! Autonomous memory: extract durable learnings and inject at startup.

use crate::{PawanError, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::cmp::Ordering;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Memory {
    /// Storage key (unique)
    pub key: String,
    /// Markdown content
    pub content: String,
    pub source_session: String,
    pub created_at: String,
    pub updated_at: String,
    pub relevance_score: f64,
}

#[derive(Debug, Clone)]
pub struct MemoryStore {
    /// Base path (default: ~/.pawan/memories/)
    pub base_path: PathBuf,
}

impl MemoryStore {
    pub fn new(base_path: PathBuf) -> Self {
        Self { base_path }
    }

    pub fn default_path() -> Result<PathBuf> {
        let home = dirs::home_dir().ok_or_else(|| {
            PawanError::Config("Failed to resolve home directory for memory store".to_string())
        })?;
        Ok(home.join(".pawan").join("memories"))
    }

    pub fn new_default() -> Result<Self> {
        Ok(Self::new(Self::default_path()?))
    }

    fn ensure_dirs(&self) -> Result<()> {
        fs::create_dir_all(&self.base_path)?;
        Ok(())
    }

    fn key_to_filename(key: &str) -> String {
        // Keep it filesystem-safe and stable.
        let mut out = String::with_capacity(key.len());
        for ch in key.chars() {
            let safe = match ch {
                ('a'..='z' | 'A'..='Z' | '0'..='9' | '-' | '_' | '.') => ch,
                _ => '_',
            };
            out.push(safe);
        }
        if out.is_empty() {
            "_".to_string()
        } else {
            out
        }
    }

    fn memory_path(&self, key: &str) -> PathBuf {
        self.base_path
            .join(format!("{}.json", Self::key_to_filename(key)))
    }

    pub fn save(&self, memory: &Memory) -> Result<()> {
        self.ensure_dirs()?;

        let path = self.memory_path(&memory.key);
        let tmp = path.with_extension("json.tmp");
        let payload = serde_json::to_vec_pretty(memory)
            .map_err(|e| PawanError::Parse(format!("Failed to serialize memory: {e}")))?;

        fs::write(&tmp, payload)?;
        fs::rename(&tmp, &path)?;

        self.evict_fifo(100)?;
        Ok(())
    }

    pub fn load(&self, key: &str) -> Result<Memory> {
        let path = self.memory_path(key);
        let bytes = fs::read(&path).map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                PawanError::NotFound(format!("Memory not found: {key}"))
            } else {
                PawanError::Io(e)
            }
        })?;
        serde_json::from_slice::<Memory>(&bytes)
            .map_err(|e| PawanError::Parse(format!("Failed to parse memory JSON: {e}")))
    }

    pub fn list(&self) -> Result<Vec<String>> {
        if !self.base_path.exists() {
            return Ok(vec![]);
        }
        let mut keys = vec![];
        for entry in fs::read_dir(&self.base_path)? {
            let entry = entry?;
            let path = entry.path();
            if path.extension().and_then(|s| s.to_str()) != Some("json") {
                continue;
            }
            if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
                keys.push(stem.to_string());
            }
        }
        keys.sort();
        Ok(keys)
    }

    pub fn delete(&self, key: &str) -> Result<()> {
        let path = self.memory_path(key);
        match fs::remove_file(&path) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(PawanError::Io(e)),
        }
    }

    pub fn search(&self, query: &str, limit: usize) -> Result<Vec<Memory>> {
        if limit == 0 {
            return Ok(vec![]);
        }
        let q = query.to_lowercase();
        let terms: Vec<&str> = q.split_whitespace().filter(|t| !t.is_empty()).collect();
        if terms.is_empty() {
            return Ok(vec![]);
        }

        let mut hits: Vec<Memory> = vec![];
        if !self.base_path.exists() {
            return Ok(vec![]);
        }
        for entry in fs::read_dir(&self.base_path)? {
            let entry = entry?;
            let path = entry.path();
            if path.extension().and_then(|s| s.to_str()) != Some("json") {
                continue;
            }
            let bytes = match fs::read(&path) {
                Ok(b) => b,
                Err(_) => continue,
            };
            let mut mem: Memory = match serde_json::from_slice(&bytes) {
                Ok(m) => m,
                Err(_) => continue,
            };
            let hay = format!("{}
{}", mem.key, mem.content).to_lowercase();
            let mut score = 0.0f64;
            for t in &terms {
                if t.len() < 2 {
                    continue;
                }
                let mut idx = 0;
                while let Some(pos) = hay[idx..].find(t) {
                    score += 1.0;
                    idx += pos + t.len();
                    if idx >= hay.len() {
                        break;
                    }
                }
            }
            if score > 0.0 {
                mem.relevance_score = score;
                hits.push(mem);
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

    fn evict_fifo(&self, keep_last: usize) -> Result<()> {
        if !self.base_path.exists() {
            return Ok(());
        }

        let mut entries: Vec<(std::time::SystemTime, PathBuf)> = vec![];
        for entry in fs::read_dir(&self.base_path)? {
            let entry = entry?;
            let path = entry.path();
            if path.extension().and_then(|s| s.to_str()) != Some("json") {
                continue;
            }
            let meta = match entry.metadata() {
                Ok(m) => m,
                Err(_) => continue,
            };
            let mtime = meta.modified().unwrap_or(std::time::SystemTime::UNIX_EPOCH);
            entries.push((mtime, path));
        }

        if entries.len() <= keep_last {
            return Ok(());
        }

        entries.sort_by(|a, b| a.0.cmp(&b.0));
        let to_delete = entries.len().saturating_sub(keep_last);
        for i in 0..to_delete {
            let _ = fs::remove_file(&entries[i].1);
        }
        Ok(())
    }

    pub fn write_memory_summary_markdown(&self, top: &[Memory]) -> Result<PathBuf> {
        self.ensure_dirs()?;
        let summary_path = self.base_path.join("memory_summary.md");

        if top.is_empty() {
            let _ = fs::remove_file(&summary_path);
            return Ok(summary_path);
        }

        let mut out = String::new();
        out.push_str("# Memory Summary

");
        out.push_str("This file is auto-generated. Treat as heuristic context.

");
        for m in top {
            out.push_str("## ");
            out.push_str(&m.key);
            out.push('\n');
            out.push_str("- source_session: ");
            out.push_str(&m.source_session);
            out.push('\n');
            out.push_str("- updated_at: ");
            out.push_str(&m.updated_at);
            out.push('\n');
            out.push_str("- relevance_score: ");
            out.push_str(&format!("{:.2}", m.relevance_score));
            out.push('\n');
            out.push('\n');
            out.push_str(m.content.trim());
            out.push_str("

---

");
        }

        fs::write(&summary_path, out)?;
        Ok(summary_path)
    }
}

/// Build the memory guidance block to inject into the system prompt.
///
/// If the memory summary does not exist (or is empty), returns None.
pub fn load_memory_guidance_block(store: &MemoryStore) -> Option<String> {
    let path = store.base_path.join("memory_summary.md");
    let text = fs::read_to_string(&path).ok()?;
    if text.trim().is_empty() {
        return None;
    }

    Some(format!(
        "## Memory Guidance

Preloaded memory resource: /root/.omp/agent/memories/--tmp--/memory_summary.md

{}",
        text.trim()
    ))
}

/// Inject memory guidance into an existing system prompt.
///
/// No-op when memory summary is missing/empty.
pub fn inject_memory_guidance_into_prompt(prompt: String, store: &MemoryStore) -> String {
    let Some(block) = load_memory_guidance_block(store) else {
        return prompt;
    };
    format!("{}

{}", prompt, block)
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ExtractedMemoryItem {
    pub title: String,
    pub markdown: String,
    #[serde(default)]
    pub relevance_score: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MemoryExtraction {
    pub items: Vec<ExtractedMemoryItem>,
}

pub fn parse_memory_extraction_json(s: &str) -> Result<MemoryExtraction> {
    serde_json::from_str::<MemoryExtraction>(s)
        .map_err(|e| PawanError::Parse(format!("Failed to parse memory extraction JSON: {e}")))
}

pub fn memories_from_extraction(session_id: &str, extraction: MemoryExtraction) -> Vec<Memory> {
    let now = Utc::now().to_rfc3339();
    extraction
        .items
        .into_iter()
        .enumerate()
        .map(|(i, item)| Memory {
            key: format!("session_{}_extract_{}", session_id, i),
            content: item.markdown,
            source_session: session_id.to_string(),
            created_at: now.clone(),
            updated_at: now.clone(),
            relevance_score: item.relevance_score.unwrap_or(1.0),
        })
        .collect()
}

pub fn session_extract_key(session_id: &str) -> String {
    format!("session_{}_extract", session_id)
}

pub fn make_session_extract_memory(session_id: &str, markdown: String) -> Memory {
    let now = Utc::now().to_rfc3339();
    Memory {
        key: session_extract_key(session_id),
        content: markdown,
        source_session: session_id.to_string(),
        created_at: now.clone(),
        updated_at: now,
        relevance_score: 1.0,
    }
}

pub fn now_rfc3339() -> String {
    DateTime::<Utc>::from(Utc::now()).to_rfc3339()
}

pub fn is_empty_or_ws(s: &str) -> bool {
    s.trim().is_empty()
}

pub fn ensure_parent_dir(path: &Path) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_extract_memories_from_synthetic_session_json() {
        let json = r#"{
  "items": [
    { "title": "Decision", "markdown": "- Use flat files for MVP", "relevance_score": 3.0 },
    { "title": "Pitfall", "markdown": "- Avoid injecting stale memory without repo checks" }
  ]
}"#;

        let extraction = parse_memory_extraction_json(json).unwrap();
        assert_eq!(extraction.items.len(), 2);

        let mems = memories_from_extraction("abc123", extraction);
        assert_eq!(mems.len(), 2);
        assert!(mems[0].key.contains("session_abc123_extract_0"));
        assert_eq!(mems[0].relevance_score, 3.0);
        assert_eq!(mems[1].relevance_score, 1.0);
    }

    #[test]
    fn test_inject_memories_into_new_session_prompt() {
        let td = TempDir::new().unwrap();
        let store = MemoryStore::new(td.path().join("memories"));

        let m = Memory {
            key: "k1".to_string(),
            content: "- Always run cargo check".to_string(),
            source_session: "s1".to_string(),
            created_at: now_rfc3339(),
            updated_at: now_rfc3339(),
            relevance_score: 5.0,
        };
        store.save(&m).unwrap();

        let top = store.search("cargo", 5).unwrap();
        store.write_memory_summary_markdown(&top).unwrap();

        let base = "You are pawan.".to_string();
        let injected = inject_memory_guidance_into_prompt(base, &store);
        assert!(injected.contains("## Memory Guidance"));
        assert!(injected.contains(/root/.omp/agent/memories/--tmp--/memory_summary.md));
        assert!(injected.contains("Always run cargo check"));
    }
}
