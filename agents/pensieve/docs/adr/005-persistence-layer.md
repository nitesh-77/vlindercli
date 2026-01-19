# ADR 005: Persistence Layer for Stateful Memory

## Status

Accepted

## Context

Pensieve currently processes URLs statelessly - each run fetches, processes, and summarizes from scratch. This wastes resources (re-downloading, re-processing) and loses valuable extracted knowledge when the agent finishes.

The host environment provides persistence capabilities:
- `get_file` / `put_file` - file-based storage
- `embed` / `store_embedding` - vector storage for semantic search

## Decision

**Transform pensieve into a stateful agent with durable memory.**

### 1. Multi-Layer Caching

Cache expensive operations to avoid redundant work:

```
/html/{url_key}.html     - Raw HTML (avoid re-fetching)
/clean/{url_key}.txt     - Cleaned text (avoid re-processing)
```

Workflow: Check cache → use if exists → otherwise compute and store.

### 2. Embedding Storage

After successful extraction, embed and store each chunk:
- Generate vector via `embed` host function
- Store via `store_embedding` with metadata (url, chunk index, preview)

This enables future semantic search across all indexed content.

## Consequences

- Efficiency: No redundant network/processing on repeat URLs
- Durability: Extracted knowledge persists across runs
- Searchability: Embedded chunks enable semantic recall
- Storage growth: Each URL adds cached files + embeddings
- Cache invalidation: Not addressed (future consideration)

## File Path Structure

```
/html/{url_key}.html    - Raw fetched HTML
/clean/{url_key}.txt    - Post-cleaning article text
```

URL key: alphanumeric transformation of URL (e.g., `https://example.com/foo` → `https___example_com_foo`)
