# ADR 002: Rule-Based Text Cleaning

## Status

Accepted

## Context

Readability extraction (ADR 001) removes most HTML noise, but residual artifacts remain: navigation text ("Sign In", "Subscribe"), short link-filled preambles, UI element labels. This noise degrades downstream summarization quality.

Options considered:
- **A: Do nothing** - Accept noise. Simplest but hurts quality.
- **B: Rule-based heuristics** - Fast, free, deterministic.
- **C: LLM post-processing** - Use inference to identify boilerplate. Smart but expensive.
- **D: CSS pre-processing** - Remove elements before readability. Adds complexity.

## Decision

**Rule-based heuristics (Option B).** Three sequential cleaning steps:

1. **Boilerplate line removal** - Filter lines matching known patterns (case-insensitive)
2. **Short leading line pruning** - Skip lines with fewer than N words from the start
3. **First paragraph detection** - Find first line exceeding M characters ending with sentence punctuation

Order matters: remove boilerplate first (catches mid-article noise), then prune short lines (removes link lists), then find real content start.

## Consequences

- Zero runtime cost (no inference)
- Configurable thresholds
- Pattern list is extensible
- Graceful degradation - returns text as-is if no "real" paragraph found
- Won't catch site-specific boilerplate not in pattern list
