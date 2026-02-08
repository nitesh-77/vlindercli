# ADR 056: Fleet Execution and Agent-to-Agent Invocation

**Status:** Proposed

## Context

Vlinder has agents that work in isolation: one agent, one REPL, one conversation. The fleet domain model exists (ADR 022) but is inert — it parses `fleet.toml` and validates paths, but nothing runs.

The goal: multi-agent systems where a fleet of agents collaborates to handle user requests. An entry agent receives user messages, delegates to specialists, and synthesizes responses.

The platform is plumbing. Agents are code. The developer writes agents that are designed to work together. The platform deploys them, routes messages, and provides storage and inference. It does not promise magic that the developer hasn't built.

### Why async-first

ADR 005/006 established a coroutine model: agents yield operations and resume with results. The WASM-specific implementation was superseded (ADR 046), but the conceptual model — yield, wait, resume — is exactly right for agent-to-agent invocation.

An orchestrator that can only delegate synchronously (one agent at a time) is artificially limited. The natural primitive is async delegation: issue multiple invocations, let the platform run them in parallel, collect results when ready. Sync delegation is sugar over async.

Parallelism is constrained by hardware, not by the platform. If the machine can run four agents concurrently, four agents run concurrently.

## Decision

### Fleet as the unit of deployment

`vlinder fleet run -p path/to/fleet` deploys all agents declared in `fleet.toml` and starts a REPL connected to the entry agent. The user talks to the entry agent. The entry agent talks to specialists. The user never addresses specialists directly.

```toml
# fleet.toml
name = "research-team"
entry = "orchestrator"

[agents]
orchestrator = { path = "agents/orchestrator" }
summarizer = { path = "agents/summarizer" }
fact-checker = { path = "agents/fact-checker" }
```

### Async delegation is the primitive

Any deployed agent can invoke any other deployed agent. Delegation is async-first: the caller issues invocations that return immediately with a handle, then waits for results when needed.

```
# Issue invocations (non-blocking)
POST /delegate
{ "agent": "summarizer", "input": "Summarize this article: ..." }
→ { "handle": "d1" }

POST /delegate
{ "agent": "fact-checker", "input": "Verify these claims: ..." }
→ { "handle": "d2" }

# Collect results (blocks until ready)
POST /wait
{ "handle": "d1" }
→ { "output": "The article discusses..." }

POST /wait
{ "handle": "d2" }
→ { "output": "Claim A: verified. Claim B: unverified." }
```

The handle is a reply queue subject. `delegate` creates a DelegateMessage on the target agent's queue and returns the reply subject. The target agent runs, completes, and its response lands on that reply queue. `wait` polls the reply queue — same mechanism as `infer` or `kv_get`. No special storage, no handle registry. Just messages on queues.

Multiple delegations run concurrently — the platform schedules them against available resources.

Synchronous delegation (call one agent, block until done) is the degenerate case: `delegate` + immediate `wait`.

Recursive delegation is allowed. Agent A can delegate to agent B, which delegates back to agent A. The platform does not detect or prevent cycles. If the developer builds an infinite loop, the platform runs an infinite loop. The developer controls the termination condition, not the platform.

### The orchestrator is just an agent

The entry agent is not a special type. It is an agent that happens to receive fleet context and has delegation available. What it does with those capabilities is entirely up to the developer.

The orchestrator could be:

- **Hard-coded logic** — static routing in Rust/Python based on input patterns
- **Template-driven** — prompts with variables, deterministic flow
- **Fully autonomous** — an LLM agent that uses inference to reason about which agents to invoke, how to assess responses, and when to iterate
- **Hybrid** — LLM decides what to do, code enforces guardrails and resource limits

The platform doesn't know which approach the developer chose. The same `delegate`, `wait`, `infer`, and `kv` primitives serve all of them.

An autonomous orchestrator's loop:

```
loop {
    // Use inference to assess state and decide next steps
    plan = infer("Given these results: {results}, which agents should I invoke next?")

    // Delegate to specialists (parallel)
    handles = [delegate(agent, input) for each agent in plan]

    // Collect results
    results = [wait(h) for h in handles]

    // Use inference to evaluate quality
    assessment = infer("Are these results sufficient? {results}")

    if assessment.done { return synthesis(results) }
    // else: loop with new information
}
```

The platform is happy to run this loop indefinitely. It does not impose iteration limits, DAG structures, or pipeline shapes. The developer (or the developer's LLM) controls the flow.

### Fleet context is injected, not discovered

When the fleet deploys, the platform assembles a fleet context document from the agents' manifests: names, descriptions, prompts, model capabilities. This context is provided to the entry agent so it can reason about routing.

The mechanism: the fleet context is prepended to the entry agent's payload on every invocation. The entry agent sees it as part of its input, alongside the user's message and conversation history.

```
Fleet: research-team
Available agents:
- summarizer: Summarizes long articles into key points. Models: phi3.
- fact-checker: Verifies claims against source material. Models: phi3, nomic-embed.

User: Is this article accurate? [article text]
```

The entry agent's own prompts and logic determine how to use this context. The platform just provides it. An autonomous orchestrator uses this context to reason about which agents can help; a hard-coded orchestrator may ignore it entirely.

### Specialists don't know about the fleet

A specialist agent receives an invocation exactly like it would from the CLI. It doesn't know whether the caller is a user or another agent. It processes the input, uses its storage and inference, and returns a response.

This means any agent can work both standalone (`vlinder agent run`) and as part of a fleet. No fleet-specific code in agents. The fleet is a deployment concern, not a coding concern.

### State is per-agent, timeline is system-wide

Each agent in the fleet has its own state (KV, vectors) tracked independently. The conversation store records all invocations — user-to-entry and entry-to-specialist — on the same timeline. `vlinder timeline log` shows the full system activity. `vlinder timeline fork` forks the entire system.

### From prototype to production

The platform provides three capabilities that compound across the development lifecycle:

1. **Observability (ADR 044)** — every delegation, every inference call, every KV operation is a typed message with eight dimensions. `nats sub vlinder.sub-abc123.>` shows the full multi-agent interaction in real time. A developer can see exactly which agents were invoked, what they returned, and how the orchestrator used the results.

2. **Timeline (ADR 055)** — the git-backed conversation store records every interaction. `timeline fork` lets a developer replay the same scenario with a different model, different prompts, or different routing logic. A/B testing is a fork operation.

3. **Fleet (this ADR)** — async delegation and fleet context give agents the primitives to collaborate. The orchestrator can be as autonomous or as deterministic as the developer chooses.

The expected workflow: start with an LLM-driven orchestrator during development. Use observability to understand which delegation patterns work. Use timeline forking to compare strategies. Gradually replace LLM decisions with deterministic logic as the patterns become clear. By the time the system reaches production, it is battle-tested, deterministic, and fully observable — the LLM may no longer be in the orchestration loop at all.

The platform doesn't promise a production-ready system. It provides the instruments for a developer to build one.

## Scope

### Message protocol

Delegation adds one new message type to the typed message protocol (ADR 044):

- **DelegateMessage** — sent by the calling agent to the target agent's queue. Contains the input payload and a reply subject. The target agent receives this as a standard invocation; it does not know it came from another agent rather than the CLI.

The target's response is a CompleteMessage on the reply subject — the same completion path used for CLI-initiated invocations.

### Day One

- `vlinder fleet run -p <path>` — deploy all agents, REPL on entry agent
- `delegate` + `wait` SDK operations — async-first agent-to-agent invocation
- DelegateMessage in the typed message protocol
- Fleet context injection — manifest-derived context prepended to entry agent payload
- Queue routing for agent-to-agent messages

### Deferred

- `vlinder fleet list` — show deployed fleets
- `vlinder fleet get` — show fleet details and agent status
- Agent-to-agent streaming — today's delegate is request/response; streaming can come later
- Fleet-scoped state — shared state across agents in a fleet (today each agent has its own)

## Consequences

- The fleet is a deployment unit: `vlinder fleet run` replaces `vlinder agent run` for multi-agent scenarios
- Async delegation is the core primitive — parallel invocation is day one, not deferred
- The orchestrator is just an agent — it can be code, an LLM, or both
- Specialists are regular agents with no fleet awareness — they work standalone or in a fleet
- The platform imposes no orchestration structure — no DAGs, no pipelines, no iteration limits
- The developer (or the developer's LLM) is responsible for coherence
- The conversation timeline captures the full multi-agent interaction, enabling system-wide fork and time-travel
