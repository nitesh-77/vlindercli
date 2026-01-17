# ADR 004: ExecutionPlan Yields Operations

## Status

Superseded by ADR 005

## Context

Agents may produce work for the runtime to execute - calling other agents, chaining operations, running things in parallel.

## Domain Decision

An ExecutionPlan is something that yields operations.

Given previous results, it decides what to run next. It controls the iteration. The runtime just executes what it's given.

- Yields nothing: done
- Yields one operation: run it
- Yields many operations: run in parallel

This separates concerns:
- **Plan**: decides WHAT to run next
- **Runtime**: decides HOW to execute

The plan is an iterator. The runtime consumes it.

AgentOutput can include an ExecutionPlan. This is how agents produce work for the runtime.

## Why Superseded

This decision assumed ExecutionPlan could be a closure at runtime level. But:

1. **Agents are wasm.** Closures can't cross the wasm boundary.
2. **Memory efficiency.** A closure requires the agent to stay loaded while operations execute. Agents should be unloadable.
3. **Orchestrator is just an agent.** The orchestrator is wasm too - same constraints apply.

The "iterator that yields operations" concept is correct. But the implementation must be a suspended wasm coroutine, not a runtime closure.

See ADR 005.
