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

    #[test]
    fn complete_err_sets_failed_state() {
        let h = SubagentHandle::start("failing run", "task", None);
        h.complete_err("connection reset");
        let snap = snapshot_queue(60_000);
        let run = snap
            .iter()
            .find(|r| r.id == h.id())
            .expect("failed run should remain in queue");
        assert_eq!(run.state, SubagentState::Failed);
        assert_eq!(run.error.as_deref(), Some("connection reset"));
        assert!(run.duration_ms.is_some());
        h.dismiss();
    }

    #[test]
    fn snapshot_queue_evicts_old_done_runs() {
        let h = SubagentHandle::start("slow done", "task", None);
        std::thread::sleep(std::time::Duration::from_millis(100));
        h.complete_ok();
        let snap = snapshot_queue(50);
        assert!(
            !snap.iter().any(|r| r.id == h.id()),
            "done run whose duration exceeds max_age_ms should be evicted"
        );
        h.dismiss();
    }

    #[test]
    fn set_tool_and_clear_tool_update_running_run() {
        let h = SubagentHandle::start("tooling", "task", None);
        h.set_tool("read");
        let with_tool = snapshot_queue(60_000)
            .into_iter()
            .find(|r| r.id == h.id())
            .expect("running run should be visible");
        assert_eq!(with_tool.state, SubagentState::Running);
        assert_eq!(with_tool.current_tool.as_deref(), Some("read"));

        h.clear_tool();
        let cleared = snapshot_queue(60_000)
            .into_iter()
            .find(|r| r.id == h.id())
            .expect("running run should still be visible");
        assert_eq!(cleared.current_tool, None);
        h.dismiss();
    }

    #[test]
    fn multiple_concurrent_handles_tracked() {
        let h1 = SubagentHandle::start("first", "task", Some("explore".into()));
        let h2 = SubagentHandle::start("second", "task", Some("quick_task".into()));
        let snap = snapshot_queue(60_000);
        assert!(snap.iter().any(|r| r.id == h1.id() && r.label == "first"));
        assert!(snap.iter().any(|r| r.id == h2.id() && r.label == "second"));
        assert_ne!(h1.id(), h2.id());
        h1.dismiss();
        h2.dismiss();
    }

    #[test]
    fn complete_ok_sets_done_state_and_duration() {
        let h = SubagentHandle::start("ok run", "task", None);
        h.complete_ok();
        let run = snapshot_queue(60_000)
            .into_iter()
            .find(|r| r.id == h.id())
            .expect("done run");
        assert_eq!(run.state, SubagentState::Done);
        assert!(run.duration_ms.is_some());
        h.dismiss();
    }

    #[test]
    fn dismiss_removes_run_from_snapshot() {
        let h = SubagentHandle::start("ephemeral", "task", None);
        assert!(snapshot_queue(60_000).iter().any(|r| r.id == h.id()));
        let id = h.id().to_string();
        h.dismiss();
        assert!(!snapshot_queue(60_000).iter().any(|r| r.id == id));
    }

    #[test]
    fn snapshot_queue_retains_running_past_ttl() {
        let h = SubagentHandle::start("still running", "task", None);
        std::thread::sleep(std::time::Duration::from_millis(100));
        let snap = snapshot_queue(1);
        assert!(
            snap.iter()
                .any(|r| r.id == h.id() && r.state == SubagentState::Running)
        );
        h.dismiss();
    }

    #[test]
    fn complete_err_clears_current_tool() {
        let h = SubagentHandle::start("tool then fail", "task", None);
        h.set_tool("bash");
        h.complete_err("boom");
        let run = snapshot_queue(60_000)
            .into_iter()
            .find(|r| r.id == h.id())
            .expect("failed run");
        assert_eq!(run.current_tool, None);
        h.dismiss();
    }

    #[test]
    fn start_records_label_source_and_agent_type() {
        let h = SubagentHandle::start("my label", "quick_task", Some("explore".into()));
        let run = snapshot_queue(60_000)
            .into_iter()
            .find(|r| r.id == h.id())
            .expect("running run");
        assert_eq!(run.label, "my label");
        assert_eq!(run.source, "quick_task");
        assert_eq!(run.agent_type.as_deref(), Some("explore"));
        h.dismiss();
    }

    #[test]
    fn concurrent_handles_receive_distinct_ids() {
        let handles: Vec<_> = (0..5)
            .map(|i| SubagentHandle::start(format!("run-{i}"), "task", None))
            .collect();
        let ids: std::collections::HashSet<_> = handles.iter().map(SubagentHandle::id).collect();
        assert_eq!(ids.len(), handles.len());
        for h in handles {
            h.dismiss();
        }
    }
}
