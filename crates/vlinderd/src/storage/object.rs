//! Object storage (virtual filesystem)
//!
//! SQLite implementation of the ObjectStorage trait.
//! The trait is defined in the domain module.

use crate::config;
use crate::domain::ObjectStorage;
use rusqlite::{params, Connection};
use std::sync::{Arc, Mutex};

// ============================================================================
// SQLite Implementation
// ============================================================================

/// SQLite-backed object storage.
pub struct SqliteObjectStorage {
    conn: Arc<Mutex<Connection>>,
}

impl SqliteObjectStorage {
    /// Open object storage for an agent (derives path from agent name).
    pub fn open(agent_name: &str) -> Result<Self, String> {
        Self::open_at(&config::agent_db_path(agent_name))
    }

    /// Open object storage at a specific path.
    pub fn open_at(db_path: &std::path::Path) -> Result<Self, String> {
        if let Some(parent) = db_path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("failed to create storage directory: {}", e))?;
        }

        let conn = Connection::open(db_path)
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
