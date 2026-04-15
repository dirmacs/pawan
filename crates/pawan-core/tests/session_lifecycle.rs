// Integration tests for session lifecycle features

use pawan::agent::{Session, Message, Role, ToolCallRequest, ToolResultMessage};
use pawan::{Result, PawanError};
use std::thread;
use std::time::Duration;
use tempfile::tempdir;

#[test]
fn roundtrip_save_load() -> Result<()> {
    let mut sess = Session::new("gpt-4");
    sess.add_tag("test").ok(); // ensure tags work
    sess.messages.push(Message {
        role: Role::User,
        content: "Hello".into(),
        tool_calls: vec![],
        tool_result: None,
    });
    let path = sess.save()?;
    let loaded = Session::load(&sess.id)?;
    assert_eq!(sess.id, loaded.id);
    assert_eq!(sess.model, loaded.model);
    assert_eq!(sess.messages, loaded.messages);
    // cleanup
    std::fs::remove_file(path).ok();
    Ok(())
}

#[test]
fn autosave_updates_same_file() -> Result<()> {
    // Use a temporary HOME to avoid polluting real sessions dir
    let tmp = tempdir()?;
    std::env::set_var("HOME", tmp.path());
    let mut sess = Session::new("gpt-4");
    let path1 = sess.save()?;
    // modify something and autosave again
    sess.messages.push(Message {
        role: Role::User,
        content: "Second".into(),
        tool_calls: vec![],
        tool_result: None,
    });
    let path2 = sess.save()?;
    // Paths should be identical and only one file exists
    assert_eq!(path1, path2);
    let entries = std::fs::read_dir(Session::sessions_dir()?)?.count();
    assert_eq!(entries, 1);
    // cleanup
    std::fs::remove_file(path1).ok();
    std::env::remove_var("HOME");
    Ok(())
}

#[test]
fn export_import_json_preserves_data() -> Result<()> {
    let mut sess = Session::new("gpt-4");
    sess.tags.push("alpha".into());
    sess.messages.push(Message {
        role: Role::User,
        content: "Export test".into(),
        tool_calls: vec![],
        tool_result: None,
    });
    // Export via serde_json string
    let json = serde_json::to_string(&sess).unwrap();
    // Write to temp file
    let dir = tempdir()?;
    let file_path = dir.path().join("export.json");
    std::fs::write(&file_path, json)?;
    // Import (new ID is generated)
    let imported = Session::from_json_file(&file_path)?;
    assert_eq!(imported.model, sess.model);
    assert_eq!(imported.tags, sess.tags);
    assert_eq!(imported.messages, sess.messages);
    Ok(())
}

#[test]
fn tool_call_preservation_roundtrip() -> Result<()> {
    let mut sess = Session::new("gpt-4");
    let tool_call = ToolCallRequest {
        id: "call1".into(),
        name: "bash".into(),
        arguments: serde_json::json!({"cmd": "echo hi"}),
    };
    let tool_res = ToolResultMessage {
        tool_call_id: tool_call.id.clone(),
        content: serde_json::json!({"output": "hi"}),
        success: true,
    };
    sess.messages.push(Message {
        role: Role::Tool,
        content: "".into(),
        tool_calls: vec![tool_call.clone()],
        tool_result: Some(tool_res.clone()),
    });
    let path = sess.save()?;
    let loaded = Session::load(&sess.id)?;
        assert_eq!(loaded.messages.len(), sess.messages.len());
        let loaded_msg = &loaded.messages[0];
        let orig_msg = &sess.messages[0];
        assert_eq!(loaded_msg.role, orig_msg.role);
        assert_eq!(loaded_msg.tool_calls.len(), orig_msg.tool_calls.len());
        assert_eq!(loaded_msg.tool_calls[0].id, orig_msg.tool_calls[0].id);
        assert_eq!(loaded_msg.tool_calls[0].name, orig_msg.tool_calls[0].name);
        assert_eq!(loaded_msg.tool_calls[0].arguments, orig_msg.tool_calls[0].arguments);
        assert_eq!(loaded_msg.tool_result.as_ref().unwrap().tool_call_id, orig_msg.tool_result.as_ref().unwrap().tool_call_id);
        assert_eq!(loaded_msg.tool_result.as_ref().unwrap().content, orig_msg.tool_result.as_ref().unwrap().content);
        assert_eq!(loaded_msg.tool_result.as_ref().unwrap().success, orig_msg.tool_result.as_ref().unwrap().success);
    std::fs::remove_file(path).ok();
    Ok(())
}

#[test]
#[ignore]
fn timeout_enforcement_triggers_error() {
    // This test is marked ignored because precise timing control is non-trivial in CI.
    // The intention is to ensure that a short idle timeout results in an Agent error.
    // Implementation would involve configuring the agent with a 0‑second timeout
    // and provoking a second iteration after a delay.
    // See issue tracker for future fleshing out.
    assert!(true);
}

#[test]
fn search_sessions_multiple_results() -> Result<()> {
    let tmp = tempdir()?;
    std::env::set_var("HOME", tmp.path());
    // create two sessions with distinct content
    let mut s1 = Session::new("m1");
    s1.messages.push(Message { role: Role::User, content: "unique term alpha".into(), tool_calls: vec![], tool_result: None });
    s1.save()?;
    let mut s2 = Session::new("m2");
    s2.messages.push(Message { role: Role::User, content: "beta content".into(), tool_calls: vec![], tool_result: None });
    s2.save()?;
let results = pawan::agent::search_sessions("alpha").unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].id, s1.id);
    std::env::remove_var("HOME");
    Ok(())
}

#[test]
fn prune_sessions_respects_policy() -> Result<()> {
    let tmp = tempdir()?;
    std::env::set_var("HOME", tmp.path());
    // create three sessions with different timestamps
    let dir = Session::sessions_dir()?;
    for i in 0..3 {
        let mut s = Session::new("m");
        s.id = format!("sess{}", i);
        s.updated_at = format!("2020-01-0{}T00:00:00Z", i+1);
        let path = dir.join(format!("{}.json", s.id));
        std::fs::write(&path, serde_json::to_string_pretty(&s)?)?;
    }
    // keep only most recent (max_sessions = 1)
let policy = pawan::agent::RetentionPolicy { max_age_days: None, max_sessions: Some(1), keep_tags: vec![] };
let deleted = pawan::agent::prune_sessions(&policy)?;
    assert_eq!(deleted, 2);
let list = pawan::agent::Session::list()?;
    assert_eq!(list.len(), 1);
    std::env::remove_var("HOME");
    Ok(())
}
