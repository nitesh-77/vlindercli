# ADR 002: Agent has Behavior

## Status

Accepted

## Context

An agent is not just a model reference. Two agents can use the same model but behave completely differently.

## Domain Decision

An agent has behavior. At minimum, this is a system prompt that shapes how the agent uses its model.

- **Model**: What to run
- **Behavior**: How to behave

The same model with different behavior produces different agents. A note-taker and a podcast assistant might use the same LLM - the behavior is what differentiates them.

This is why agentic systems are more than models. The behavior is the value-add.
