//! Vector storage (embeddings)

use crate::config;
use rusqlite::{params, Connection};
use sqlite_vec::sqlite3_vec_init;
use std::sync::{Arc, Mutex};
use zerocopy::AsBytes;

/// Vector storage for embedding operations
pub struct VectorStorage {
    conn: Arc<Mutex<Connection>>,
}

impl VectorStorage {
    /// Open vector storage for an agent
    pub fn open(agent_name: &str) -> Result<Self, String> {
        let db_path = config::agent_db_path(agent_name);

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

        conn.execute_batch("PRAGMA journal_mode=WAL;")
            .map_err(|e| format!("failed to set WAL mode: {}", e))?;

        conn.execute(
            "CREATE VIRTUAL TABLE IF NOT EXISTS vec_items USING vec0(
                key TEXT PRIMARY KEY,
                embedding float[768],
                metadata TEXT
            )",
            [],
        ).map_err(|e| format!("failed to create vec_items table: {}", e))?;

        Ok(VectorStorage {
            conn: Arc::new(Mutex::new(conn)),
        })
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
