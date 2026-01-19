# ADR 001: Use Readability Algorithm for Content Extraction

## Status

Accepted

## Context

Pensieve fetches web pages and needs to extract article content from HTML. The HTML contains navigation, ads, scripts, and other noise that must be removed.

Options considered:
- **A: LLM extraction** - Prompt an LLM to extract content. Flexible but expensive.
- **B: Readability algorithm** - Mozilla's algorithm used by Firefox Reader View. Fast, free, proven.
- **C: CSS selectors** - Target specific elements. Requires per-site rules.
- **D: Simple tag stripping** - Remove all tags. Loses structure, keeps noise.

## Decision

**Use the Readability algorithm (Option B).**

Extraction is preprocessing, not core value. Save inference budget for reasoning tasks (summarization, analysis). Readability is battle-tested across millions of sites and costs nothing at runtime.

## Consequences

- Zero inference cost for extraction
- Adds `readability` and `url` crates (~50KB to wasm)
- Works well for article-shaped content
- May struggle with non-standard layouts (SPAs, paywalls)
- Can add post-processing heuristics if needed
