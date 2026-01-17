# ADR 004: ExecutionPlan Yields Operations

## Status

Accepted

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
