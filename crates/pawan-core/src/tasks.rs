//! Beads-style task tracking — hash IDs, dependency graphs, memory decay
//!
//! Inspired by Steve Yegge's Beads. Each task ("bead") has a content-addressable
//! hash ID (bd-XXXXXXXX), can depend on other beads, and supports memory decay
//! (old closed tasks get summarized to save context window).
//!
//! Storage: SQLite at ~/.pawan/beads.db

use crate::{PawanError, Result};
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// 8-char hash prefix, displayed as bd-{hash}
///
/// BeadId is a content-addressable identifier for beads (tasks/issues).
/// It's generated from the title and creation timestamp using a hash function.
/// The ID is represented as an 8-character hexadecimal string and displayed with the "bd-" prefix.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct BeadId(pub String);

impl BeadId {
    /// Generate a new BeadId from a title and timestamp
    ///
    /// # Arguments
    /// * `title` - The title of the task/bead
    /// * `created_at` - The creation timestamp in RFC3339 format
    ///
    /// # Returns
    /// A new BeadId with an 8-character hash prefix
    pub fn generate(title: &str, created_at: &str) -> Self {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};
        let mut hasher = DefaultHasher::new();
        title.hash(&mut hasher);
        created_at.hash(&mut hasher);
        let hash = hasher.finish();
        Self(format!("{:08x}", hash & 0xFFFFFFFF))
    }

    /// Display the BeadId in the standard format "bd-XXXXXXXX"
    ///
    /// # Returns
    /// A formatted string representation of the BeadId
    pub fn display(&self) -> String {
        format!("bd-{}", self.0)
    }

    /// Parse a BeadId from a string representation
    ///
    /// Accepts both "bd-XXXXXXXX" and "XXXXXXXX" formats
    ///
    /// # Arguments
    /// * `s` - The string to parse
    ///
    /// # Returns
    /// A BeadId parsed from the string
    pub fn parse(s: &str) -> Self {
        Self(s.strip_prefix("bd-").unwrap_or(s).to_string())
    }
}

impl std::fmt::Display for BeadId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "bd-{}", self.0)
    }
}

/// Task status
///
/// Represents the current state of a bead (task/issue):
/// - `Open`: Task is created but not yet started
/// - `InProgress`: Task is actively being worked on
/// - `Closed`: Task is completed or abandoned
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BeadStatus {
    Open,
    InProgress,
    Closed,
}

impl BeadStatus {
    /// Convert the BeadStatus to a string representation
    ///
    /// # Returns
    /// A string slice representing the status ("open", "in_progress", or "closed")
    pub fn to_str(&self) -> &'static str {
        match self {
            Self::Open => "open",
            Self::InProgress => "in_progress",
            Self::Closed => "closed",
        }
    }

}

impl std::str::FromStr for BeadStatus {
    type Err = std::convert::Infallible;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        Ok(match s {
            "in_progress" => Self::InProgress,
            "closed" => Self::Closed,
            _ => Self::Open,
        })
    }
}

/// A single bead (task/issue)
///
/// Represents a task or issue in the beads system with the following properties:
/// - `id`: Unique identifier for the bead
/// - `title`: Short description of the task
/// - `description`: Optional detailed description
/// - `status`: Current status (Open, InProgress, Closed)
/// - `priority`: Priority level (0 = critical, 4 = backlog)
/// - `created_at`: RFC3339 timestamp when the bead was created
/// - `updated_at`: RFC3339 timestamp when the bead was last updated
/// - `closed_at`: Optional RFC3339 timestamp when the bead was closed
/// - `closed_reason`: Optional reason for closing the bead
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Bead {
    pub id: BeadId,
    pub title: String,
    pub description: Option<String>,
    pub status: BeadStatus,
    /// 0 = critical, 4 = backlog
    pub priority: u8,
    pub created_at: String,
    pub updated_at: String,
    pub closed_at: Option<String>,
    pub closed_reason: Option<String>,
}

/// SQLite-backed bead store
///
/// BeadStore provides persistent storage for beads (tasks/issues) using SQLite.
/// It handles creation, retrieval, updating, and deletion of beads, as well as
/// managing their dependencies and status transitions.
///
/// The store is located at `~/.pawan/beads.db` by default.
///
/// # Features
/// - Create, read, update, and delete beads
/// - Query beads by status, priority, or search term
/// - Manage bead dependencies
/// - Track bead history and transitions
/// - Efficient indexing for large numbers of beads
pub struct BeadStore {
    conn: Connection,
}

impl BeadStore {
    /// Open or create the bead store
    pub fn open() -> Result<Self> {
        let path = Self::db_path()?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| PawanError::Config(format!("Create dir: {}", e)))?;
        }
        let conn = Connection::open(&path)
            .map_err(|e| PawanError::Config(format!("Open DB: {}", e)))?;
        let store = Self { conn };
        store.init_schema()?;
        Ok(store)
    }

    /// Open with custom connection (for testing)
    pub fn with_conn(conn: Connection) -> Result<Self> {
        let store = Self { conn };
        store.init_schema()?;
        Ok(store)
    }

    fn db_path() -> Result<PathBuf> {
        let home = std::env::var("HOME").unwrap_or_else(|_| "/root".into());
        Ok(PathBuf::from(home).join(".pawan").join("beads.db"))
    }

    fn init_schema(&self) -> Result<()> {
        self.conn
            .execute_batch(
                "CREATE TABLE IF NOT EXISTS beads (
                    id TEXT PRIMARY KEY,
                    title TEXT NOT NULL,
                    description TEXT,
                    status TEXT NOT NULL DEFAULT 'open',
                    priority INTEGER NOT NULL DEFAULT 2,
                    created_at TEXT NOT NULL,
                    updated_at TEXT NOT NULL,
                    closed_at TEXT,
                    closed_reason TEXT
                );
                CREATE TABLE IF NOT EXISTS deps (
                    bead_id TEXT NOT NULL,
                    depends_on TEXT NOT NULL,
                    PRIMARY KEY (bead_id, depends_on),
                    FOREIGN KEY (bead_id) REFERENCES beads(id),
                    FOREIGN KEY (depends_on) REFERENCES beads(id)
                );
                CREATE TABLE IF NOT EXISTS archives (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    summary TEXT NOT NULL,
                    bead_count INTEGER NOT NULL,
                    archived_at TEXT NOT NULL
                );",
            )
            .map_err(|e| PawanError::Config(format!("Schema: {}", e)))?;
        Ok(())
    }

    /// Create a new bead
    pub fn create(&self, title: &str, description: Option<&str>, priority: u8) -> Result<Bead> {
        let now = chrono::Utc::now().to_rfc3339();
        let id = BeadId::generate(title, &now);

        self.conn
            .execute(
                "INSERT INTO beads (id, title, description, status, priority, created_at, updated_at)
                 VALUES (?1, ?2, ?3, 'open', ?4, ?5, ?6)",
                params![id.0, title, description, priority, now, now],
            )
            .map_err(|e| PawanError::Config(format!("Insert: {}", e)))?;

        Ok(Bead {
            id,
            title: title.into(),
            description: description.map(String::from),
            status: BeadStatus::Open,
            priority,
            created_at: now.clone(),
            updated_at: now,
            closed_at: None,
            closed_reason: None,
        })
    }

    /// Get a bead by ID
    pub fn get(&self, id: &BeadId) -> Result<Bead> {
        self.conn
            .query_row(
                "SELECT id, title, description, status, priority, created_at, updated_at, closed_at, closed_reason
                 FROM beads WHERE id = ?1",
                params![id.0],
                |row| {
                    Ok(Bead {
                        id: BeadId(row.get::<_, String>(0)?),
                        title: row.get(1)?,
                        description: row.get(2)?,
                        status: row.get::<_, String>(3)?.parse().unwrap_or(BeadStatus::Open),
                        priority: row.get(4)?,
                        created_at: row.get(5)?,
                        updated_at: row.get(6)?,
                        closed_at: row.get(7)?,
                        closed_reason: row.get(8)?,
                    })
                },
            )
            .map_err(|e| PawanError::NotFound(format!("Bead {}: {}", id, e)))
    }

    /// Update a bead's fields
    pub fn update(
        &self,
        id: &BeadId,
        title: Option<&str>,
        status: Option<BeadStatus>,
        priority: Option<u8>,
    ) -> Result<()> {
        let now = chrono::Utc::now().to_rfc3339();

        if let Some(t) = title {
            self.conn
                .execute(
                    "UPDATE beads SET title = ?1, updated_at = ?2 WHERE id = ?3",
                    params![t, now, id.0],
                )
                .map_err(|e| PawanError::Config(format!("Update title: {}", e)))?;
        }
        if let Some(s) = status {
            self.conn
                .execute(
                    "UPDATE beads SET status = ?1, updated_at = ?2 WHERE id = ?3",
                    params![s.to_str(), now, id.0],
                )
                .map_err(|e| PawanError::Config(format!("Update status: {}", e)))?;
        }
        if let Some(p) = priority {
            self.conn
                .execute(
                    "UPDATE beads SET priority = ?1, updated_at = ?2 WHERE id = ?3",
                    params![p, now, id.0],
                )
                .map_err(|e| PawanError::Config(format!("Update priority: {}", e)))?;
        }
        Ok(())
    }

    /// Close a bead with optional reason
    pub fn close(&self, id: &BeadId, reason: Option<&str>) -> Result<()> {
        let now = chrono::Utc::now().to_rfc3339();
        self.conn
            .execute(
                "UPDATE beads SET status = 'closed', closed_at = ?1, closed_reason = ?2, updated_at = ?3 WHERE id = ?4",
                params![now, reason, now, id.0],
            )
            .map_err(|e| PawanError::Config(format!("Close: {}", e)))?;
        Ok(())
    }

    /// Delete a bead
    pub fn delete(&self, id: &BeadId) -> Result<()> {
        self.conn
            .execute("DELETE FROM deps WHERE bead_id = ?1 OR depends_on = ?1", params![id.0])
            .map_err(|e| PawanError::Config(format!("Delete deps: {}", e)))?;
        self.conn
            .execute("DELETE FROM beads WHERE id = ?1", params![id.0])
            .map_err(|e| PawanError::Config(format!("Delete: {}", e)))?;
        Ok(())
    }

    /// List beads with optional filters
    pub fn list(
        &self,
        status: Option<&str>,
        max_priority: Option<u8>,
    ) -> Result<Vec<Bead>> {
        let mut sql = "SELECT id, title, description, status, priority, created_at, updated_at, closed_at, closed_reason FROM beads WHERE 1=1".to_string();
        let mut bind_vals: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();

        if let Some(s) = status {
            sql.push_str(&format!(" AND status = ?{}", bind_vals.len() + 1));
            bind_vals.push(Box::new(s.to_string()));
        }
        if let Some(p) = max_priority {
            sql.push_str(&format!(" AND priority <= ?{}", bind_vals.len() + 1));
            bind_vals.push(Box::new(p));
        }
        sql.push_str(" ORDER BY priority ASC, updated_at DESC");

        let params_refs: Vec<&dyn rusqlite::types::ToSql> = bind_vals.iter().map(|b| b.as_ref()).collect();

        let mut stmt = self.conn.prepare(&sql)
            .map_err(|e| PawanError::Config(format!("Prepare: {}", e)))?;

        let beads = stmt
            .query_map(params_refs.as_slice(), |row| {
                Ok(Bead {
                    id: BeadId(row.get::<_, String>(0)?),
                    title: row.get(1)?,
                    description: row.get(2)?,
                    status: row.get::<_, String>(3)?.parse().unwrap_or(BeadStatus::Open),
                    priority: row.get(4)?,
                    created_at: row.get(5)?,
                    updated_at: row.get(6)?,
                    closed_at: row.get(7)?,
                    closed_reason: row.get(8)?,
                })
            })
            .map_err(|e| PawanError::Config(format!("Query: {}", e)))?
            .filter_map(|r| r.ok())
            .collect();

        Ok(beads)
    }

    /// Add a dependency: bead_id depends on depends_on
    pub fn dep_add(&self, bead_id: &BeadId, depends_on: &BeadId) -> Result<()> {
        self.conn
            .execute(
                "INSERT OR IGNORE INTO deps (bead_id, depends_on) VALUES (?1, ?2)",
                params![bead_id.0, depends_on.0],
            )
            .map_err(|e| PawanError::Config(format!("Dep add: {}", e)))?;
        Ok(())
    }

    /// Remove a dependency
    pub fn dep_remove(&self, bead_id: &BeadId, depends_on: &BeadId) -> Result<()> {
        self.conn
            .execute(
                "DELETE FROM deps WHERE bead_id = ?1 AND depends_on = ?2",
                params![bead_id.0, depends_on.0],
            )
            .map_err(|e| PawanError::Config(format!("Dep rm: {}", e)))?;
        Ok(())
    }

    /// Get dependencies of a bead
    pub fn deps(&self, bead_id: &BeadId) -> Result<Vec<BeadId>> {
        let mut stmt = self.conn
            .prepare("SELECT depends_on FROM deps WHERE bead_id = ?1")
            .map_err(|e| PawanError::Config(format!("Prepare: {}", e)))?;

        let ids = stmt
            .query_map(params![bead_id.0], |row| {
                Ok(BeadId(row.get::<_, String>(0)?))
            })
            .map_err(|e| PawanError::Config(format!("Query: {}", e)))?
            .filter_map(|r| r.ok())
            .collect();

        Ok(ids)
    }

    /// Ready beads: Open beads whose ALL dependencies are Closed
    pub fn ready(&self) -> Result<Vec<Bead>> {
        let all_open = self.list(Some("open"), None)?;
        let mut ready = Vec::new();

        for bead in all_open {
            let deps = self.deps(&bead.id)?;
            let all_closed = deps.iter().all(|dep_id| {
                self.get(dep_id)
                    .map(|b| b.status == BeadStatus::Closed)
                    .unwrap_or(true) // missing dep = treat as closed
            });
            if all_closed {
                ready.push(bead);
            }
        }

        Ok(ready)
    }

    /// Memory decay: summarize closed beads older than max_age_days into archive
    pub fn memory_decay(&self, max_age_days: u64) -> Result<usize> {
        let cutoff = chrono::Utc::now() - chrono::Duration::days(max_age_days as i64);
        let cutoff_str = cutoff.to_rfc3339();

        // Find old closed beads
        let mut stmt = self.conn
            .prepare(
                "SELECT id, title, closed_reason FROM beads
                 WHERE status = 'closed' AND closed_at < ?1
                 ORDER BY closed_at ASC",
            )
            .map_err(|e| PawanError::Config(format!("Prepare: {}", e)))?;

        let old_beads: Vec<(String, String, Option<String>)> = stmt
            .query_map(params![cutoff_str], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, Option<String>>(2)?,
                ))
            })
            .map_err(|e| PawanError::Config(format!("Query: {}", e)))?
            .filter_map(|r| r.ok())
            .collect();

        if old_beads.is_empty() {
            return Ok(0);
        }

        let count = old_beads.len();

        // Build summary
        let summary_lines: Vec<String> = old_beads
            .iter()
            .map(|(id, title, reason)| {
                let r = reason.as_deref().unwrap_or("done");
                format!("- bd-{}: {} ({})", id, title, r)
            })
            .collect();
        let summary = format!(
            "Archived {} beads (before {}):\n{}",
            count,
            cutoff_str,
            summary_lines.join("\n")
        );

        let now = chrono::Utc::now().to_rfc3339();
        self.conn
            .execute(
                "INSERT INTO archives (summary, bead_count, archived_at) VALUES (?1, ?2, ?3)",
                params![summary, count, now],
            )
            .map_err(|e| PawanError::Config(format!("Archive: {}", e)))?;

        // Delete archived beads
        for (id, _, _) in &old_beads {
            self.conn
                .execute("DELETE FROM deps WHERE bead_id = ?1 OR depends_on = ?1", params![id])
                .ok();
            self.conn
                .execute("DELETE FROM beads WHERE id = ?1", params![id])
                .ok();
        }

        Ok(count)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_store() -> BeadStore {
        let conn = Connection::open_in_memory().unwrap();
        BeadStore::with_conn(conn).unwrap()
    }

    #[test]
    fn create_and_get() {
        let store = test_store();
        let bead = store.create("Fix bug", Some("It's broken"), 1).unwrap();
        assert!(bead.id.0.len() == 8);
        assert_eq!(bead.title, "Fix bug");
        assert_eq!(bead.priority, 1);

        let loaded = store.get(&bead.id).unwrap();
        assert_eq!(loaded.title, "Fix bug");
    }

    #[test]
    fn list_filters() {
        let store = test_store();
        store.create("A", None, 0).unwrap();
        store.create("B", None, 2).unwrap();
        let c = store.create("C", None, 4).unwrap();
        store.close(&c.id, Some("done")).unwrap();

        let all = store.list(None, None).unwrap();
        assert_eq!(all.len(), 3);

        let open = store.list(Some("open"), None).unwrap();
        assert_eq!(open.len(), 2);

        let critical = store.list(None, Some(1)).unwrap();
        assert_eq!(critical.len(), 1);
        assert_eq!(critical[0].title, "A");
    }

    #[test]
    fn deps_and_ready() {
        let store = test_store();
        let a = store.create("Task A", None, 1).unwrap();
        let b = store.create("Task B", None, 1).unwrap();
        let c = store.create("Task C", None, 1).unwrap();

        // C depends on A and B
        store.dep_add(&c.id, &a.id).unwrap();
        store.dep_add(&c.id, &b.id).unwrap();

        // Only A and B should be ready (C is blocked)
        let ready = store.ready().unwrap();
        assert_eq!(ready.len(), 2);
        let ready_ids: Vec<&str> = ready.iter().map(|b| b.id.0.as_str()).collect();
        assert!(!ready_ids.contains(&c.id.0.as_str()));

        // Close A — C still blocked by B
        store.close(&a.id, None).unwrap();
        let ready = store.ready().unwrap();
        assert_eq!(ready.len(), 1); // only B
        assert_eq!(ready[0].id, b.id);

        // Close B — C now ready
        store.close(&b.id, None).unwrap();
        let ready = store.ready().unwrap();
        assert_eq!(ready.len(), 1);
        assert_eq!(ready[0].id, c.id);
    }

    #[test]
    fn close_and_delete() {
        let store = test_store();
        let bead = store.create("Temp", None, 3).unwrap();

        store.close(&bead.id, Some("no longer needed")).unwrap();
        let loaded = store.get(&bead.id).unwrap();
        assert_eq!(loaded.status, BeadStatus::Closed);
        assert_eq!(loaded.closed_reason.as_deref(), Some("no longer needed"));

        store.delete(&bead.id).unwrap();
        assert!(store.get(&bead.id).is_err());
    }

    #[test]
    fn memory_decay_archives() {
        let store = test_store();

        // Create and close a bead with old timestamp
        let bead = store.create("Old task", None, 2).unwrap();
        let old_time = (chrono::Utc::now() - chrono::Duration::days(60)).to_rfc3339();
        store.conn
            .execute(
                "UPDATE beads SET status = 'closed', closed_at = ?1 WHERE id = ?2",
                params![old_time, bead.id.0],
            )
            .unwrap();

        // Create a recent closed bead (should NOT be decayed)
        let recent = store.create("Recent task", None, 2).unwrap();
        store.close(&recent.id, Some("just done")).unwrap();

        // Decay beads older than 30 days
        let count = store.memory_decay(30).unwrap();
        assert_eq!(count, 1);

        // Old bead should be gone
        assert!(store.get(&bead.id).is_err());
        // Recent bead should remain
        assert!(store.get(&recent.id).is_ok());

        // Archive should exist
        let summary: String = store.conn
            .query_row("SELECT summary FROM archives ORDER BY id DESC LIMIT 1", [], |r| r.get(0))
            .unwrap();
        assert!(summary.contains("Old task"));
    }
}
