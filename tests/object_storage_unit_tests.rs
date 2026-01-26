//! Unit tests for ObjectStorage trait behavior.
//! Uses in-memory implementation for speed.

use vlindercli::storage::{ObjectStorage, InMemoryObjectStorage};

#[test]
fn put_and_get_file() {
    let storage = InMemoryObjectStorage::new();

    let content = b"Hello, AgentFS!";
    storage.put_file("/docs/readme.txt", content).unwrap();

    let retrieved = storage.get_file("/docs/readme.txt").unwrap();
    assert_eq!(retrieved, Some(content.to_vec()));
}

#[test]
fn get_nonexistent_file() {
    let storage = InMemoryObjectStorage::new();

    let result = storage.get_file("/does/not/exist.txt").unwrap();
    assert_eq!(result, None);
}

#[test]
fn delete_file() {
    let storage = InMemoryObjectStorage::new();

    storage.put_file("/temp/data.bin", b"temporary").unwrap();
    assert!(storage.get_file("/temp/data.bin").unwrap().is_some());

    let deleted = storage.delete_file("/temp/data.bin").unwrap();
    assert!(deleted);
    assert!(storage.get_file("/temp/data.bin").unwrap().is_none());

    // Delete non-existent returns false
    let deleted_again = storage.delete_file("/temp/data.bin").unwrap();
    assert!(!deleted_again);
}

#[test]
fn list_files() {
    let storage = InMemoryObjectStorage::new();

    storage.put_file("/notes/day1.md", b"day 1").unwrap();
    storage.put_file("/notes/day2.md", b"day 2").unwrap();
    storage.put_file("/data/config.json", b"{}").unwrap();

    let notes = storage.list_files("/notes").unwrap();
    assert_eq!(notes.len(), 2);
    assert!(notes.contains(&"/notes/day1.md".to_string()));
    assert!(notes.contains(&"/notes/day2.md".to_string()));

    let data = storage.list_files("/data").unwrap();
    assert_eq!(data.len(), 1);
    assert!(data.contains(&"/data/config.json".to_string()));
}

#[test]
fn overwrite_file() {
    let storage = InMemoryObjectStorage::new();

    storage.put_file("/config.yaml", b"version: 1").unwrap();
    storage.put_file("/config.yaml", b"version: 2").unwrap();

    let content = storage.get_file("/config.yaml").unwrap().unwrap();
    assert_eq!(content, b"version: 2");
}

#[test]
fn binary_content() {
    let storage = InMemoryObjectStorage::new();

    // Store binary data with null bytes
    let binary: Vec<u8> = (0..=255).collect();
    storage.put_file("/binary.dat", &binary).unwrap();

    let retrieved = storage.get_file("/binary.dat").unwrap().unwrap();
    assert_eq!(retrieved, binary);
}

#[test]
fn list_files_empty_dir() {
    let storage = InMemoryObjectStorage::new();

    storage.put_file("/data/file.txt", b"content").unwrap();

    // Directory with no files returns empty
    let empty = storage.list_files("/nonexistent").unwrap();
    assert!(empty.is_empty());
}

#[test]
fn list_files_root() {
    let storage = InMemoryObjectStorage::new();

    storage.put_file("/a.txt", b"a").unwrap();
    storage.put_file("/dir/b.txt", b"b").unwrap();
    storage.put_file("/dir/sub/c.txt", b"c").unwrap();

    let all = storage.list_files("/").unwrap();
    assert_eq!(all.len(), 3);
    assert!(all.contains(&"/a.txt".to_string()));
    assert!(all.contains(&"/dir/b.txt".to_string()));
    assert!(all.contains(&"/dir/sub/c.txt".to_string()));
}

#[test]
fn unicode_paths_and_content() {
    let storage = InMemoryObjectStorage::new();

    // Unicode filename
    storage.put_file("/文档/日记.txt", "今日の天気は晴れです".as_bytes()).unwrap();

    let content = storage.get_file("/文档/日记.txt").unwrap().unwrap();
    assert_eq!(String::from_utf8(content).unwrap(), "今日の天気は晴れです");

    // Emoji in path
    storage.put_file("/🎉/party.txt", "🎊".as_bytes()).unwrap();
    let emoji_content = storage.get_file("/🎉/party.txt").unwrap().unwrap();
    assert_eq!(String::from_utf8(emoji_content).unwrap(), "🎊");

    // List unicode directory
    let files = storage.list_files("/文档").unwrap();
    assert_eq!(files.len(), 1);
    assert!(files.contains(&"/文档/日记.txt".to_string()));
}
