use vlindercli::storage::Storage;

#[test]
fn storage_open_creates_db() {
    let storage = Storage::open("test-agent-open").unwrap();
    // If we got here, the DB was created successfully
    drop(storage);

    // Cleanup
    let _ = std::fs::remove_dir_all(".vlinder/agents/test-agent-open");
}

#[test]
fn storage_put_and_get_file() {
    let storage = Storage::open("test-agent-files").unwrap();

    // Write a file
    let content = b"Hello, AgentFS!";
    storage.put_file("/docs/readme.txt", content).unwrap();

    // Read it back
    let retrieved = storage.get_file("/docs/readme.txt").unwrap();
    assert_eq!(retrieved, Some(content.to_vec()));

    // Cleanup
    drop(storage);
    let _ = std::fs::remove_dir_all(".vlinder/agents/test-agent-files");
}

#[test]
fn storage_get_nonexistent_file() {
    let storage = Storage::open("test-agent-nofile").unwrap();

    let result = storage.get_file("/does/not/exist.txt").unwrap();
    assert_eq!(result, None);

    // Cleanup
    drop(storage);
    let _ = std::fs::remove_dir_all(".vlinder/agents/test-agent-nofile");
}

#[test]
fn storage_delete_file() {
    let storage = Storage::open("test-agent-delete").unwrap();

    // Write and verify
    storage.put_file("/temp/data.bin", b"temporary").unwrap();
    assert!(storage.get_file("/temp/data.bin").unwrap().is_some());

    // Delete and verify
    let deleted = storage.delete_file("/temp/data.bin").unwrap();
    assert!(deleted);
    assert!(storage.get_file("/temp/data.bin").unwrap().is_none());

    // Delete non-existent returns false
    let deleted_again = storage.delete_file("/temp/data.bin").unwrap();
    assert!(!deleted_again);

    // Cleanup
    drop(storage);
    let _ = std::fs::remove_dir_all(".vlinder/agents/test-agent-delete");
}

#[test]
fn storage_list_files() {
    let storage = Storage::open("test-agent-list").unwrap();

    // Create files in different directories
    storage.put_file("/notes/day1.md", b"day 1").unwrap();
    storage.put_file("/notes/day2.md", b"day 2").unwrap();
    storage.put_file("/data/config.json", b"{}").unwrap();

    // List files in /notes
    let notes = storage.list_files("/notes").unwrap();
    assert_eq!(notes.len(), 2);
    assert!(notes.contains(&"/notes/day1.md".to_string()));
    assert!(notes.contains(&"/notes/day2.md".to_string()));

    // List files in /data
    let data = storage.list_files("/data").unwrap();
    assert_eq!(data.len(), 1);
    assert!(data.contains(&"/data/config.json".to_string()));

    // Cleanup
    drop(storage);
    let _ = std::fs::remove_dir_all(".vlinder/agents/test-agent-list");
}

#[test]
fn storage_overwrite_file() {
    let storage = Storage::open("test-agent-overwrite").unwrap();

    // Write initial content
    storage.put_file("/config.yaml", b"version: 1").unwrap();

    // Overwrite with new content
    storage.put_file("/config.yaml", b"version: 2").unwrap();

    // Verify new content
    let content = storage.get_file("/config.yaml").unwrap().unwrap();
    assert_eq!(content, b"version: 2");

    // Cleanup
    drop(storage);
    let _ = std::fs::remove_dir_all(".vlinder/agents/test-agent-overwrite");
}

#[test]
fn storage_binary_content() {
    let storage = Storage::open("test-agent-binary").unwrap();

    // Store binary data with null bytes
    let binary: Vec<u8> = (0..=255).collect();
    storage.put_file("/binary.dat", &binary).unwrap();

    // Verify round-trip
    let retrieved = storage.get_file("/binary.dat").unwrap().unwrap();
    assert_eq!(retrieved, binary);

    // Cleanup
    drop(storage);
    let _ = std::fs::remove_dir_all(".vlinder/agents/test-agent-binary");
}

/// Security test: Agents cannot access host filesystem via path traversal.
/// The storage is a virtual filesystem - paths are just keys in SQLite,
/// not real filesystem paths.
#[test]
fn storage_cannot_read_host_filesystem() {
    let storage = Storage::open("test-agent-security").unwrap();

    // Attempt to read host files - should return None
    // because these are just database keys, not real paths
    assert_eq!(storage.get_file("/etc/hosts").unwrap(), None);
    assert_eq!(storage.get_file("/System/Library/CoreServices/SystemVersion.plist").unwrap(), None);
    assert_eq!(storage.get_file("../../../etc/hosts").unwrap(), None);
    assert_eq!(storage.get_file(env!("CARGO_MANIFEST_DIR")).unwrap(), None);

    // Verify list_files doesn't expose host directories
    let system_files = storage.list_files("/System").unwrap();
    assert!(system_files.is_empty());

    // Cleanup
    drop(storage);
    let _ = std::fs::remove_dir_all(".vlinder/agents/test-agent-security");
}

/// Security test: Each agent's storage is isolated
#[test]
fn storage_agent_isolation() {
    let storage_a = Storage::open("test-agent-isolated-a").unwrap();
    let storage_b = Storage::open("test-agent-isolated-b").unwrap();

    // Agent A writes a secret
    storage_a.put_file("/secret.txt", b"agent-a-secret").unwrap();

    // Agent B cannot read Agent A's files
    assert_eq!(storage_b.get_file("/secret.txt").unwrap(), None);

    // Agent B's own file is separate
    storage_b.put_file("/secret.txt", b"agent-b-secret").unwrap();

    // Each agent sees only their own data
    assert_eq!(
        storage_a.get_file("/secret.txt").unwrap(),
        Some(b"agent-a-secret".to_vec())
    );
    assert_eq!(
        storage_b.get_file("/secret.txt").unwrap(),
        Some(b"agent-b-secret".to_vec())
    );

    // Cleanup
    drop(storage_a);
    drop(storage_b);
    let _ = std::fs::remove_dir_all(".vlinder/agents/test-agent-isolated-a");
    let _ = std::fs::remove_dir_all(".vlinder/agents/test-agent-isolated-b");
}
