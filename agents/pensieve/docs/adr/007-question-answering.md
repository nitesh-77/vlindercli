# ADR 007: Question Answering with RAG

## Status

Accepted

## Context

The agent can search for relevant passages and display them to the user (SEARCH intent). But users often want a direct answer synthesized from their stored knowledge, not just raw passages to read through.

Two distinct user needs:
- "Show me what I have about productivity" → wants to browse source material
- "How can I be more productive?" → wants a synthesized answer

## Decision

**Add a QUESTION intent that implements Retrieval-Augmented Generation (RAG).**

### Intent Distinction

| Intent | User Goal | Output |
|--------|-----------|--------|
| SEARCH | Browse source material | Raw passages with sources |
| QUESTION | Get a direct answer | Synthesized response + sources |

### Classification Heuristics

SEARCH signals:
- "search for X", "find passages about X"
- "what do I have on X", "what do I know about X"

QUESTION signals:
- Questions ending with "?"
- "what is X", "how does X work", "why"
- "explain X", "tell me about X"

### RAG Pipeline

```
User Question
     ↓
Query Expansion (existing)
     ↓
Embedding + Vector Search (existing, via find_relevant_chunks)
     ↓
Top 5 Chunks as Context
     ↓
LLM Synthesis Prompt
     ↓
Answer + Source Attribution
```

### Reuse via Refactoring

Extracted `find_relevant_chunks(query, limit)` from `handle_search` to share the retrieval logic:
- Query expansion
- Embedding generation
- Vector search
- Result parsing

Both SEARCH and QUESTION use this, differing only in presentation (raw vs synthesized).

### Answer Generation Prompt

The synthesis prompt instructs the LLM to:
- Use ONLY the provided context
- Admit when context is insufficient
- Speak naturally (not "the passages say...")
- Synthesize multiple perspectives coherently

## Consequences

- **Direct answers**: Users can ask questions and get synthesized responses
- **Source attribution**: Answers include source URLs for verification
- **Shared retrieval**: Both intents use identical retrieval logic
- **Two LLM calls**: QUESTION requires retrieval + synthesis (vs SEARCH's retrieval only)
- **Hallucination risk**: Mitigated by grounding prompt in provided context only
