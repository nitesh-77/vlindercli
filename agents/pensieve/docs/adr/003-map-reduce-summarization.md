# ADR 003: Map-Reduce Summarization

## Status

Accepted

## Context

Articles often exceed LLM context windows. A 35k character article truncated to 4k loses ~90% of content. The summary would only reflect the introduction.

Options considered:
- **A: Truncate** - Simple but loses most content.
- **B: Map-reduce** - Process chunks separately, then synthesize. Full coverage.
- **C: Sliding window** - Sequential processing with overlap. Complex state management.
- **D: Key passage extraction** - Summarize selected passages. Requires good selection heuristic.

## Decision

**Map-reduce (Option B).** Two-phase summarization:

1. **Map:** Chunk text, sample evenly (cap at N chunks for cost control), get 1-2 sentence summary per chunk
2. **Reduce:** Combine chunk summaries, synthesize into coherent final summary

This ensures every part of the article influences the final summary.

## Consequences

- Full article coverage regardless of length
- Cost scales with article length (N+1 inference calls)
- Sampling maintains coverage while bounding cost
- Quality depends on chunk summary quality
- Introduces first LLM dependency (`infer` host function)
