//! Live subagent run tracking for tools and the TUI queue strip.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::time::Instant;

/// Lifecycle state exposed to the TUI.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SubagentState {
    Running,
    Done,
    Failed,
}

/// One tracked subagent invocation.
#[derive(Debug, Clone)]
pub struct SubagentRun {
    pub id: String,
    pub label: String,
    pub source: String,
    pub agent_type: Option<String>,
    pub state: SubagentState,
    pub current_tool: Option<String>,
    started_at: Instant,
    pub duration_ms: Option<u64>,
    pub error: Option<String>,
}

/// Handle returned when a subagent run starts; updates the global tracker.
#[derive(Debug, Clone)]
pub struct SubagentHandle {
    id: String,
}

fn tracker() -> &'static Mutex<HashMap<String, SubagentRun>> {
    static TRACKER: OnceLock<Mutex<HashMap<String, SubagentRun>>> = OnceLock::new();
    TRACKER.get_or_init(|| Mutex::new(HashMap::new()))
}

impl SubagentHandle {
    pub fn start(
        label: impl Into<String>,
        source: &str,
        agent_type: Option<String>,
    ) -> Self {
        let id = uuid::Uuid::new_v4().to_string();
        let short_id = id[..8.min(id.len())].to_string();
        let run = SubagentRun {
            id: short_id.clone(),
            label: label.into(),
            source: source.to_string(),
            agent_type,
            state: SubagentState::Running,
            current_tool: None,
            started_at: Instant::now(),
            duration_ms: None,
            error: None,
        };
        tracker().lock().unwrap().insert(short_id.clone(), run);
        Self { id: short_id }
    }

    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn set_tool(&self, tool_name: &str) {
        if let Some(run) = tracker().lock().unwrap().get_mut(&self.id) {
            run.current_tool = Some(tool_name.to_string());
        }
    }

    pub fn clear_tool(&self) {
        if let Some(run) = tracker().lock().unwrap().get_mut(&self.id) {
            run.current_tool = None;
        }
    }

    pub fn complete_ok(&self) {
        if let Some(run) = tracker().lock().unwrap().get_mut(&self.id) {
            run.state = SubagentState::Done;
            run.duration_ms = Some(run.started_at.elapsed().as_millis() as u64);
            run.current_tool = None;
        }
    }

    pub fn complete_err(&self, error: impl Into<String>) {
        if let Some(run) = tracker().lock().unwrap().get_mut(&self.id) {
            run.state = SubagentState::Failed;
            run.error = Some(error.into());
            run.duration_ms = Some(run.started_at.elapsed().as_millis() as u64);
            run.current_tool = None;
        }
    }

    pub fn dismiss(self) {
        tracker().lock().unwrap().remove(&self.id);
    }
}

/// Snapshot of runs still visible in the queue strip.
pub fn snapshot_queue(max_age_ms: u64) -> Vec<SubagentRun> {
    let now = Instant::now();
    let mut guard = tracker().lock().unwrap();
    guard.retain(|_, run| {
        if run.state == SubagentState::Running {
            return true;
        }
        let age = run
            .duration_ms
            .unwrap_or_else(|| now.duration_since(run.started_at).as_millis() as u64);
        age < max_age_ms
    });
    let mut runs: Vec<_> = guard.values().cloned().collect();
    runs.sort_by(|a, b| a.started_at.cmp(&b.started_at));
    runs
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn handle_lifecycle_updates_state() {
        let h = SubagentHandle::start("explore auth", "task", Some("explore".into()));
        h.set_tool("grep_search");
        h.clear_tool();
        h.complete_ok();
        let snap = snapshot_queue(60_000);
        assert!(snap.iter().any(|r| r.id == h.id() && r.state == SubagentState::Done));
        h.dismiss();
        assert!(snapshot_queue(60_000).is_empty());
    }
}
