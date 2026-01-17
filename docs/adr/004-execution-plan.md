# ADR 004: ExecutionPlan is a DAG

## Status

Accepted

## Context

Agents may need to specify what happens next - call another agent, chain operations, or run things in parallel.

## Domain Decision

An ExecutionPlan is a DAG (directed acyclic graph) of operations.

- Single operation: trivial DAG with one node
- Sequence: linear DAG (A → B → C)
- Complex workflow: DAG with branches and dependencies

AgentOutput can include an ExecutionPlan:

```rust
struct AgentOutput {
    response: Option<String>,
    next: Option<ExecutionPlan>,
}
```

The specific data structure is an implementation detail. The domain concept is: it's a DAG.
