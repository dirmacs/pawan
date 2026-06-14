use pawan::tools::{rmux::RmuxTool, Tool};
use serde_json::json;

fn live_rmux_enabled() -> bool {
    std::env::var("PAWAN_RMUX_LIVE")
        .map(|value| matches!(value.as_str(), "1" | "true" | "TRUE" | "yes" | "YES"))
        .unwrap_or(false)
}

fn rmux_available() -> bool {
    std::process::Command::new("rmux")
        .arg("--version")
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false)
}

#[tokio::test]
#[ignore = "requires PAWAN_RMUX_LIVE=1 and an rmux binary/daemon"]
async fn live_rmux_session_roundtrip() {
    if !live_rmux_enabled() {
        eprintln!("skipping live rmux roundtrip: set PAWAN_RMUX_LIVE=1 to enable");
        return;
    }
    if !rmux_available() {
        eprintln!("skipping live rmux roundtrip: rmux binary is not available on PATH");
        return;
    }

    let tool = RmuxTool::new();
    let session = format!("pawan-live-{}", std::process::id());
    let marker = format!("pawan-rmux-live-ready-{session}");
    let command = format!("printf '{marker}\\n'; sleep 30");

    let ensure = tool
        .execute(json!({
            "action": "ensure_session",
            "session": session,
            "cols": 80,
            "rows": 24,
            "command": command,
            "detached": true,
            "timeout_secs": 10
        }))
        .await
        .expect("ensure rmux session");
    assert_eq!(ensure["session"], session);

    tool.execute(json!({
        "action": "wait_for_text",
        "session": session,
        "text": marker,
        "timeout_secs": 10
    }))
    .await
    .expect("wait for marker text");

    let snapshot = tool
        .execute(json!({
            "action": "snapshot",
            "session": session,
            "timeout_secs": 10
        }))
        .await
        .expect("capture rmux snapshot");
    assert!(
        snapshot["visible_text"]
            .as_str()
            .expect("visible_text string")
            .contains(&marker),
        "snapshot should contain marker: {snapshot}"
    );

    let killed = tool
        .execute(json!({
            "action": "kill_session",
            "session": session,
            "timeout_secs": 10
        }))
        .await
        .expect("kill rmux session");
    assert_eq!(killed["killed"], true);
}
