//! RMUX tool: typed terminal multiplexer control for long-running agent workflows.
//!
//! Pawan uses this as the first integration point for durable terminal panes:
//! create/reuse a named RMUX session, drive input, wait for visible text, and
//! capture pane snapshots without scraping an ad-hoc shell subprocess.

use std::time::Duration;

use async_trait::async_trait;
use rmux_sdk::{
    EnsureSession, EnsureSessionPolicy, PaneProcessState, Rmux, SessionName, TerminalSizeSpec,
};
use serde::Deserialize;
use serde_json::{json, Value};

use super::Tool;
use crate::{PawanError, Result};

const DEFAULT_TIMEOUT_SECS: u64 = 10;
const DEFAULT_WINDOW: u32 = 0;
const DEFAULT_PANE: u32 = 0;

#[derive(Debug, Deserialize)]
struct RmuxArgs {
    action: String,
    session: Option<String>,
    window: Option<u32>,
    pane: Option<u32>,
    text: Option<String>,
    key: Option<String>,
    cols: Option<u16>,
    rows: Option<u16>,
    cwd: Option<String>,
    command: Option<String>,
    detached: Option<bool>,
    timeout_secs: Option<u64>,
    title: Option<String>,
    title_prefix: Option<String>,
    command_contains: Option<String>,
    cwd_contains: Option<String>,
    running: Option<bool>,
}

#[derive(Clone, Default)]
pub struct RmuxTool;

impl RmuxTool {
    pub fn new() -> Self {
        Self
    }

    fn timeout(args: &RmuxArgs) -> Duration {
        Duration::from_secs(args.timeout_secs.unwrap_or(DEFAULT_TIMEOUT_SECS))
    }

    async fn client(args: &RmuxArgs) -> Result<Rmux> {
        Rmux::builder()
            .default_timeout(Self::timeout(args))
            .connect_or_start()
            .await
            .map_err(|e| {
                PawanError::Tool(format!(
                    "rmux connect_or_start failed: {e}. Ensure the rmux binary is installed, on PATH, and able to start its daemon."
                ))
            })
    }

    fn session_name(args: &RmuxArgs) -> Result<SessionName> {
        let session = args
            .session
            .as_deref()
            .ok_or_else(|| PawanError::Tool("rmux session is required".into()))?;
        SessionName::new(session.to_string())
            .map_err(|e| PawanError::Tool(format!("invalid rmux session name: {e}")))
    }

    async fn pane(rmux: &Rmux, args: &RmuxArgs) -> Result<rmux_sdk::Pane> {
        let session_name = Self::session_name(args)?;
        let window = args.window.unwrap_or(DEFAULT_WINDOW);
        let pane = args.pane.unwrap_or(DEFAULT_PANE);
        let session = rmux.session(session_name.clone()).await.map_err(|e| {
            PawanError::Tool(format!(
                "rmux session lookup failed for session '{}': {e}",
                session_name.as_str()
            ))
        })?;
        Ok(session.pane(window, pane))
    }

    async fn ensure_session(args: RmuxArgs) -> Result<Value> {
        let session_name = Self::session_name(&args)?;
        if matches!((args.cols, args.rows), (Some(_), None) | (None, Some(_))) {
            return Err(PawanError::Tool(
                "rmux cols and rows must be supplied together".into(),
            ));
        }

        let rmux = Self::client(&args).await?;
        let mut ensure = EnsureSession::named(session_name)
            .policy(EnsureSessionPolicy::CreateOrReuse)
            .detached(args.detached.unwrap_or(true));
        if let (Some(cols), Some(rows)) = (args.cols, args.rows) {
            ensure = ensure.size(TerminalSizeSpec::new(cols, rows));
        }
        if let Some(cwd) = args.cwd.as_deref() {
            ensure = ensure.working_directory(cwd.to_string());
        }
        if let Some(command) = args.command.as_deref() {
            ensure = ensure.shell(command.to_string());
        }

        let session = rmux
            .ensure_session(ensure)
            .await
            .map_err(|e| PawanError::Tool(format!("rmux ensure_session failed: {e}")))?;
        Ok(json!({
            "session": session.name().as_str(),
            "created": session.was_created(),
            "endpoint": format!("{:?}", session.endpoint()),
        }))
    }

    async fn send_text(args: RmuxArgs) -> Result<Value> {
        let _session_name = Self::session_name(&args)?;
        let text = args
            .text
            .as_deref()
            .ok_or_else(|| PawanError::Tool("rmux text is required for send_text".into()))?;
        let rmux = Self::client(&args).await?;
        let pane = Self::pane(&rmux, &args).await?;
        pane.send_text(text)
            .await
            .map_err(|e| PawanError::Tool(format!("rmux send_text failed: {e}")))?;
        Ok(json!({"ok": true}))
    }

    async fn send_key(args: RmuxArgs) -> Result<Value> {
        let _session_name = Self::session_name(&args)?;
        let key = args
            .key
            .as_deref()
            .ok_or_else(|| PawanError::Tool("rmux key is required for send_key".into()))?;
        let rmux = Self::client(&args).await?;
        let pane = Self::pane(&rmux, &args).await?;
        pane.send_key(key)
            .await
            .map_err(|e| PawanError::Tool(format!("rmux send_key failed: {e}")))?;
        Ok(json!({"ok": true}))
    }

    async fn wait_for_text(args: RmuxArgs) -> Result<Value> {
        let _session_name = Self::session_name(&args)?;
        let text = args
            .text
            .as_deref()
            .ok_or_else(|| PawanError::Tool("rmux text is required for wait_for_text".into()))?;
        let rmux = Self::client(&args).await?;
        let pane = Self::pane(&rmux, &args).await?;
        pane.wait_for_text(text)
            .await
            .map_err(|e| PawanError::Tool(format!("rmux wait_for_text failed: {e}")))?;
        Ok(json!({"ok": true}))
    }

    async fn snapshot(args: RmuxArgs) -> Result<Value> {
        let _session_name = Self::session_name(&args)?;
        let rmux = Self::client(&args).await?;
        let pane = Self::pane(&rmux, &args).await?;
        let snapshot = pane
            .snapshot()
            .await
            .map_err(|e| PawanError::Tool(format!("rmux snapshot failed: {e}")))?;
        let visible_text = snapshot.visible_text();
        Ok(json!({
            "cols": snapshot.cols,
            "rows": snapshot.rows,
            "revision": snapshot.revision,
            "text": visible_text,
            "visible_text": visible_text,
        }))
    }

    async fn list_sessions(args: RmuxArgs) -> Result<Value> {
        let rmux = Self::client(&args).await?;
        let sessions = rmux
            .list_sessions()
            .await
            .map_err(|e| PawanError::Tool(format!("rmux list_sessions failed: {e}")))?
            .into_iter()
            .map(|session| session.as_str().to_string())
            .collect::<Vec<_>>();
        let count = sessions.len();
        Ok(json!({"sessions": sessions, "count": count}))
    }

    async fn list_panes(args: RmuxArgs) -> Result<Value> {
        let rmux = Self::client(&args).await?;
        let mut finder = rmux.find_panes();
        if let Some(session) = args.session.as_deref() {
            finder = finder.session(session);
        }
        if let Some(title) = args.title.as_deref() {
            finder = finder.title(title);
        }
        if let Some(title_prefix) = args.title_prefix.as_deref() {
            finder = finder.title_prefix(title_prefix);
        }
        if let Some(command_contains) = args.command_contains.as_deref() {
            finder = finder.command_contains(command_contains);
        }
        if let Some(cwd_contains) = args.cwd_contains.as_deref() {
            finder = finder.cwd_contains(cwd_contains);
        }
        if let Some(window) = args.window {
            finder = finder.window_index(window);
        }
        if let Some(running) = args.running {
            finder = if running {
                finder.running()
            } else {
                finder.exited()
            };
        }

        let panes = finder
            .all()
            .await
            .map_err(|e| PawanError::Tool(format!("rmux list_panes failed: {e}")))?
            .into_iter()
            .map(|pane| {
                json!({
                    "session": pane.session_name.as_str(),
                    "session_id": pane.session_id.as_u32(),
                    "window_id": pane.window_id.as_u32(),
                    "window_index": pane.window_index,
                    "pane_id": pane.pane_id.as_u32(),
                    "pane_index": pane.pane_index,
                    "title": pane.title,
                    "command": pane.command,
                    "working_directory": pane.working_directory,
                    "tags": pane.tags,
                    "process": pane_process_state(&pane.process),
                })
            })
            .collect::<Vec<_>>();
        let count = panes.len();
        Ok(json!({"panes": panes, "count": count}))
    }

    async fn kill_session(args: RmuxArgs) -> Result<Value> {
        let session_name = Self::session_name(&args)?;
        let rmux = Self::client(&args).await?;
        let session = rmux.session(session_name.clone()).await.map_err(|e| {
            PawanError::Tool(format!(
                "rmux session lookup failed for session '{}': {e}",
                session_name.as_str()
            ))
        })?;
        let killed = session
            .kill()
            .await
            .map_err(|e| PawanError::Tool(format!("rmux kill_session failed: {e}")))?;
        Ok(json!({"killed": killed}))
    }
}

fn pane_process_state(process: &PaneProcessState) -> Value {
    match process {
        PaneProcessState::Unknown => json!({"state": "unknown"}),
        PaneProcessState::Running { pid } => json!({"state": "running", "pid": pid}),
        PaneProcessState::Exited => json!({"state": "exited"}),
        _ => json!({"state": "unknown"}),
    }
}

#[async_trait]
impl Tool for RmuxTool {
    fn name(&self) -> &str {
        "rmux"
    }

    fn description(&self) -> &str {
        "Drive persistent RMUX terminal sessions/panes: list sessions/panes, ensure sessions, send input, wait for text, capture snapshots, and clean up sessions."
    }

    fn mutating(&self) -> bool {
        true
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["list_sessions", "list_panes", "ensure_session", "send_text", "send_key", "wait_for_text", "snapshot", "kill_session"],
                    "description": "RMUX operation to perform"
                },
                "session": {"type": "string", "description": "RMUX session name"},
                "window": {"type": "integer", "minimum": 0, "default": 0},
                "pane": {"type": "integer", "minimum": 0, "default": 0},
                "text": {"type": "string", "description": "Text to send or wait for"},
                "key": {"type": "string", "description": "tmux/RMUX key token, e.g. Enter or C-c"},
                "cols": {"type": "integer", "minimum": 1, "default": 120},
                "rows": {"type": "integer", "minimum": 1, "default": 32},
                "cwd": {"type": "string", "description": "Initial working directory for a new session"},
                "command": {"type": "string", "description": "Initial shell command for a new session"},
                "detached": {"type": "boolean", "default": true},
                "timeout_secs": {"type": "integer", "minimum": 1, "default": 10},
                "title": {"type": "string", "description": "Restrict list_panes to exact pane title"},
                "title_prefix": {"type": "string", "description": "Restrict list_panes to pane titles starting with prefix"},
                "command_contains": {"type": "string", "description": "Restrict list_panes to argv containing this text"},
                "cwd_contains": {"type": "string", "description": "Restrict list_panes to working directories containing this text"},
                "running": {"type": "boolean", "description": "Restrict list_panes to running panes when true, exited panes when false"}
            },
            "required": ["action"]
        })
    }

    async fn execute(&self, args: Value) -> Result<Value> {
        let args: RmuxArgs = serde_json::from_value(args)
            .map_err(|e| PawanError::Tool(format!("invalid rmux args: {e}")))?;
        match args.action.as_str() {
            "list_sessions" => Self::list_sessions(args).await,
            "list_panes" => Self::list_panes(args).await,
            "ensure_session" => Self::ensure_session(args).await,
            "send_text" => Self::send_text(args).await,
            "send_key" => Self::send_key(args).await,
            "wait_for_text" => Self::wait_for_text(args).await,
            "snapshot" => Self::snapshot(args).await,
            "kill_session" => Self::kill_session(args).await,
            other => Err(PawanError::Tool(format!(
                "unknown rmux action: {other}. Use list_sessions, list_panes, ensure_session, send_text, send_key, wait_for_text, snapshot, or kill_session"
            ))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schema_lists_supported_actions() {
        let schema = RmuxTool::new().parameters_schema();
        let actions = schema["properties"]["action"]["enum"]
            .as_array()
            .expect("action enum");
        assert!(actions.iter().any(|v| v == "list_sessions"));
        assert!(actions.iter().any(|v| v == "list_panes"));
        assert!(actions.iter().any(|v| v == "ensure_session"));
        assert!(actions.iter().any(|v| v == "snapshot"));
        assert!(actions.iter().any(|v| v == "kill_session"));
    }

    #[tokio::test]
    async fn rejects_missing_session_before_connecting() {
        let err = RmuxTool::new()
            .execute(json!({"action": "snapshot", "timeout_secs": 1}))
            .await
            .unwrap_err();
        assert!(err.to_string().contains("rmux session is required"));
    }

    #[tokio::test]
    async fn rejects_partial_terminal_size_before_connecting() {
        let err = RmuxTool::new()
            .execute(json!({"action": "ensure_session", "session": "dev", "cols": 120, "timeout_secs": 1}))
            .await
            .unwrap_err();
        assert!(err
            .to_string()
            .contains("rmux cols and rows must be supplied together"));
    }

    #[tokio::test]
    async fn rejects_unknown_action_before_connecting() {
        let err = RmuxTool::new()
            .execute(json!({"action": "teleport"}))
            .await
            .unwrap_err();
        assert!(err.to_string().contains("unknown rmux action"));
    }
}
