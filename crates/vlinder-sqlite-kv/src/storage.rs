//! SQLite-backed object storage (virtual filesystem).
//!
//! Concrete type with inherent methods — no `ObjectStorage` trait.
//! Moved from vlinderd/src/storage/object.rs.

use rusqlite::{params, Connection};
use std::sync::{Arc, Mutex};

/// SQLite-backed object storage for agent files.
pub struct SqliteObjectStorage {
    conn: Arc<Mutex<Connection>>,
}

impl SqliteObjectStorage {
    /// Open object storage at a specific path.
    pub fn open_at(db_path: &std::path::Path) -> Result<Self, String> {
        if let Some(parent) = db_path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("failed to create storage directory: {e}"))?;
        }

        let conn =
            Connection::open(db_path).map_err(|e| format!("failed to open database: {e}"))?;

        conn.execute_batch("PRAGMA journal_mode=WAL;")
            .map_err(|e| format!("failed to set WAL mode: {e}"))?;

        conn.execute(
            "CREATE TABLE IF NOT EXISTS files (
                path TEXT PRIMARY KEY,
                content BLOB NOT NULL,
                created_at INTEGER DEFAULT (unixepoch()),
                updated_at INTEGER DEFAULT (unixepoch())
            )",
            [],
        )
        .map_err(|e| format!("failed to create files table: {e}"))?;

        Ok(SqliteObjectStorage {
            conn: Arc::new(Mutex::new(conn)),
        })
    }

    pub fn put_file(&self, path: &str, content: &[u8]) -> Result<(), String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        conn.execute(
            "INSERT OR REPLACE INTO files (path, content, updated_at) VALUES (?, ?, unixepoch())",
            params![path, content],
        )
        .map_err(|e| format!("failed to write file: {e}"))?;
        Ok(())
    }

    pub fn get_file(&self, path: &str) -> Result<Option<Vec<u8>>, String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        let mut stmt = conn
            .prepare("SELECT content FROM files WHERE path = ?")
            .map_err(|e| format!("failed to prepare query: {e}"))?;

        let mut rows = stmt
            .query(params![path])
            .map_err(|e| format!("failed to query file: {e}"))?;

        if let Some(row) = rows
            .next()
            .map_err(|e| format!("failed to read row: {e}"))?
        {
            let content: Vec<u8> = row
                .get(0)
                .map_err(|e| format!("failed to get content: {e}"))?;
            Ok(Some(content))
        } else {
            Ok(None)
        }
    }

    pub fn delete_file(&self, path: &str) -> Result<bool, String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        let rows_affected = conn
            .execute("DELETE FROM files WHERE path = ?", params![path])
            .map_err(|e| format!("failed to delete file: {e}"))?;
        Ok(rows_affected > 0)
    }

    pub fn list_files(&self, dir_path: &str) -> Result<Vec<String>, String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        let pattern = if dir_path == "/" || dir_path.is_empty() {
            "/%".to_string()
        } else {
            format!("{}/%", dir_path.trim_end_matches('/'))
        };

        let mut stmt = conn
            .prepare("SELECT path FROM files WHERE path LIKE ?")
            .map_err(|e| format!("failed to prepare query: {e}"))?;

        let rows = stmt
            .query_map(params![pattern], |row| row.get(0))
            .map_err(|e| format!("failed to list files: {e}"))?;

        let mut files = Vec::new();
        for path_result in rows {
            files.push(path_result.map_err(|e| format!("failed to get path: {e}"))?);
        }
        Ok(files)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn put_and_get_file() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("objects.db");
        let storage = SqliteObjectStorage::open_at(&db_path).unwrap();

        storage
            .put_file("/docs/readme.txt", b"Hello, AgentFS!")
            .unwrap();
        let retrieved = storage.get_file("/docs/readme.txt").unwrap();
        assert_eq!(retrieved, Some(b"Hello, AgentFS!".to_vec()));
    }

    #[test]
    fn get_nonexistent_file() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("objects.db");
        let storage = SqliteObjectStorage::open_at(&db_path).unwrap();

        let result = storage.get_file("/does/not/exist.txt").unwrap();
        assert_eq!(result, None);
    }

    #[test]
    fn delete_file() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("objects.db");
        let storage = SqliteObjectStorage::open_at(&db_path).unwrap();

        storage.put_file("/temp/data.bin", b"temporary").unwrap();
        assert!(storage.get_file("/temp/data.bin").unwrap().is_some());

        let deleted = storage.delete_file("/temp/data.bin").unwrap();
        assert!(deleted);
        assert!(storage.get_file("/temp/data.bin").unwrap().is_none());

        let deleted_again = storage.delete_file("/temp/data.bin").unwrap();
        assert!(!deleted_again);
    }

    #[test]
    fn list_files() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("objects.db");
        let storage = SqliteObjectStorage::open_at(&db_path).unwrap();

        storage.put_file("/notes/day1.md", b"day 1").unwrap();
        storage.put_file("/notes/day2.md", b"day 2").unwrap();
        storage.put_file("/data/config.json", b"{}").unwrap();

        let notes = storage.list_files("/notes").unwrap();
        assert_eq!(notes.len(), 2);
        assert!(notes.contains(&"/notes/day1.md".to_string()));
        assert!(notes.contains(&"/notes/day2.md".to_string()));
    }

    #[test]
    fn overwrite_file() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("objects.db");
        let storage = SqliteObjectStorage::open_at(&db_path).unwrap();

        storage.put_file("/config.yaml", b"version: 1").unwrap();
        storage.put_file("/config.yaml", b"version: 2").unwrap();
        let content = storage.get_file("/config.yaml").unwrap().unwrap();
        assert_eq!(content, b"version: 2");
    }
}
