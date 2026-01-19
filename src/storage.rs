//! MIT-only storage for agents
//!
//! Uses rusqlite (bundled) + sqlite-vec for file and vector storage.
//! Single .db file per agent containing:
//! - files table for virtual filesystem
//! - vec_items virtual table for embeddings (sqlite-vec)

use crate::config;
use rusqlite::{params, Connection};
use sqlite_vec::sqlite3_vec_init;
use std::sync::{Arc, Mutex};
use zerocopy::AsBytes;

/// Storage handle for an agent
pub struct Storage {
    conn: Arc<Mutex<Connection>>,
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

        // Register sqlite-vec extension
        unsafe {
            rusqlite::ffi::sqlite3_auto_extension(Some(std::mem::transmute(
                sqlite3_vec_init as *const (),
            )));
        }

        let conn = Connection::open(&db_path)
            .map_err(|e| format!("failed to open database: {}", e))?;

        // Enable WAL mode for better concurrency
        conn.execute_batch("PRAGMA journal_mode=WAL;")
            .map_err(|e| format!("failed to set WAL mode: {}", e))?;

        // Initialize files table
        conn.execute(
            "CREATE TABLE IF NOT EXISTS files (
                path TEXT PRIMARY KEY,
                content BLOB NOT NULL,
                created_at INTEGER DEFAULT (unixepoch()),
                updated_at INTEGER DEFAULT (unixepoch())
            )",
            [],
        ).map_err(|e| format!("failed to create files table: {}", e))?;

        // Initialize vec_items virtual table (sqlite-vec)
        conn.execute(
            "CREATE VIRTUAL TABLE IF NOT EXISTS vec_items USING vec0(
                key TEXT PRIMARY KEY,
                embedding float[768],
                metadata TEXT
            )",
            [],
        ).map_err(|e| format!("failed to create vec_items table: {}", e))?;

        Ok(Storage {
            conn: Arc::new(Mutex::new(conn)),
        })
    }

    /// Write a file to storage
    pub fn put_file(&self, path: &str, content: &[u8]) -> Result<(), String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        conn.execute(
            "INSERT OR REPLACE INTO files (path, content, updated_at) VALUES (?, ?, unixepoch())",
            params![path, content],
        ).map_err(|e| format!("failed to write file: {}", e))?;
        Ok(())
    }

    /// Read a file from storage
    pub fn get_file(&self, path: &str) -> Result<Option<Vec<u8>>, String> {
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

    /// Delete a file from storage
    pub fn delete_file(&self, path: &str) -> Result<bool, String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        let rows_affected = conn.execute("DELETE FROM files WHERE path = ?", params![path])
            .map_err(|e| format!("failed to delete file: {}", e))?;
        Ok(rows_affected > 0)
    }

    /// List files in a directory
    pub fn list_files(&self, dir_path: &str) -> Result<Vec<String>, String> {
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

    /// Store an embedding vector (768 dimensions)
    pub fn store_embedding(&self, key: &str, vector: &[f32], metadata: &str) -> Result<(), String> {
        if vector.len() != 768 {
            return Err(format!("expected 768 dimensions, got {}", vector.len()));
        }

        let conn = self.conn.lock().map_err(|e| e.to_string())?;

        // sqlite-vec virtual tables don't support INSERT OR REPLACE
        // Delete first if exists, then insert
        let _ = conn.execute("DELETE FROM vec_items WHERE key = ?", params![key]);

        conn.execute(
            "INSERT INTO vec_items (key, embedding, metadata) VALUES (?, ?, ?)",
            params![key, vector.as_bytes(), metadata],
        ).map_err(|e| format!("failed to store embedding: {}", e))?;
        Ok(())
    }

    /// Search for similar vectors using sqlite-vec
    pub fn search_by_vector(&self, query_vector: &[f32], limit: u32) -> Result<Vec<(String, String, f64)>, String> {
        if query_vector.len() != 768 {
            return Err(format!("expected 768 dimensions, got {}", query_vector.len()));
        }

        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        let mut stmt = conn.prepare(
            "SELECT key, metadata, distance
             FROM vec_items
             WHERE embedding MATCH ?
             ORDER BY distance
             LIMIT ?"
        ).map_err(|e| format!("failed to prepare search query: {}", e))?;

        let rows = stmt.query_map(params![query_vector.as_bytes(), limit], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, f64>(2)?,
            ))
        }).map_err(|e| format!("failed to search: {}", e))?;

        let mut results = Vec::new();
        for result in rows {
            results.push(result.map_err(|e| format!("failed to get result: {}", e))?);
        }
        Ok(results)
    }

    /// Delete an embedding by key
    pub fn delete_embedding(&self, key: &str) -> Result<bool, String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        let rows_affected = conn.execute("DELETE FROM vec_items WHERE key = ?", params![key])
            .map_err(|e| format!("failed to delete embedding: {}", e))?;
        Ok(rows_affected > 0)
    }
}
