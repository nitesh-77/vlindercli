# ADR 006: Intent-Driven Dispatcher Architecture

## Status

Accepted

## Context

Pensieve started as a single-purpose agent: give it a URL, get a summary. But a memory system needs more capabilities - listing what's stored, recalling specific memories, searching across content. Users shouldn't need to remember different commands; they should just describe what they want.

The challenge: how do we route natural language input to the right behavior without building a fragile keyword-matching system?

## Decision

**Transform the `process` function into an intent classifier + dispatcher.**

### Intent Classification

Use LLM to classify user input into discrete intents:

```
PROCESS_URL   - "save https://...", bare URLs
LIST_MEMORIES - "what have I saved?", "show my memories"
GET_MEMORY    - "recall the article from example.com"
SEARCH        - "what do I know about X?"
UNKNOWN       - anything that doesn't clearly match
```

### Confidence Threshold

The classifier returns a confidence score (0.0-1.0). If confidence < 0.6, return `UNKNOWN` regardless of the classified intent. This prevents the agent from confidently doing the wrong thing when it's actually guessing.

### Fast Path

Bare URLs (starting with `http://`, `https://`, or `www.`) bypass LLM classification entirely and route directly to `PROCESS_URL`. This is both faster and more reliable for the most common case.

### Fallback Chain

When LLM classification fails (malformed JSON, network error):
1. Try keyword-based detection (list/show → LIST_MEMORIES, search/find → SEARCH)
2. If no keywords match → UNKNOWN

### UNKNOWN Handling

Instead of guessing, `UNKNOWN` returns a helpful message explaining what the agent can do. This is preferable to executing an incorrect action.

## Consequences

- **Multi-capability**: Agent now handles 5 distinct intents
- **Natural language**: Users can phrase requests conversationally
- **Graceful degradation**: Fast path → LLM → keywords → UNKNOWN
- **Explicit uncertainty**: Agent admits when it doesn't understand
- **LLM dependency**: Intent classification requires inference call (except fast path)
- **Latency**: Non-URL inputs have LLM classification overhead

## Module Structure

```
lib.rs        - Dispatcher: determine_intent() → match → handler
intent.rs     - Intent enum, LLM prompt, confidence check, fallback
handlers.rs   - handle_process_url, handle_list_memories, etc.
```
