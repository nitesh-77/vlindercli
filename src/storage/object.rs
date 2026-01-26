//! Object storage (virtual filesystem)
//!
//! Trait + SQLite implementation. Future backends (S3, filesystem, etc.)
//! would implement the same trait.

use crate::config;
use crate::domain::Agent;
use rusqlite::{params, Connection};
use std::sync::{Arc, Mutex};

// ============================================================================
// Trait
// ============================================================================

/// Object storage for file operations.
pub trait ObjectStorage: Send + Sync {
    fn put_file(&self, path: &str, content: &[u8]) -> Result<(), String>;
    fn get_file(&self, path: &str) -> Result<Option<Vec<u8>>, String>;
    fn delete_file(&self, path: &str) -> Result<bool, String>;
    fn list_files(&self, dir_path: &str) -> Result<Vec<String>, String>;
}

/// Open object storage for an agent. Currently uses SQLite.
pub fn open_object_storage(agent: &Agent) -> Result<Arc<dyn ObjectStorage>, String> {
    Ok(Arc::new(SqliteObjectStorage::open(&agent.name)?))
}

// ============================================================================
// SQLite Implementation
// ============================================================================

/// SQLite-backed object storage.
pub struct SqliteObjectStorage {
    conn: Arc<Mutex<Connection>>,
}

impl SqliteObjectStorage {
    /// Open object storage for an agent
    pub fn open(agent_name: &str) -> Result<Self, String> {
        let db_path = config::agent_db_path(agent_name);

        if let Some(parent) = db_path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("failed to create agent directory: {}", e))?;
        }

        let conn = Connection::open(&db_path)
            .map_err(|e| format!("failed to open database: {}", e))?;

        conn.execute_batch("PRAGMA journal_mode=WAL;")
            .map_err(|e| format!("failed to set WAL mode: {}", e))?;

        conn.execute(
            "CREATE TABLE IF NOT EXISTS files (
                path TEXT PRIMARY KEY,
                content BLOB NOT NULL,
                created_at INTEGER DEFAULT (unixepoch()),
                updated_at INTEGER DEFAULT (unixepoch())
            )",
            [],
        ).map_err(|e| format!("failed to create files table: {}", e))?;

        Ok(SqliteObjectStorage {
            conn: Arc::new(Mutex::new(conn)),
        })
    }
}

// ============================================================================
// In-Memory Implementation
// ============================================================================

use std::collections::HashMap;

/// In-memory object storage. Useful for testing.
pub struct InMemoryObjectStorage {
    files: Mutex<HashMap<String, Vec<u8>>>,
}

impl InMemoryObjectStorage {
    pub fn new() -> Self {
        InMemoryObjectStorage {
            files: Mutex::new(HashMap::new()),
        }
    }
}

impl ObjectStorage for InMemoryObjectStorage {
    fn put_file(&self, path: &str, content: &[u8]) -> Result<(), String> {
        let mut files = self.files.lock().map_err(|e| e.to_string())?;
        files.insert(path.to_string(), content.to_vec());
        Ok(())
    }

    fn get_file(&self, path: &str) -> Result<Option<Vec<u8>>, String> {
        let files = self.files.lock().map_err(|e| e.to_string())?;
        Ok(files.get(path).cloned())
    }

    fn delete_file(&self, path: &str) -> Result<bool, String> {
        let mut files = self.files.lock().map_err(|e| e.to_string())?;
        Ok(files.remove(path).is_some())
    }

    fn list_files(&self, dir_path: &str) -> Result<Vec<String>, String> {
        let files = self.files.lock().map_err(|e| e.to_string())?;
        let prefix = if dir_path == "/" || dir_path.is_empty() {
            "/".to_string()
        } else {
            format!("{}/", dir_path.trim_end_matches('/'))
        };

        Ok(files.keys()
            .filter(|p| p.starts_with(&prefix))
            .cloned()
            .collect())
    }
}

impl ObjectStorage for SqliteObjectStorage {
    fn put_file(&self, path: &str, content: &[u8]) -> Result<(), String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        conn.execute(
            "INSERT OR REPLACE INTO files (path, content, updated_at) VALUES (?, ?, unixepoch())",
            params![path, content],
        ).map_err(|e| format!("failed to write file: {}", e))?;
        Ok(())
    }

    fn get_file(&self, path: &str) -> Result<Option<Vec<u8>>, String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        let mut stmt = conn.prepare("SELECT content FROM files WHERE path = ?")
            .map_err(|e| format!("failed to prepare query: {}", e))?;

        let mut rows = stmt.query(params![path])
            .map_err(|e| format!("failed to query file: {}", e))?;

        if let Some(row) = rows.next().map_err(|e| format!("failed to read row: {}", e))? {
            let content: Vec<u8> = row.get(0).map_err(|e| format!("failed to get content: {}", e))?;
            Ok(Some(content))
        } else {
            Ok(None)
        }
    }

    fn delete_file(&self, path: &str) -> Result<bool, String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        let rows_affected = conn.execute("DELETE FROM files WHERE path = ?", params![path])
            .map_err(|e| format!("failed to delete file: {}", e))?;
        Ok(rows_affected > 0)
    }

    fn list_files(&self, dir_path: &str) -> Result<Vec<String>, String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        let pattern = if dir_path == "/" || dir_path.is_empty() {
            "/%".to_string()
        } else {
            format!("{}/%", dir_path.trim_end_matches('/'))
        };

        let mut stmt = conn.prepare("SELECT path FROM files WHERE path LIKE ?")
            .map_err(|e| format!("failed to prepare query: {}", e))?;

        let rows = stmt.query_map(params![pattern], |row| row.get(0))
            .map_err(|e| format!("failed to list files: {}", e))?;

        let mut files = Vec::new();
        for path_result in rows {
            files.push(path_result.map_err(|e| format!("failed to get path: {}", e))?);
        }
        Ok(files)
    }
}
