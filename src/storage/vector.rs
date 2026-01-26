//! Vector storage (embeddings)
//!
//! Trait + SQLite implementation. The SQLite version uses sqlite-vec
//! for efficient similarity search.

use crate::config;
use crate::domain::Agent;
use rusqlite::{params, Connection};
use sqlite_vec::sqlite3_vec_init;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use zerocopy::AsBytes;

// ============================================================================
// Trait
// ============================================================================

/// Vector storage for embedding operations.
pub trait VectorStorage: Send + Sync {
    fn store_embedding(&self, key: &str, vector: &[f32], metadata: &str) -> Result<(), String>;
    fn search_by_vector(&self, query_vector: &[f32], limit: u32) -> Result<Vec<(String, String, f64)>, String>;
    fn delete_embedding(&self, key: &str) -> Result<bool, String>;
}

/// Open vector storage for an agent. Currently uses SQLite with sqlite-vec.
pub fn open_vector_storage(agent: &Agent) -> Result<Arc<dyn VectorStorage>, String> {
    Ok(Arc::new(SqliteVectorStorage::open(&agent.name)?))
}

// ============================================================================
// SQLite Implementation
// ============================================================================

/// SQLite-backed vector storage using sqlite-vec.
pub struct SqliteVectorStorage {
    conn: Arc<Mutex<Connection>>,
}

impl SqliteVectorStorage {
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

        Ok(SqliteVectorStorage {
            conn: Arc::new(Mutex::new(conn)),
        })
    }
}

impl VectorStorage for SqliteVectorStorage {
    fn store_embedding(&self, key: &str, vector: &[f32], metadata: &str) -> Result<(), String> {
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

    fn search_by_vector(&self, query_vector: &[f32], limit: u32) -> Result<Vec<(String, String, f64)>, String> {
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

    fn delete_embedding(&self, key: &str) -> Result<bool, String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        let rows_affected = conn.execute("DELETE FROM vec_items WHERE key = ?", params![key])
            .map_err(|e| format!("failed to delete embedding: {}", e))?;
        Ok(rows_affected > 0)
    }
}

// ============================================================================
// In-Memory Implementation
// ============================================================================

/// In-memory vector storage. Useful for testing.
/// Uses brute-force euclidean distance for similarity search.
pub struct InMemoryVectorStorage {
    items: Mutex<HashMap<String, (Vec<f32>, String)>>,
}

impl InMemoryVectorStorage {
    pub fn new() -> Self {
        InMemoryVectorStorage {
            items: Mutex::new(HashMap::new()),
        }
    }
}

impl VectorStorage for InMemoryVectorStorage {
    fn store_embedding(&self, key: &str, vector: &[f32], metadata: &str) -> Result<(), String> {
        if vector.len() != 768 {
            return Err(format!("expected 768 dimensions, got {}", vector.len()));
        }

        let mut items = self.items.lock().map_err(|e| e.to_string())?;
        items.insert(key.to_string(), (vector.to_vec(), metadata.to_string()));
        Ok(())
    }

    fn search_by_vector(&self, query_vector: &[f32], limit: u32) -> Result<Vec<(String, String, f64)>, String> {
        if query_vector.len() != 768 {
            return Err(format!("expected 768 dimensions, got {}", query_vector.len()));
        }

        let items = self.items.lock().map_err(|e| e.to_string())?;

        // Calculate distances and sort
        let mut results: Vec<(String, String, f64)> = items.iter()
            .map(|(key, (vector, metadata))| {
                let distance = euclidean_distance(query_vector, vector);
                (key.clone(), metadata.clone(), distance)
            })
            .collect();

        results.sort_by(|a, b| a.2.partial_cmp(&b.2).unwrap_or(std::cmp::Ordering::Equal));
        results.truncate(limit as usize);

        Ok(results)
    }

    fn delete_embedding(&self, key: &str) -> Result<bool, String> {
        let mut items = self.items.lock().map_err(|e| e.to_string())?;
        Ok(items.remove(key).is_some())
    }
}

fn euclidean_distance(a: &[f32], b: &[f32]) -> f64 {
    a.iter()
        .zip(b.iter())
        .map(|(x, y)| (x - y).powi(2) as f64)
        .sum::<f64>()
        .sqrt()
}
