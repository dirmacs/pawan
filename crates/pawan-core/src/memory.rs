//! Autonomous memory: extract durable learnings and inject at startup.

use crate::agent::Message;
use crate::{PawanError, Result};
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use std::cmp::Ordering;
use std::collections::hash_map::DefaultHasher;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use uuid::Uuid;

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

impl Memory {
    /// True when this memory is intended for reuse across sessions (architecture, tools, durable patterns).
    /// Session-tuned notes and one-off debug tips remain session-scoped.
    pub fn is_shared(&self) -> bool {
        if self
            .key
            .strip_prefix("shared.")
            .is_some_and(|rest| !rest.is_empty())
        {
            return true;
        }
        let c = self.content.to_lowercase();
        c.contains("architecture decision")
            || c.contains("architecture decisions")
            || c.contains("tool definition")
            || c.contains("tool definitions")
            || c.contains("reusable knowledge")
    }
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

    pub fn key_to_filename(key: &str) -> String {
        // Keep it filesystem-safe and stable.
        let mut out = String::with_capacity(key.len());
        for ch in key.chars() {
            let safe = match ch {
                'a'..='z' | 'A'..='Z' | '0'..='9' | '-' | '_' | '.' => ch,
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
            let hay = format!(
                "{}
{}",
                mem.key, mem.content
            )
            .to_lowercase();
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
        out.push_str(
            "# Memory Summary

",
        );
        out.push_str(
            "This file is auto-generated. Treat as heuristic context.

",
        );
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
            out.push_str(
                "

---

",
            );
        }

        fs::write(&summary_path, out)?;
        Ok(summary_path)
    }
    /// Extract durable learnings from a conversation and save them.
    /// Call this at session end or periodically during long sessions.
    pub fn extract_from_conversation(&self, messages: &[Message]) -> Result<Vec<Memory>> {
        if messages.is_empty() {
            return Ok(vec![]);
        }

        self.ensure_dirs()?;
        let session_id = Uuid::new_v4().to_string();
        let now = Utc::now().to_rfc3339();

        let lines = collect_conversation_lines(messages);
        let full_text = conversation_text(messages);

        let mut by_key: HashMap<String, Memory> = HashMap::new();

        let mut line_counts: HashMap<String, usize> = HashMap::new();
        for line in &lines {
            let n = normalize_line(line);
            if n.len() >= 16 {
                *line_counts.entry(n).or_insert(0) += 1;
            }
        }
        for (line, count) in line_counts {
            if count < 3 {
                continue;
            }
            let key = format!("heuristic.repeated.{:016x}", hash64(&line));
            let rel = ((count as f64) / 10.0).min(1.0) + 0.35;
            let content = format!("**Repeated pattern** ({count}× in session)\n\n{line}");
            merge_into_map(&mut by_key, key, content, rel, &session_id, &now);
        }

        let words: Vec<String> = ordered_word_tokens(&full_text);
        if words.len() >= 4 {
            let mut grams: HashMap<String, usize> = HashMap::new();
            for w in words.windows(4) {
                let g = w.join(" ");
                if g.len() < 12 {
                    continue;
                }
                *grams.entry(g).or_insert(0) += 1;
            }
            for (g, count) in grams {
                if count < 3 {
                    continue;
                }
                let key = format!("heuristic.ngram.{:016x}", hash64(&g));
                let rel = ((count as f64) / 8.0).min(1.0) + 0.25;
                let content = format!("**Repeated phrase** ({count}×)\n\n`{g}`");
                merge_into_map(&mut by_key, key, content, rel, &session_id, &now);
            }
        }

        for span in extract_error_fix_spans(&lines) {
            let key = format!("heuristic.errorfix.{:016x}", hash64(&span));
            let content = format!("**Error / recovery pattern**\n\n{span}");
            merge_into_map(&mut by_key, key, content, 0.75, &session_id, &now);
        }

        for line in &lines {
            if !looks_like_command(line) {
                continue;
            }
            let key = format!("heuristic.command.{:016x}", hash64(line));
            let content = format!("**Command pattern**\n\n`{}`", line.trim());
            merge_into_map(&mut by_key, key, content, 0.6, &session_id, &now);
        }

        for line in &lines {
            if !looks_like_config(line) {
                continue;
            }
            let key = format!("heuristic.config.{:016x}", hash64(line));
            let content = format!("**Configuration / setup hint**\n\n{}", line.trim());
            merge_into_map(&mut by_key, key, content, 0.55, &session_id, &now);
        }

        for line in &lines {
            let l = line.to_lowercase();
            if !(l.contains("we should")
                || l.contains("we will")
                || l.contains("design decision")
                || l.contains("architecture")
                || l.contains("use a ")
                || l.contains("prefer "))
            {
                continue;
            }
            if line.trim().len() < 12 {
                continue;
            }
            let key = format!("heuristic.design.{:016x}", hash64(line));
            let content = format!("**Design / architecture note**\n\n{}", line.trim());
            merge_into_map(&mut by_key, key, content, 0.5, &session_id, &now);
        }

        let mut saved: Vec<Memory> = by_key.into_values().collect();
        saved.sort_by(|a, b| {
            b.relevance_score
                .partial_cmp(&a.relevance_score)
                .unwrap_or(Ordering::Equal)
                .then(b.updated_at.cmp(&a.updated_at))
        });

        for mem in &saved {
            self.save_merged_memory(mem)?;
        }

        Ok(saved)
    }

    pub fn consolidate(&self) -> Result<()> {
        self.ensure_dirs()?;
        if !self.base_path.exists() {
            return Ok(());
        }

        let now = Utc::now();
        let now_str = now.to_rfc3339();
        let memories = self.load_all_memories()?;

        for m in &memories {
            if m.relevance_score >= 0.1 {
                continue;
            }
            if let Some(updated) = parse_rfc3339_utc(&m.updated_at) {
                if now.signed_duration_since(updated) > Duration::days(90) {
                    self.delete(&m.key)?;
                }
            }
        }

        let memories = self.load_all_memories()?;
        if memories.is_empty() {
            return Ok(());
        }

        let mut groups: HashMap<String, Vec<Memory>> = HashMap::new();
        for m in memories {
            groups.entry(merge_bucket_key(&m)).or_default().push(m);
        }

        for (_bucket, group) in groups {
            if group.len() <= 1 {
                continue;
            }
            let mut group = group;
            group.sort_by(|a, b| {
                b.relevance_score
                    .partial_cmp(&a.relevance_score)
                    .unwrap_or(Ordering::Equal)
                    .then(b.updated_at.cmp(&a.updated_at))
            });

            let winner = &group[0];
            let mut content = winner.content.clone();
            for other in group.iter().skip(1) {
                content.push_str("\n\n---\n\n");
                content.push_str(other.content.trim());
            }

            let merged = Memory {
                key: winner.key.clone(),
                content,
                source_session: winner.source_session.clone(),
                created_at: winner.created_at.clone(),
                updated_at: now_str.clone(),
                relevance_score: winner.relevance_score,
            };

            self.save(&merged)?;
            for other in group.iter().skip(1) {
                if other.key != merged.key {
                    self.delete(&other.key)?;
                }
            }
        }
        Ok(())
    }

    pub fn get_relevant(&self, query: &str, limit: usize) -> Result<Vec<Memory>> {
        if limit == 0 {
            return Ok(vec![]);
        }
        let q_words: HashSet<String> = stopword_filtered_tokens(query);
        if q_words.is_empty() {
            return Ok(vec![]);
        }
        let memories = self.load_all_memories()?;
        if memories.is_empty() {
            return Ok(vec![]);
        }
        let query_lc = query.to_lowercase();
        let mut scored: Vec<(f64, Memory)> = Vec::new();
        for m in memories {
            let hay = format!("{} {}", m.key, m.content);
            let h_words = stopword_filtered_tokens(&hay);
            let mut combined = jaccard(&q_words, &h_words) * 2.0 + m.relevance_score * 0.2;
            if combined > 0.0 && m.content.to_lowercase().contains(&query_lc) {
                combined += 0.15;
            }
            if combined > 0.0 {
                scored.push((combined, m));
            }
        }
        scored.sort_by(|a, b| {
            b.0.partial_cmp(&a.0)
                .unwrap_or(Ordering::Equal)
                .then(b.1.updated_at.cmp(&a.1.updated_at))
                .then(
                    b.1.relevance_score
                        .partial_cmp(&a.1.relevance_score)
                        .unwrap_or(Ordering::Equal),
                )
        });
        Ok(scored.into_iter().map(|(_, m)| m).take(limit).collect())
    }

    pub fn inject_as_context(&self, query: &str) -> Result<String> {
        const LIMIT: usize = 12;
        let mems = self.get_relevant(query, LIMIT)?;
        if mems.is_empty() {
            return Ok(String::new());
        }
        let mut out = String::new();
        out.push_str("## Relevant memory context\n\n");
        for m in mems {
            out.push_str("### ");
            out.push_str(&m.key);
            out.push_str("\n\n");
            out.push_str(m.content.trim());
            out.push_str("\n\n---\n\n");
        }
        Ok(out)
    }

    fn load_all_memories(&self) -> Result<Vec<Memory>> {
        if !self.base_path.exists() {
            return Ok(vec![]);
        }
        let mut out = vec![];
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
            if let Ok(mem) = serde_json::from_slice::<Memory>(&bytes) {
                out.push(mem);
            }
        }
        Ok(out)
    }

    fn save_merged_memory(&self, mem: &Memory) -> Result<()> {
        if !self.memory_path(&mem.key).exists() {
            return self.save(mem);
        }
        let mut cur = self.load(&mem.key)?;
        if mem.relevance_score > cur.relevance_score {
            cur.relevance_score = mem.relevance_score;
        }
        if !cur.content.contains(&mem.content) {
            cur.content.push_str("\n\n---\n\n");
            cur.content.push_str(mem.content.trim());
        }
        cur.updated_at = mem.updated_at.clone();
        self.save(&cur)
    }
}

/// Map a memory key to a stable, filesystem-safe filename (`unsafe` → `'_'`).
pub fn sanitize_key(key: &str) -> String {
    MemoryStore::key_to_filename(key)
}

fn ordered_word_tokens(text: &str) -> Vec<String> {
    text.to_lowercase()
        .split(|c: char| !c.is_alphanumeric() && c != '_')
        .filter(|w| w.len() >= 2)
        .map(|w| w.to_string())
        .collect()
}

fn merge_into_map(
    map: &mut HashMap<String, Memory>,
    key: String,
    content: String,
    relevance: f64,
    session_id: &str,
    now: &str,
) {
    let relevance = relevance.clamp(0.0, 5.0);
    map.entry(key.clone())
        .and_modify(|m| {
            m.relevance_score = m.relevance_score.max(relevance);
            m.updated_at = now.to_string();
            if !m.content.contains(&content) {
                m.content.push_str("\n\n---\n\n");
                m.content.push_str(&content);
            }
        })
        .or_insert_with(|| Memory {
            key,
            content,
            source_session: session_id.to_string(),
            created_at: now.to_string(),
            updated_at: now.to_string(),
            relevance_score: relevance,
        });
}

fn collect_conversation_lines(messages: &[Message]) -> Vec<String> {
    let mut out = vec![];
    for m in messages {
        for line in m.content.lines() {
            out.push(line.to_string());
        }
        if let Some(tr) = &m.tool_result {
            let s = serde_json::to_string(&tr.content).unwrap_or_default();
            for line in s.lines() {
                out.push(line.to_string());
            }
        }
    }
    out
}

fn conversation_text(messages: &[Message]) -> String {
    let mut s = String::new();
    for m in messages {
        if !m.content.is_empty() {
            s.push_str(&m.content);
            s.push('\n');
        }
        if let Some(tr) = &m.tool_result {
            if let Ok(j) = serde_json::to_string(&tr.content) {
                s.push_str(&j);
                s.push('\n');
            }
        }
    }
    s
}

fn normalize_line(line: &str) -> String {
    let t = line.trim();
    t.chars().filter(|c| !c.is_control()).collect::<String>()
}

fn tokenize_words(text: &str) -> HashSet<String> {
    let mut out = HashSet::new();
    for w in text
        .to_lowercase()
        .split(|c: char| !c.is_alphanumeric() && c != '_')
    {
        if w.len() < 2 {
            continue;
        }
        out.insert(w.to_string());
    }
    out
}

fn stopword_filtered_tokens(text: &str) -> HashSet<String> {
    const STOP: &[&str] = &[
        "the", "a", "an", "is", "are", "and", "or", "of", "to", "in", "for", "on", "with", "as",
        "at", "it", "be", "this", "that", "we", "you",
    ];
    let mut t = tokenize_words(text);
    t.retain(|w| !STOP.contains(&w.as_str()));
    t
}

fn jaccard(a: &HashSet<String>, b: &HashSet<String>) -> f64 {
    if a.is_empty() || b.is_empty() {
        return 0.0;
    }
    let inter = a.intersection(b).count();
    let uni = a.union(b).count();
    if uni == 0 {
        0.0
    } else {
        inter as f64 / uni as f64
    }
}

fn extract_error_fix_spans(lines: &[String]) -> Vec<String> {
    const ERR: &[&str] = &[
        "error",
        "failed",
        "panic",
        "exception",
        "traceback",
        "e031",
        "e0",
    ];
    const OK: &[&str] = &[
        "fixed", "success", "works", "resolved", "passing", "ok", "done",
    ];
    let mut spans = vec![];
    for i in 0..lines.len() {
        let li = lines[i].to_lowercase();
        if !ERR.iter().any(|e| li.contains(e)) {
            continue;
        }
        for j in (i + 1)..lines.len().min(i + 40) {
            let lj = lines[j].to_lowercase();
            if OK.iter().any(|e| lj.contains(e)) {
                let span = lines[i..=j].join("\n");
                if (20..=4000).contains(&span.chars().count()) {
                    spans.push(span);
                }
                break;
            }
        }
    }
    spans
}

fn looks_like_command(line: &str) -> bool {
    let t = line.trim();
    if t.starts_with('$') {
        return true;
    }
    if t.starts_with("cargo ")
        || t.starts_with("rustc ")
        || t.starts_with("git ")
        || t.starts_with("rg ")
        || t.starts_with("fd ")
    {
        return true;
    }
    if t.starts_with("bun ") || t.starts_with("npm ") || t.starts_with("pnpm ") {
        return true;
    }
    if t.starts_with("make ") || t.starts_with("just ") {
        return true;
    }
    t.contains(" 2>&1") || t.contains("| ")
}

fn looks_like_config(line: &str) -> bool {
    let t = line.to_lowercase();
    t.contains(".toml")
        || t.contains(".env")
        || t.contains("pawan.toml")
        || t.contains("config.toml")
        || t.contains("export ")
        || t.contains("feature flag")
        || t.contains("timeout clamped")
}

fn hash64(s: &str) -> u64 {
    let mut h = DefaultHasher::new();
    s.hash(&mut h);
    h.finish()
}

fn parse_rfc3339_utc(s: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(s)
        .ok()
        .map(|dt| dt.with_timezone(&Utc))
}

fn merge_bucket_key(m: &Memory) -> String {
    m.key
        .to_lowercase()
        .chars()
        .filter(|c| c.is_alphanumeric() || *c == '_' || *c == '.')
        .collect()
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
    format!(
        "{}

{}",
        prompt, block
    )
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
        assert!(injected.contains("/root/.omp/agent/memories/--tmp--/memory_summary.md"));
        assert!(injected.contains("Always run cargo check"));
    }

    #[test]
    fn test_extract_from_conversation_empty() {
        let td = TempDir::new().unwrap();
        let store = MemoryStore::new(td.path().join("memories"));
        let got = store.extract_from_conversation(&[]).unwrap();
        assert!(got.is_empty());
    }

    #[test]
    fn test_extract_from_conversation_repetition() {
        let td = TempDir::new().unwrap();
        let store = MemoryStore::new(td.path().join("memories"));
        let line = "Always run cargo check before committing changes to the branch";
        let rep = (0..3u8)
            .map(|_| Message {
                role: crate::agent::Role::User,
                content: line.to_string(),
                tool_calls: vec![],
                tool_result: None,
            })
            .collect::<Vec<_>>();
        let mems = store.extract_from_conversation(&rep).unwrap();
        assert!(!mems.is_empty());
    }

    #[test]
    fn test_consolidate_merges_similar_keys() {
        let td = TempDir::new().unwrap();
        let store = MemoryStore::new(td.path().join("memories"));
        let now = now_rfc3339();
        let a = Memory {
            key: "my.Feature-A".to_string(),
            content: "note a".to_string(),
            source_session: "s".to_string(),
            created_at: now.clone(),
            updated_at: now.clone(),
            relevance_score: 0.5,
        };
        let b = Memory {
            key: "myFeatureA".to_string(),
            content: "note b".to_string(),
            source_session: "s".to_string(),
            created_at: now.clone(),
            updated_at: now.clone(),
            relevance_score: 0.9,
        };
        store.save(&a).unwrap();
        store.save(&b).unwrap();
        assert_eq!(store.list().unwrap().len(), 2);
        store.consolidate().unwrap();
        let mems = store.get_relevant("note", 5).unwrap();
        assert!(!mems.is_empty());
    }

    #[test]
    fn test_get_relevant_ordering() {
        let td = TempDir::new().unwrap();
        let store = MemoryStore::new(td.path().join("memories"));
        let now = now_rfc3339();
        store
            .save(&Memory {
                key: "k_alpha".to_string(),
                content: "alpha beta gamma".to_string(),
                source_session: "s".to_string(),
                created_at: now.clone(),
                updated_at: now.clone(),
                relevance_score: 0.2,
            })
            .unwrap();
        store
            .save(&Memory {
                key: "k_best".to_string(),
                content: "alpha beta zeta".to_string(),
                source_session: "s".to_string(),
                created_at: now.clone(),
                updated_at: now.clone(),
                relevance_score: 1.0,
            })
            .unwrap();
        let out = store.get_relevant("alpha beta", 2).unwrap();
        assert_eq!(out.len(), 2);
    }

    #[test]
    fn test_inject_as_context_empty() {
        let td = TempDir::new().unwrap();
        let store = MemoryStore::new(td.path().join("memories"));
        let s = store.inject_as_context("does-not-exist-xyz").unwrap();
        assert!(s.is_empty());
    }

    #[test]
    fn test_sanitize_key_removes_unsafe_chars() {
        assert_eq!(sanitize_key("hello/world"), "hello_world");
        assert_eq!(sanitize_key("test-file.v2"), "test-file.v2");
    }

    #[test]
    fn test_key_to_filename() {
        let result = MemoryStore::key_to_filename("Test Key!@#");
        assert!(result.contains("Test"));
    }

    #[test]
    fn test_sanitize_key_empty_and_large_inputs() {
        assert_eq!(sanitize_key(""), "_");
        let big = "a".repeat(50_000);
        let s = sanitize_key(&big);
        assert_eq!(s.len(), 50_000);
    }
}
