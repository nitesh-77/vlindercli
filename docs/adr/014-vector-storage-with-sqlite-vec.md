# ADR 014: Vector Storage with sqlite-vec

## Status

Accepted (interface updated by ADR 018)

The sqlite-vec storage decision and EmbeddingEngine trait remain valid. The host function interface (`embed`, `store_embedding`, `search_by_vector`) was replaced by queue-based services (`embed`, `vector-put`, `vector-search` queues).

## Context

Agents need vector storage for semantic search over embeddings. This enables:
- Finding similar documents/content
- RAG (retrieval-augmented generation) patterns
- Memory/knowledge base for agents

We needed a solution that:
- Maintains MIT license compliance
- Integrates with our existing SQLite storage
- Provides efficient similarity search

## Decision

**Use sqlite-vec extension with rusqlite for vector storage.**

This required switching from libsql to rusqlite:
- libsql doesn't easily support loading sqlite-vec extension
- rusqlite with `bundled` feature + `sqlite3_auto_extension` loads sqlite-vec statically

**Storage schema (vec0 virtual table):**

```sql
CREATE VIRTUAL TABLE IF NOT EXISTS vec_items USING vec0(
    key TEXT PRIMARY KEY,
    embedding float[768],
    metadata TEXT
);
```

**Host functions exposed to WASM agents:**

```rust
fn embed(model: String, text: String) -> String;           // JSON array of floats
fn store_embedding(key: String, vector: String, meta: String) -> String;
fn search_by_vector(query: String, limit: String) -> String;  // JSON results
```

**EmbeddingEngine trait for llama.cpp:**

```rust
pub trait EmbeddingEngine: Send + Sync {
    fn embed(&self, text: &str) -> Result<Vec<f32>, String>;
}
```

Uses `LlamaContextParams::with_embeddings(true)` to get embeddings from any compatible model.

## Consequences

- 100% MIT license compliance maintained
- Native SQLite vector search via sqlite-vec MATCH operator
- Fixed 768-dimension vectors (matches nomic-embed and similar models)
- Agents can build semantic search and RAG patterns
- Each agent's embeddings are isolated in their own .db file
- Switched from async libsql to sync rusqlite (simpler, since we were using block_on anyway)
