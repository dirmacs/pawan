//! Agent definition loading and parsing (YAML frontmatter + Markdown body).
//!
//! File format:
//! - Optional frontmatter delimited by `---` lines at the start of the file
//! - Markdown body after the closing delimiter becomes the agent system prompt
//!
//! We intentionally support a small, JSON-compatible subset of YAML in frontmatter
//! to avoid adding a hard dependency on `serde_yaml`.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use tracing::warn;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThinkingLevel {
    Minimal,
    Low,
    Medium,
    High,
}

impl ThinkingLevel {
    fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "minimal" => Some(Self::Minimal),
            "low" => Some(Self::Low),
            "medium" => Some(Self::Medium),
            "high" => Some(Self::High),
            _ => None,
        }
    }
}

impl Default for ThinkingLevel {
    fn default() -> Self {
        Self::Minimal
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentDefinition {
    pub name: String,                  // "explore", "plan", etc.
    pub description: String,           // what this agent does
    pub tools: Vec<String>,            // allowed tool names (or ["*"] for all)
    pub model_pattern: String,         // model filter ("*" = any, "claude*" = claude only)
    pub thinking_level: ThinkingLevel, // minimal/low/medium/high
    pub blocking: bool,                // wait for result before continuing
    pub spawns: Vec<String>,           // agent types this can spawn (["*"] or ["explore", "task"])
    pub system_prompt: String,         // the markdown body
    pub max_turns: u32,                // max tool-calling turns
}

impl AgentDefinition {
    pub fn defaults_with_name(name: impl Into<String>, system_prompt: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            description: String::new(),
            tools: vec!["*".to_string()],
            model_pattern: "*".to_string(),
            thinking_level: ThinkingLevel::default(),
            blocking: true,
            spawns: Vec::new(),
            system_prompt: system_prompt.into(),
            max_turns: 15,
        }
    }
}

pub struct AgentRegistry {
    definitions: Vec<AgentDefinition>,
}

impl AgentRegistry {
    /// Load bundled + user + project agents.
    ///
    /// Duplicate resolution: project > user > bundled.
    pub fn load() -> Self {
        let mut map: HashMap<String, AgentDefinition> = HashMap::new();

        for def in bundled_agent_definitions() {
            map.insert(def.name.clone(), def);
        }

        for def in load_dir_agents(user_agents_dir()) {
            map.insert(def.name.clone(), def);
        }

        for def in load_dir_agents(project_agents_dir()) {
            map.insert(def.name.clone(), def);
        }

        let mut definitions: Vec<AgentDefinition> = map.into_values().collect();
        definitions.sort_by(|a, b| a.name.cmp(&b.name));
        Self { definitions }
    }

    pub fn find(&self, name: &str) -> Option<&AgentDefinition> {
        self.definitions.iter().find(|d| d.name == name)
    }

    #[cfg(test)]
    fn names(&self) -> Vec<String> {
        self.definitions.iter().map(|d| d.name.clone()).collect()
    }
}

fn bundled_agent_definitions() -> Vec<AgentDefinition> {
    // Keep this tiny; it only exists so the registry always has something.
    const EXPLORE_MD: &str = r#"---
name: explore
description: Fast read-only codebase scout
tools: ["read", "grep", "find"]
model: "*"
thinking: minimal
blocking: true
spawns: []
max_turns: 15
---

You are a code exploration agent. Focus on quickly mapping a codebase and returning
compact, high-signal context for another agent to act on.
"#;

    match parse_agent_markdown("explore.md", EXPLORE_MD) {
        Ok(def) => vec![def],
        Err(e) => {
            warn!("Bundled agent explore failed to parse: {e}");
            vec![]
        }
    }
}

fn user_agents_dir() -> Option<PathBuf> {
    dirs::home_dir().map(|h| h.join(".pawan").join("agents"))
}

fn project_agents_dir() -> Option<PathBuf> {
    std::env::current_dir()
        .ok()
        .map(|d| d.join(".pawan").join("agents"))
}

fn load_dir_agents(dir: Option<PathBuf>) -> Vec<AgentDefinition> {
    let Some(dir) = dir else { return vec![] };
    let Ok(read_dir) = fs::read_dir(&dir) else {
        return vec![];
    };

    let mut out = Vec::new();
    for entry in read_dir.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("md") {
            continue;
        }
        match fs::read_to_string(&path) {
            Ok(content) => match parse_agent_markdown(
                path.file_name()
                    .and_then(|s| s.to_str())
                    .unwrap_or("agent.md"),
                &content,
            ) {
                Ok(def) => out.push(def),
                Err(e) => warn!("Skipping agent file {}: {e}", path.display()),
            },
            Err(e) => warn!("Skipping unreadable agent file {}: {e}", path.display()),
        }
    }
    out
}

#[derive(Default, Debug, Clone)]
struct Frontmatter {
    name: Option<String>,
    description: Option<String>,
    tools: Option<Vec<String>>,
    model: Option<String>,
    thinking: Option<ThinkingLevel>,
    blocking: Option<bool>,
    spawns: Option<Vec<String>>,
    max_turns: Option<u32>,
}

pub fn parse_agent_markdown(file_name: &str, content: &str) -> Result<AgentDefinition, String> {
    let (frontmatter, body) = split_frontmatter(content);

    let stem_name = Path::new(file_name)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("unknown")
        .to_string();

    let Some(front) = frontmatter else {
        return Ok(AgentDefinition::defaults_with_name(stem_name, body));
    };

    let fm = parse_frontmatter_kv(&front).map_err(|e| format!("invalid frontmatter: {e}"))?;

    let name = fm.name.unwrap_or(stem_name);
    let mut def = AgentDefinition::defaults_with_name(name, body);

    if let Some(d) = fm.description {
        def.description = d;
    }
    if let Some(t) = fm.tools {
        def.tools = t;
    }
    if let Some(m) = fm.model {
        def.model_pattern = m;
    }
    if let Some(th) = fm.thinking {
        def.thinking_level = th;
    }
    if let Some(b) = fm.blocking {
        def.blocking = b;
    }
    if let Some(s) = fm.spawns {
        def.spawns = s;
    }
    if let Some(mt) = fm.max_turns {
        def.max_turns = mt;
    }

    Ok(def)
}

fn split_frontmatter(content: &str) -> (Option<String>, String) {
    let mut lines = content.lines();
    let Some(first) = lines.next() else {
        return (None, String::new());
    };
    if first.trim() != "---" {
        return (None, content.to_string());
    }

    let mut front = Vec::new();
    let mut found_end = false;
    for line in lines.by_ref() {
        if line.trim() == "---" {
            found_end = true;
            break;
        }
        front.push(line);
    }

    if !found_end {
        // Malformed frontmatter; treat as missing.
        return (None, content.to_string());
    }

    let body = lines.collect::<Vec<_>>().join("\n");
    (
        Some(front.join("\n")),
        body.trim_start_matches('\n').to_string(),
    )
}

fn parse_frontmatter_kv(front: &str) -> Result<Frontmatter, String> {
    let mut fm = Frontmatter::default();

    for (idx, raw) in front.lines().enumerate() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let (k, v) = line
            .split_once(':')
            .ok_or_else(|| format!("line {} missing ':'", idx + 1))?;
        let key = k.trim().to_ascii_lowercase();
        let val = v.trim();

        match key.as_str() {
            "name" => fm.name = Some(parse_string(val)?),
            "description" => fm.description = Some(parse_string(val)?),
            "tools" => fm.tools = Some(parse_string_list(val)?),
            "model" => fm.model = Some(parse_string(val)?),
            "thinking" => {
                let s = parse_string(val)?;
                fm.thinking = ThinkingLevel::parse(&s).or_else(|| ThinkingLevel::parse(val));
                if fm.thinking.is_none() {
                    return Err(format!("invalid thinking level: {val}"));
                }
            }
            "blocking" => fm.blocking = Some(parse_bool(val)?),
            "spawns" => fm.spawns = Some(parse_string_list(val)?),
            "max_turns" => fm.max_turns = Some(parse_u32(val)?),
            _ => {
                // Ignore unknown keys for forward compatibility.
            }
        }
    }

    Ok(fm)
}

fn parse_string(s: &str) -> Result<String, String> {
    let t = s.trim();
    if t.starts_with('"') && t.ends_with('"') && t.len() >= 2 {
        return Ok(t[1..t.len() - 1].to_string());
    }
    if t.starts_with('\'') && t.ends_with('\'') && t.len() >= 2 {
        return Ok(t[1..t.len() - 1].to_string());
    }
    Ok(t.to_string())
}

fn parse_bool(s: &str) -> Result<bool, String> {
    match s.trim().to_ascii_lowercase().as_str() {
        "true" => Ok(true),
        "false" => Ok(false),
        _ => Err(format!("invalid bool: {s}")),
    }
}

fn parse_u32(s: &str) -> Result<u32, String> {
    s.trim()
        .parse::<u32>()
        .map_err(|e| format!("invalid u32: {e}"))
}

fn parse_string_list(s: &str) -> Result<Vec<String>, String> {
    // We support JSON-style arrays since that's also YAML-valid: ["a", "b"] or [].
    let t = s.trim();
    if !t.starts_with('[') {
        return Err("expected JSON-style array (e.g. [\"read\", \"grep\"])".to_string());
    }

    let v: serde_json::Value =
        serde_json::from_str(t).map_err(|e| format!("invalid array syntax: {e}"))?;
    let arr = v.as_array().ok_or_else(|| "expected array".to_string())?;

    let mut out = Vec::with_capacity(arr.len());
    for el in arr {
        let s = el
            .as_str()
            .ok_or_else(|| "array elements must be strings".to_string())?;
        out.push(s.to_string());
    }

    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_valid_agent_definition() {
        let md = r#"---
name: explore
description: Fast read-only codebase scout
tools: ["read", "grep", "find"]
model: "*"
thinking: minimal
blocking: true
spawns: []
max_turns: 15
---

You are a code exploration agent...
"#;

        let def = parse_agent_markdown("explore.md", md).expect("parse");
        assert_eq!(def.name, "explore");
        assert_eq!(def.description, "Fast read-only codebase scout");
        assert_eq!(def.tools, vec!["read", "grep", "find"]);
        assert_eq!(def.model_pattern, "*");
        assert_eq!(def.thinking_level, ThinkingLevel::Minimal);
        assert!(def.blocking);
        assert_eq!(def.spawns, Vec::<String>::new());
        assert_eq!(def.max_turns, 15);
        assert!(def
            .system_prompt
            .contains("You are a code exploration agent"));
    }

    #[test]
    fn parse_missing_frontmatter_uses_defaults() {
        let md = "System prompt only.\nSecond line.\n";
        let def = parse_agent_markdown("custom.md", md).expect("parse");
        assert_eq!(def.name, "custom");
        assert_eq!(def.tools, vec!["*"]);
        assert_eq!(def.model_pattern, "*");
        assert_eq!(def.thinking_level, ThinkingLevel::Minimal);
        assert!(def.blocking);
        assert_eq!(def.spawns, Vec::<String>::new());
        assert_eq!(def.max_turns, 15);
        assert!(def.system_prompt.contains("System prompt only."));
    }

    #[test]
    fn registry_loads_bundled_agents() {
        let reg = AgentRegistry::load();
        assert!(reg.find("explore").is_some());
        assert!(reg.names().contains(&"explore".to_string()));
    }
}
