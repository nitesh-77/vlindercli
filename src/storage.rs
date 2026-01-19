//! MIT-only storage implementing AgentFS specification
//!
//! Uses libsql (core only, no GPL deps) for file storage.
//! Single .db file per agent containing:
//! - files table (AgentFS spec)

use crate::config;
use libsql::{Builder, Connection};
use std::sync::Arc;
use tokio::runtime::Runtime as TokioRuntime;
use tokio::sync::Mutex;

/// Storage handle for an agent (MIT-only implementation)
pub struct Storage {
    conn: Arc<Mutex<Connection>>,
    runtime: Arc<TokioRuntime>,
}

impl Storage {
    /// Open storage for an agent (creates DB if doesn't exist)
    pub fn open(agent_name: &str) -> Result<Self, String> {
        let db_path = config::agent_db_path(agent_name);

        // Ensure agent directory exists
        if let Some(parent) = db_path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("failed to create agent directory: {}", e))?;
        }

        let runtime = Arc::new(
            TokioRuntime::new()
                .map_err(|e| format!("failed to create tokio runtime: {}", e))?
        );

        let conn = runtime.block_on(async {
            let db = Builder::new_local(db_path)
                .build()
                .await
                .map_err(|e| format!("failed to open database: {}", e))?;

            let conn = db.connect()
                .map_err(|e| format!("failed to connect: {}", e))?;

            // Enable WAL mode for better concurrency
            // Use query since PRAGMA returns rows
            let _ = conn.query("PRAGMA journal_mode=WAL;", ())
                .await
                .map_err(|e| format!("failed to set WAL mode: {}", e))?;

            // Initialize AgentFS-spec files table
            conn.execute(
                "CREATE TABLE IF NOT EXISTS files (
                    path TEXT PRIMARY KEY,
                    content BLOB NOT NULL,
                    created_at INTEGER DEFAULT (unixepoch()),
                    updated_at INTEGER DEFAULT (unixepoch())
                )",
                (),
            ).await.map_err(|e| format!("failed to create files table: {}", e))?;

            Ok::<Connection, String>(conn)
        })?;

        Ok(Storage {
            conn: Arc::new(Mutex::new(conn)),
            runtime,
        })
    }

    /// Write a file to storage (AgentFS spec)
    pub fn put_file(&self, path: &str, content: &[u8]) -> Result<(), String> {
        self.runtime.block_on(async {
            let conn = self.conn.lock().await;
            conn.execute(
                "INSERT OR REPLACE INTO files (path, content, updated_at) VALUES (?, ?, unixepoch())",
                libsql::params![path, content],
            )
            .await
            .map_err(|e| format!("failed to write file: {}", e))?;
            Ok(())
        })
    }

    /// Read a file from storage (AgentFS spec)
    pub fn get_file(&self, path: &str) -> Result<Option<Vec<u8>>, String> {
        self.runtime.block_on(async {
            let conn = self.conn.lock().await;
            let mut rows = conn
                .query("SELECT content FROM files WHERE path = ?", libsql::params![path])
                .await
                .map_err(|e| format!("failed to query file: {}", e))?;

            if let Some(row) = rows.next().await.map_err(|e| format!("failed to read row: {}", e))? {
                let content: Vec<u8> = row.get(0).map_err(|e| format!("failed to get content: {}", e))?;
                Ok(Some(content))
            } else {
                Ok(None)
            }
        })
    }

    /// Delete a file from storage (AgentFS spec)
    pub fn delete_file(&self, path: &str) -> Result<bool, String> {
        self.runtime.block_on(async {
            let conn = self.conn.lock().await;
            let rows_affected = conn
                .execute("DELETE FROM files WHERE path = ?", libsql::params![path])
                .await
                .map_err(|e| format!("failed to delete file: {}", e))?;
            Ok(rows_affected > 0)
        })
    }

    /// List files in a directory (AgentFS spec)
    pub fn list_files(&self, dir_path: &str) -> Result<Vec<String>, String> {
        self.runtime.block_on(async {
            let conn = self.conn.lock().await;
            let pattern = if dir_path == "/" || dir_path.is_empty() {
                "/%".to_string()
            } else {
                format!("{}/%", dir_path.trim_end_matches('/'))
            };

            let mut rows = conn
                .query("SELECT path FROM files WHERE path LIKE ?", libsql::params![pattern])
                .await
                .map_err(|e| format!("failed to list files: {}", e))?;

            let mut files = Vec::new();
            while let Some(row) = rows.next().await.map_err(|e| format!("failed to read row: {}", e))? {
                let path: String = row.get(0).map_err(|e| format!("failed to get path: {}", e))?;
                files.push(path);
            }
            Ok(files)
        })
    }
}
