# ADR 008: Two-Tier Embeddings for Search vs RAG

## Status

Accepted

## Context

Pensieve stores article content as chunked embeddings for RAG-style question answering. When users search, they were seeing raw chunk fragments - disjointed sentences without context.

The problem: **search and question-answering have different retrieval granularities.**

- QUESTION needs precise chunk retrieval to synthesize accurate answers
- SEARCH needs article-level results with meaningful summaries for scanning

## Decision

**Store two tiers of embeddings per article:**

1. **Chunk embeddings** (existing) - key: `{url_key}:chunk:{n}`
   - Fine-grained passages (~1000 chars)
   - Used for QUESTION intent (RAG)
   - Metadata: `{url_key, chunk_index, preview}`

2. **Article embeddings** (new) - key: `article:{url_key}`
   - High-level document summary (Core Argument from briefing)
   - Used for SEARCH intent
   - Metadata: `{url, summary}`

**Search flow:**
1. Embed query
2. Search vector store, filter to `article:` prefix keys
3. Display article URL + summary from metadata

**Question flow (unchanged):**
1. Embed query
2. Search chunk embeddings
3. Build context from relevant chunks
4. Generate synthesized answer

## Consequences

- SEARCH returns meaningful article summaries instead of fragments
- QUESTION still uses chunk-level precision for accurate synthesis
- Storage overhead: one additional embedding per article (~3KB)
- Existing articles need re-commit to get article embeddings
- `extract_core_argument()` parses briefing to get concise summary
