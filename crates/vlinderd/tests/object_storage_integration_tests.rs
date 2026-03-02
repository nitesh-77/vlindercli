//! Integration tests for SqliteObjectStorage.
//! Tests SQLite-specific behavior: persistence, isolation, security.

use tempfile::TempDir;
use vlinder_sqlite_kv::SqliteObjectStorage;

#[test]
fn sqlite_open_creates_db() {
    let dir = TempDir::new().unwrap();
    let db_path = dir.path().join("agent.db");
    let _storage = SqliteObjectStorage::open_at(&db_path).unwrap();
    assert!(db_path.exists());
}

/// Verify data persists across close/reopen
#[test]
fn sqlite_persistence() {
    let dir = TempDir::new().unwrap();
    let db_path = dir.path().join("agent.db");

    // Write data
    {
        let storage = SqliteObjectStorage::open_at(&db_path).unwrap();
        storage
            .put_file("/persistent.txt", b"survives restart")
            .unwrap();
    } // storage dropped, connection closed

    // Reopen and verify data persists
    {
        let storage = SqliteObjectStorage::open_at(&db_path).unwrap();
        let content = storage.get_file("/persistent.txt").unwrap();
        assert_eq!(content, Some(b"survives restart".to_vec()));
    }
}

/// Security test: Agents cannot access host filesystem via path traversal.
/// The storage is a virtual filesystem - paths are just keys in SQLite,
/// not real filesystem paths.
#[test]
fn sqlite_cannot_read_host_filesystem() {
    let dir = TempDir::new().unwrap();
    let db_path = dir.path().join("agent.db");
    let storage = SqliteObjectStorage::open_at(&db_path).unwrap();

    // Attempt to read host files - should return None
    // because these are just database keys, not real paths
    assert_eq!(storage.get_file("/etc/hosts").unwrap(), None);
    assert_eq!(
        storage
            .get_file("/System/Library/CoreServices/SystemVersion.plist")
            .unwrap(),
        None
    );
    assert_eq!(storage.get_file("../../../etc/hosts").unwrap(), None);
    assert_eq!(storage.get_file(env!("CARGO_MANIFEST_DIR")).unwrap(), None);

    // Verify list_files doesn't expose host directories
    let system_files = storage.list_files("/System").unwrap();
    assert!(system_files.is_empty());
}

/// Security test: Each agent's storage is isolated
#[test]
fn sqlite_agent_isolation() {
    let dir_a = TempDir::new().unwrap();
    let dir_b = TempDir::new().unwrap();
    let storage_a = SqliteObjectStorage::open_at(&dir_a.path().join("agent.db")).unwrap();
    let storage_b = SqliteObjectStorage::open_at(&dir_b.path().join("agent.db")).unwrap();

    // Agent A writes a secret
    storage_a
        .put_file("/secret.txt", b"agent-a-secret")
        .unwrap();

    // Agent B cannot read Agent A's files
    assert_eq!(storage_b.get_file("/secret.txt").unwrap(), None);

    // Agent B's own file is separate
    storage_b
        .put_file("/secret.txt", b"agent-b-secret")
        .unwrap();

    // Each agent sees only their own data
    assert_eq!(
        storage_a.get_file("/secret.txt").unwrap(),
        Some(b"agent-a-secret".to_vec())
    );
    assert_eq!(
        storage_b.get_file("/secret.txt").unwrap(),
        Some(b"agent-b-secret".to_vec())
    );
}
