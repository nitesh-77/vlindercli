# ADR 001: Agent is not Model

## Status

Accepted

## Context

The current AI landscape conflates models with applications. People say "I'll use GPT-4" when they mean "I'll build something that uses GPT-4."

## Domain Decision

An agent is not a model. A model is a dependency of an agent.

- **Model**: A trained inference engine (LLM, vision, speech)
- **Agent**: A packaged capability - behavior, prompts, tools, code - that uses a model

Agentic systems are far more than models. The model is just one component. The agent is what creates real value.
