# ADR 111: Step Function Execution Mode

**Status:** Draft

## Context

ADR 075 introduced state machine agents where the platform drives the
execution loop. The agent was a pure function: (Event, State) → (Action,
State). Each step was a git commit — enabling mid-execution time travel.

We removed it (ADR 105) because the programming model was hostile. The
`AgentEvent`/`AgentAction` enum protocol required agents to implement a
10-variant dispatch loop. Every new platform service meant new enum
variants in both the sidecar (Rust) and every agent SDK. The bridge
endpoints were a pinhole — restrictive and unfriendly.

We replaced it with typed messages (ADR 105): agents call provider
hostnames directly using each provider's native wire protocol. This made
agent development natural — standard HTTP calls to familiar APIs. But it
moved loop control into the agent, and with it, the intermediate state
that time travel needs.

We now have both pieces:
1. Typed messages with provider-native protocol support (ADR 105)
2. The original insight that platform-driven step functions enable time
   travel (ADR 075)

We need to bring back step function execution using typed messages.

## Decision

Add a step function execution mode that coexists with the current
unmanaged mode. The platform drives the tool-use loop. Each step is a
git commit with full intermediate state.

Design details are deferred to implementation — this ADR captures the
intent and the constraint: step functions must use typed messages
(provider-native wire protocols), not custom enums.

## Consequences

- Time travel becomes possible for step-mode agents
- Unmanaged mode is untouched — existing agents keep working
- Agent authors choose their execution mode
- Implementation must avoid the ADR 075 trap: no custom dispatch enums,
  no two-sided schema coupling
