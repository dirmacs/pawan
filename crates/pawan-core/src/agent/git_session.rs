//! Git-backed session store — conversation trees with fork/lineage/leaves
//!
//! Each pawan session is a git commit in a bare repo. Branching conversations
//! = forking from any commit in the DAG.
//!
//! Uses gitoxide (gix) — pure Rust, no C dependency.
//!
//! Inspired by Karpathy's AgentHub (bare git DAG) and Yegge's Beads (git-backed memory).

use crate::agent::session::Session;
use crate::{PawanError, Result};
use gix::bstr::{BStr, ByteSlice};
use gix::object::tree::EntryKind;
use gix::ObjectId;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::path::PathBuf;

/// Summary of a git-backed session commit
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommitInfo {
    pub hash: String,
    pub short_hash: String,
    pub message: String,
    pub timestamp: i64,
    pub message_count: usize,
    pub model: String,
}

/// Git-backed session store using a bare repo at ~/.pawan/repo/
pub struct GitSessionStore {
    repo: gix::Repository,
}

impl GitSessionStore {
    /// Initialize or open the git session store
    pub fn init() -> Result<Self> {
        let path = Self::default_path()?;
        let repo = if path.join("HEAD").exists() {
            gix::open(&path)
                .map_err(|e| PawanError::Git(format!("Open repo: {}", e)))?
        } else {
            std::fs::create_dir_all(&path)
                .map_err(|e| PawanError::Git(format!("Create dir: {}", e)))?;
            gix::create::into(&path, gix::create::Kind::Bare, gix::create::Options::default())
                .map_err(|e| PawanError::Git(format!("Init repo: {}", e)))?;
            gix::open(&path)
                .map_err(|e| PawanError::Git(format!("Open after init: {}", e)))?
        };
        Ok(Self { repo })
    }

    /// Open with a custom path (for testing)
    pub fn open(repo: gix::Repository) -> Self {
        Self { repo }
    }

    fn default_path() -> Result<PathBuf> {
        let home = std::env::var("HOME").unwrap_or_else(|_| "/root".into());
        Ok(PathBuf::from(home).join(".pawan").join("repo"))
    }

    fn commit_message(session: &Session) -> String {
        session
            .messages
            .iter()
            .rev()
            .find(|m| m.role == crate::agent::Role::User)
            .map(|m| {
                let trunc: String = m.content.chars().take(80).collect();
                format!("[{}] {}", session.id, trunc)
            })
            .unwrap_or_else(|| format!("[{}] new session", session.id))
    }

    /// Save session as a git commit. Returns the commit hash.
    pub fn save_commit(&self, session: &Session, parent_hash: Option<&str>) -> Result<String> {
        let json = serde_json::to_string_pretty(session)
            .map_err(|e| PawanError::Git(format!("Serialize: {}", e)))?;

        // Write blob
        let blob_id = self.repo
            .write_blob(json.as_bytes())
            .map_err(|e| PawanError::Git(format!("Blob: {}", e)))?
            .detach();

        // Build tree with a single "session.json" entry
        let empty_tree_id = self.repo.empty_tree().id;
        let tree_id = self.repo
            .edit_tree(empty_tree_id)
            .map_err(|e| PawanError::Git(format!("TreeEditor init: {}", e)))?
            .upsert("session.json", EntryKind::Blob, blob_id)
            .map_err(|e| PawanError::Git(format!("Upsert: {}", e)))?
            .write()
            .map_err(|e| PawanError::Git(format!("Write tree: {}", e)))?
            .detach();

        let msg = Self::commit_message(session);
        let refname = format!("refs/sessions/{}", session.id);
        let now = chrono::Utc::now().timestamp();
        let time = gix::date::Time::new(now, 0);
        let time_str = time.to_string();
        let sig = gix::actor::SignatureRef {
            name: "pawan".into(),
            email: "pawan@localhost".into(),
            time: time_str.as_str().into(),
        };

        // Resolve parent OIDs
        let parents: Vec<ObjectId> = match parent_hash {
            Some(h) => {
                let oid = ObjectId::from_hex(h.as_bytes())
                    .map_err(|e| PawanError::Git(format!("Bad hash: {}", e)))?;
                // Verify commit exists
                self.repo.find_commit(oid)
                    .map_err(|e| PawanError::Git(format!("Parent not found: {}", e)))?;
                vec![oid]
            }
            None => vec![],
        };

        let commit_id = self.repo
            .commit_as(sig, sig, refname.as_str(), msg.as_str(), tree_id, parents)
            .map_err(|e| PawanError::Git(format!("Commit: {}", e)))?
            .detach();

        Ok(commit_id.to_hex().to_string())
    }

    /// Load session from a commit hash
    pub fn load_commit(&self, hash: &str) -> Result<Session> {
        let oid = ObjectId::from_hex(hash.as_bytes())
            .map_err(|e| PawanError::Git(format!("Bad hash: {}", e)))?;
        let commit = self.repo.find_commit(oid)
            .map_err(|e| PawanError::Git(format!("Not found: {}", e)))?;
        self.session_from_commit(&commit)
    }

    /// Fork: create a new commit branching off parent
    pub fn fork(&self, parent_hash: &str, session: &Session) -> Result<String> {
        self.save_commit(session, Some(parent_hash))
    }

    /// List leaf commits (conversation tips with no children)
    pub fn list_leaves(&self) -> Result<Vec<CommitInfo>> {
        let all_oids = self.all_oids()?;
        let mut parent_set: HashSet<ObjectId> = HashSet::new();

        for &oid in &all_oids {
            if let Ok(c) = self.repo.find_commit(oid) {
                for pid in c.parent_ids() {
                    parent_set.insert(pid.detach());
                }
            }
        }

        let mut leaves = Vec::new();
        for &oid in &all_oids {
            if !parent_set.contains(&oid) {
                if let Ok(info) = self.info(oid) {
                    leaves.push(info);
                }
            }
        }
        leaves.sort_by(|a, b| b.timestamp.cmp(&a.timestamp));
        Ok(leaves)
    }

    /// Walk lineage from commit to root
    pub fn lineage(&self, hash: &str) -> Result<Vec<CommitInfo>> {
        let mut oid = ObjectId::from_hex(hash.as_bytes())
            .map_err(|e| PawanError::Git(format!("Bad hash: {}", e)))?;
        let mut chain = Vec::new();

        loop {
            let commit = self.repo.find_commit(oid)
                .map_err(|e| PawanError::Git(format!("Not found: {}", e)))?;
            chain.push(self.info(oid)?);
            let mut parents = commit.parent_ids();
            match parents.next() {
                Some(pid) => oid = pid.detach(),
                None => break,
            }
        }
        Ok(chain)
    }

    /// Find all children of a commit
    pub fn children(&self, hash: &str) -> Result<Vec<CommitInfo>> {
        let target = ObjectId::from_hex(hash.as_bytes())
            .map_err(|e| PawanError::Git(format!("Bad hash: {}", e)))?;
        let all = self.all_oids()?;
        let mut result = Vec::new();

        for &oid in &all {
            if let Ok(c) = self.repo.find_commit(oid) {
                for pid in c.parent_ids() {
                    if pid.detach() == target {
                        if let Ok(info) = self.info(oid) {
                            result.push(info);
                        }
                    }
                }
            }
        }
        Ok(result)
    }

    /// List all session refs (latest commit per session)
    pub fn list_sessions(&self) -> Result<Vec<CommitInfo>> {
        let mut sessions = Vec::new();
        let refs = self.repo.references()
            .map_err(|e| PawanError::Git(format!("References: {}", e)))?;
        let session_refs = refs.prefixed("refs/sessions/")
            .map_err(|e| PawanError::Git(format!("Prefixed refs: {}", e)))?;
        for r in session_refs.flatten() {
            if let Some(id) = r.try_id() {
                if let Ok(info) = self.info(id.detach()) {
                    sessions.push(info);
                }
            }
        }
        sessions.sort_by(|a, b| b.timestamp.cmp(&a.timestamp));
        Ok(sessions)
    }

    // -- internals --

    fn all_oids(&self) -> Result<Vec<ObjectId>> {
        let mut oids: Vec<ObjectId> = Vec::new();
        let mut visited: HashSet<ObjectId> = HashSet::new();
        let mut stack: Vec<ObjectId> = Vec::new();

        let refs = self.repo.references()
            .map_err(|e| PawanError::Git(format!("References: {}", e)))?;
        if let Ok(session_refs) = refs.prefixed("refs/sessions/") {
            for r in session_refs.flatten() {
                if let Some(id) = r.try_id() {
                    stack.push(id.detach());
                }
            }
        }

        while let Some(oid) = stack.pop() {
            if !visited.insert(oid) { continue; }
            oids.push(oid);
            if let Ok(c) = self.repo.find_commit(oid) {
                for pid in c.parent_ids() {
                    let pid = pid.detach();
                    if !visited.contains(&pid) {
                        stack.push(pid);
                    }
                }
            }
        }
        Ok(oids)
    }

    fn info(&self, oid: ObjectId) -> Result<CommitInfo> {
        let commit = self.repo.find_commit(oid)
            .map_err(|e| PawanError::Git(format!("Not found: {}", e)))?;
        let hash = oid.to_hex().to_string();
        let (mc, model) = self.session_meta(&commit).unwrap_or((0, "unknown".into()));
        let decoded = commit.decode()
            .map_err(|e| PawanError::Git(format!("Decode: {}", e)))?;
        // decoded.author is &BStr (raw); parse it to get time
        let author_sig = decoded.author()
            .map_err(|e| PawanError::Git(format!("Author parse: {}", e)))?;
        let timestamp: i64 = author_sig.time.parse::<gix::date::Time>()
            .map(|t| t.seconds)
            .unwrap_or(0);

        Ok(CommitInfo {
            short_hash: hash[..8].to_string(),
            hash,
            message: decoded.message.trim().to_str_lossy().to_string(),
            timestamp,
            message_count: mc,
            model,
        })
    }

    fn session_meta(&self, commit: &gix::Commit<'_>) -> Option<(usize, String)> {
        let decoded = commit.decode().ok()?;
        let tree_oid = decoded.tree();
        let tree = self.repo.find_tree(tree_oid).ok()?;
        let tree_data = tree.decode().ok()?;
        let entry = tree_data.entries.iter().find(|e| e.filename == b"session.json")?;
        let entry_oid = entry.oid.to_owned();
        let blob = self.repo.find_blob(entry_oid).ok()?;
        let json = std::str::from_utf8(&blob.data).ok()?;
        let s: Session = serde_json::from_str(json).ok()?;
        Some((s.messages.len(), s.model))
    }

    fn session_from_commit(&self, commit: &gix::Commit<'_>) -> Result<Session> {
        let decoded = commit.decode()
            .map_err(|e| PawanError::Git(format!("Decode: {}", e)))?;
        let tree_oid = decoded.tree();
        let tree = self.repo.find_tree(tree_oid)
            .map_err(|e| PawanError::Git(format!("Tree: {}", e)))?;
        let tree_data = tree.decode()
            .map_err(|e| PawanError::Git(format!("Decode tree: {}", e)))?;
        let entry = tree_data.entries.iter()
            .find(|e| e.filename == b"session.json")
            .ok_or_else(|| PawanError::Git("No session.json".into()))?;
        let entry_oid = entry.oid.to_owned();
        let blob = self.repo.find_blob(entry_oid)
            .map_err(|e| PawanError::Git(format!("Blob: {}", e)))?;
        let json = std::str::from_utf8(&blob.data)
            .map_err(|e| PawanError::Git(format!("UTF-8: {}", e)))?;
        serde_json::from_str(json)
            .map_err(|e| PawanError::Git(format!("Parse: {}", e)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::{Message, Role};

    fn test_store() -> (GitSessionStore, tempfile::TempDir) {
        let dir = tempfile::TempDir::new().unwrap();
        gix::create::into(dir.path(), gix::create::Kind::Bare, gix::create::Options::default())
            .unwrap();
        let repo = gix::open(dir.path()).unwrap();
        (GitSessionStore { repo }, dir)
    }

    fn session(id: &str, msg: &str) -> Session {
        Session {
            notes: String::new(),
            parent_id: None,
            root_id: None,
            branch_label: None,
            branch_depth: 0,
            labels: vec![],
            id: id.into(),
            model: "test-model".into(),
            created_at: chrono::Utc::now().to_rfc3339(),
            updated_at: chrono::Utc::now().to_rfc3339(),
            messages: vec![Message {
                role: Role::User,
                content: msg.into(),
                tool_calls: vec![],
                tool_result: None,
            }],
            total_tokens: 0,
            iteration_count: 0,
            tags: Vec::new(),
        }
    }

    #[test]
    fn save_and_load() {
        let (store, _dir) = test_store();
        let s = session("s1", "hello world");
        let hash = store.save_commit(&s, None).unwrap();
        let loaded = store.load_commit(&hash).unwrap();
        assert_eq!(loaded.id, "s1");
        assert_eq!(loaded.messages[0].content, "hello world");
    }

    #[test]
    fn fork_creates_branch() {
        let (store, _dir) = test_store();
        let s1 = session("s1", "root msg");
        let root = store.save_commit(&s1, None).unwrap();

        let s2 = session("s1-fork", "branch msg");
        let fork = store.fork(&root, &s2).unwrap();

        let lineage = store.lineage(&fork).unwrap();
        assert_eq!(lineage.len(), 2);
        assert_eq!(lineage[1].hash, root);
    }

    #[test]
    fn leaves_finds_tips() {
        let (store, _dir) = test_store();
        let s = session("s1", "root");
        let root = store.save_commit(&s, None).unwrap();

        let a = session("a", "child a");
        let ha = store.save_commit(&a, Some(&root)).unwrap();

        let b = session("b", "child b");
        let hb = store.save_commit(&b, Some(&root)).unwrap();

        let leaves = store.list_leaves().unwrap();
        let hashes: Vec<&str> = leaves.iter().map(|l| l.hash.as_str()).collect();
        assert_eq!(leaves.len(), 2);
        assert!(hashes.contains(&ha.as_str()));
        assert!(hashes.contains(&hb.as_str()));
    }

    #[test]
    fn children_finds_forks() {
        let (store, _dir) = test_store();
        let s = session("s1", "root");
        let root = store.save_commit(&s, None).unwrap();

        store.save_commit(&session("a", "fork1"), Some(&root)).unwrap();
        store.save_commit(&session("b", "fork2"), Some(&root)).unwrap();

        let children = store.children(&root).unwrap();
        assert_eq!(children.len(), 2);
    }

    #[test]
    fn test_list_sessions_after_save() {
        let (store, _dir) = test_store();
        let s = session("sess-list-1", "session list test");
        store.save_commit(&s, None).unwrap();

        let sessions = store.list_sessions().unwrap();
        assert!(!sessions.is_empty(), "list_sessions must be non-empty after save");
        let found = sessions.iter().any(|c| c.message.contains("sess-list-1"));
        assert!(found, "saved session id must appear in list_sessions()");
    }

    #[test]
    fn test_load_commit_bad_hash_returns_git_error() {
        let (store, _dir) = test_store();
        let err = store.load_commit("not_a_valid_hash_zzz").unwrap_err();
        match err {
            crate::PawanError::Git(msg) => {
                assert!(!msg.is_empty(), "Git error message must not be empty")
            }
            other => panic!("expected PawanError::Git, got {:?}", other),
        }
    }

    #[test]
    fn test_list_leaves_empty_repo_returns_empty() {
        let (store, _dir) = test_store();
        let leaves = store.list_leaves().unwrap();
        assert!(leaves.is_empty(), "empty repo must have no leaves");
    }

    #[test]
    fn test_commit_message_no_user_messages_uses_fallback() {
        let s = Session {
            notes: String::new(),
            parent_id: None,
            root_id: None,
            branch_label: None,
            branch_depth: 0,
            labels: vec![],
            id: "no-msg".into(),
            model: "m".into(),
            created_at: chrono::Utc::now().to_rfc3339(),
            updated_at: chrono::Utc::now().to_rfc3339(),
            messages: vec![],
            total_tokens: 0,
            iteration_count: 0,
            tags: Vec::new(),
        };
        let msg = GitSessionStore::commit_message(&s);
        assert!(
            msg.contains("new session"),
            "commit message with no user messages must say 'new session', got: {msg}"
        );
        assert!(msg.contains("no-msg"), "must include session id, got: {msg}");
    }

    #[test]
    fn test_lineage_root_has_single_entry() {
        let (store, _dir) = test_store();
        let s = session("root-only", "the root");
        let root_hash = store.save_commit(&s, None).unwrap();

        let lineage = store.lineage(&root_hash).unwrap();
        assert_eq!(lineage.len(), 1, "root commit must have lineage of length 1");
        assert_eq!(lineage[0].hash, root_hash);
    }
}
