# ADR 003: Agents Produce AgentOutput

## Status

Superceded by ADR 018 (Agent outputs are queue messages)

## Context

Agents need to return results. The simplest approach is returning a string. But this limits future extensibility.

## Domain Decision

Agents produce AgentOutput, not raw strings.

```rust
struct AgentOutput {
    response: String,
}
```

This establishes a uniform contract. The structure can grow to support:
- Optional responses
- Next operations (enabling DAGs, chaining)
- Self-refinement loops
- Progressive output

The runtime handles AgentOutput uniformly. Orchestrators and regular agents are the same from runtime's perspective.
