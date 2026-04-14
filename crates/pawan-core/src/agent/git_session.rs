//! Git-backed session store — conversation trees with fork/lineage/leaves
//!
//! Each pawan session is a git commit in a bare repo. Branching conversations
//! = forking from any commit in the DAG.
//!
//! Uses libgit2 (git2-rs). Future: layer jj (Jujutsu) as porcelain on top.
//!
//! Inspired by Karpathy's AgentHub (bare git DAG) and Yegge's Beads (git-backed memory).

use crate::agent::session::Session;
use crate::{PawanError, Result};
use git2::{Oid, Repository, Signature, Time};
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
    repo: Repository,
}

impl GitSessionStore {
    /// Initialize or open the git session store
    pub fn init() -> Result<Self> {
        let path = Self::default_path()?;
        let repo = if path.join("HEAD").exists() {
            Repository::open_bare(&path)
                .map_err(|e| PawanError::Git(format!("Open repo: {}", e)))?
        } else {
            std::fs::create_dir_all(&path)
                .map_err(|e| PawanError::Git(format!("Create dir: {}", e)))?;
            Repository::init_bare(&path)
                .map_err(|e| PawanError::Git(format!("Init repo: {}", e)))?
        };
        Ok(Self { repo })
    }

    /// Open with a custom path (for testing)
    pub fn open(repo: Repository) -> Self {
        Self { repo }
    }

    fn default_path() -> Result<PathBuf> {
        let home = std::env::var("HOME").unwrap_or_else(|_| "/root".into());
        Ok(PathBuf::from(home).join(".pawan").join("repo"))
    }

    fn sig(&self) -> Signature<'_> {
        let now = chrono::Utc::now().timestamp();
        Signature::new("pawan", "pawan@localhost", &Time::new(now, 0))
            .expect("valid signature")
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

        let blob_oid = self.repo.blob(json.as_bytes())
            .map_err(|e| PawanError::Git(format!("Blob: {}", e)))?;

        let mut tb = self.repo.treebuilder(None)
            .map_err(|e| PawanError::Git(format!("Treebuilder: {}", e)))?;
        tb.insert("session.json", blob_oid, 0o100644)
            .map_err(|e| PawanError::Git(format!("Insert: {}", e)))?;
        let tree_oid = tb.write()
            .map_err(|e| PawanError::Git(format!("Write tree: {}", e)))?;
        let tree = self.repo.find_tree(tree_oid)
            .map_err(|e| PawanError::Git(format!("Find tree: {}", e)))?;

        let sig = self.sig();
        let msg = Self::commit_message(session);

        let parents: Vec<git2::Commit> = match parent_hash {
            Some(h) => {
                let oid = Oid::from_str(h)
                    .map_err(|e| PawanError::Git(format!("Bad hash: {}", e)))?;
                vec![self.repo.find_commit(oid)
                    .map_err(|e| PawanError::Git(format!("Parent not found: {}", e)))?]
            }
            None => vec![],
        };
        let parent_refs: Vec<&git2::Commit> = parents.iter().collect();

        let oid = self.repo.commit(None, &sig, &sig, &msg, &tree, &parent_refs)
            .map_err(|e| PawanError::Git(format!("Commit: {}", e)))?;

        // Update session ref
        let refname = format!("refs/sessions/{}", session.id);
        self.repo.reference(&refname, oid, true, &msg)
            .map_err(|e| PawanError::Git(format!("Ref: {}", e)))?;

        Ok(oid.to_string())
    }

    /// Load session from a commit hash
    pub fn load_commit(&self, hash: &str) -> Result<Session> {
        let oid = Oid::from_str(hash)
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
        let mut parent_set = HashSet::new();

        for &oid in &all_oids {
            if let Ok(c) = self.repo.find_commit(oid) {
                for p in c.parents() {
                    parent_set.insert(p.id());
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
        let mut oid = Oid::from_str(hash)
            .map_err(|e| PawanError::Git(format!("Bad hash: {}", e)))?;
        let mut chain = Vec::new();

        loop {
            let commit = self.repo.find_commit(oid)
                .map_err(|e| PawanError::Git(format!("Not found: {}", e)))?;
            chain.push(self.info(oid)?);
            if commit.parent_count() == 0 { break; }
            oid = commit.parent_id(0)
                .map_err(|e| PawanError::Git(format!("Parent: {}", e)))?;
        }
        Ok(chain)
    }

    /// Find all children of a commit
    pub fn children(&self, hash: &str) -> Result<Vec<CommitInfo>> {
        let target = Oid::from_str(hash)
            .map_err(|e| PawanError::Git(format!("Bad hash: {}", e)))?;
        let all = self.all_oids()?;
        let mut result = Vec::new();

        for &oid in &all {
            if let Ok(c) = self.repo.find_commit(oid) {
                for p in c.parents() {
                    if p.id() == target {
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
        if let Ok(refs) = self.repo.references_glob("refs/sessions/*") {
            for r in refs.flatten() {
                if let Some(oid) = r.target() {
                    if let Ok(info) = self.info(oid) {
                        sessions.push(info);
                    }
                }
            }
        }
        sessions.sort_by(|a, b| b.timestamp.cmp(&a.timestamp));
        Ok(sessions)
    }

    // -- internals --

    fn all_oids(&self) -> Result<Vec<Oid>> {
        let mut oids = Vec::new();
        let mut visited = HashSet::new();
        let mut stack = Vec::new();

        if let Ok(refs) = self.repo.references_glob("refs/sessions/*") {
            for r in refs.flatten() {
                if let Some(oid) = r.target() {
                    stack.push(oid);
                }
            }
        }

        while let Some(oid) = stack.pop() {
            if !visited.insert(oid) { continue; }
            oids.push(oid);
            if let Ok(c) = self.repo.find_commit(oid) {
                for p in c.parents() {
                    if !visited.contains(&p.id()) {
                        stack.push(p.id());
                    }
                }
            }
        }
        Ok(oids)
    }

    fn info(&self, oid: Oid) -> Result<CommitInfo> {
        let commit = self.repo.find_commit(oid)
            .map_err(|e| PawanError::Git(format!("Not found: {}", e)))?;
        let hash = oid.to_string();
        let (mc, model) = self.session_meta(&commit).unwrap_or((0, "unknown".into()));

        Ok(CommitInfo {
            short_hash: hash[..8].to_string(),
            hash,
            message: commit.message().unwrap_or("").to_string(),
            timestamp: commit.time().seconds(),
            message_count: mc,
            model,
        })
    }

    fn session_meta(&self, commit: &git2::Commit) -> Option<(usize, String)> {
        let tree = commit.tree().ok()?;
        let entry = tree.get_name("session.json")?;
        let blob = self.repo.find_blob(entry.id()).ok()?;
        let json = std::str::from_utf8(blob.content()).ok()?;
        let s: Session = serde_json::from_str(json).ok()?;
        Some((s.messages.len(), s.model))
    }

    fn session_from_commit(&self, commit: &git2::Commit) -> Result<Session> {
        let tree = commit.tree()
            .map_err(|e| PawanError::Git(format!("Tree: {}", e)))?;
        let entry = tree.get_name("session.json")
            .ok_or_else(|| PawanError::Git("No session.json".into()))?;
        let blob = self.repo.find_blob(entry.id())
            .map_err(|e| PawanError::Git(format!("Blob: {}", e)))?;
        let json = std::str::from_utf8(blob.content())
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
        let repo = Repository::init_bare(dir.path()).unwrap();
        (GitSessionStore { repo }, dir)
    }

    fn session(id: &str, msg: &str) -> Session {
        Session {
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
        // A session with zero messages → "new session" fallback
        let s = Session {
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
