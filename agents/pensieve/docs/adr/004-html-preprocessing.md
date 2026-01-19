# ADR 004: HTML Pre-processing Before Readability

## Status

Accepted

## Context

Readability (ADR 001) extracts article content but sometimes includes navigation, sidebars, and footer text. Post-processing heuristics (ADR 002) catch some of this, but work on text where structure is already lost.

Better approach: remove boilerplate HTML elements *before* readability sees them.

## Decision

**Add CSS selector-based pre-processing.** Before passing HTML to readability:

1. Parse HTML into a DOM tree
2. Remove elements matching known boilerplate selectors (`nav`, `footer`, `[role="navigation"]`, `.sidebar`, etc.)
3. Serialize cleaned DOM back to HTML
4. Pass cleaned HTML to readability

This gives readability a cleaner input, improving extraction accuracy.

## Consequences

- Cleaner readability input = better extraction
- Adds `scraper` crate (~adds to wasm size)
- Selector list is extensible
- May accidentally remove valid content if selectors are too aggressive
- Works at structure level (complements text-level ADR 002)

## Pipeline

```
Fetch HTML → Pre-process HTML (ADR 004) → Readability (ADR 001) → Clean Text (ADR 002) → Summarize (ADR 003)
```
