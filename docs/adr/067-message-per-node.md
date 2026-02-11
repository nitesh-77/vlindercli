# ADR 067: One Message, One Node

## Status

Accepted

**Supersedes:** ADR 061 (the paired invoke/complete model)

## Context

ADR 061 defined a DAG node as one invoke/complete **pair**. The DagCaptureWorker holds invoke payloads in memory, waits for the matching complete, then writes a single node with both `payload_in` and `payload_out`.

This has three problems:

**Stateful worker.** The worker holds a `pending: HashMap<(String, String), PendingInvoke>` in memory. If it crashes between invoke and complete, the invoke is lost. The pairing logic is the only reason the worker has state.

**Coarse granularity.** You only see the result, not the invoke as it happens. An agent that was invoked 30 seconds ago but hasn't completed yet is invisible. The DAG captures completions, not activity.

**Incomplete picture.** The worker explicitly ignores `request`, `response`, and `delegate` messages (`_ => {}` in the match). Three of the five message types are invisible. You see agent boundaries but not what happened inside — no inference requests, no storage calls, no delegations.

### The five message types already exist

ADR 044 defined five typed messages. They already have well-defined structs and an `ObservableMessage` enum that unifies them:

| Message | Direction | What it captures |
|---------|-----------|-----------------|
| `Invoke` | Harness → Runtime | User asked an agent something |
| `Request` | Agent → Service | Agent needs inference, storage, etc. |
| `Response` | Service → Agent | Service replied |
| `Complete` | Runtime → Harness | Agent finished |
| `Delegate` | Agent → Agent | Agent invoking another agent |

Each message has a `payload`, a `submission` (groups all messages for one user request), and enough metadata to know who sent it and who receives it. They are independently meaningful — an invoke without its complete tells you "this agent was invoked and hasn't finished."

## Decision

**Every NATS message is its own DAG node.** No pairing. No buffering. No in-memory state. Message arrives, node is written.

### DagNode

```rust
pub struct DagNode {
    pub hash: String,
    pub parent_hash: String,
    pub message_type: MessageType,
    pub from: String,
    pub to: String,
    pub session_id: String,
    pub submission_id: String,
    pub payload: Vec<u8>,
    pub created_at: String,
}

pub enum MessageType {
    Invoke,
    Request,
    Response,
    Complete,
    Delegate,
}
```

One payload per node, not two. `from` and `to` parsed from the NATS subject. `message_type` parsed from the subject's type segment. Content hash: `sha256(payload || parent_hash || message_type)`.

### Stateless worker

```rust
fn process_message(&mut self, subject: &str, headers: &HashMap<String, String>, payload: &[u8]) {
    // Parse subject → message_type, from, to
    // Get session_id from headers
    // Compute hash, chain to last node in session
    // Write node. Done.
}
```

No `pending` map. No pairing. The only state is `last_node: HashMap<String, String>` for Merkle chaining per session — and even that could be reconstructed from the store on restart.

### What the DAG looks like now

```
$ vlinder timeline route ses-abc123

1. invoke    cli → container.support-agent          [2.1s]
2. request   support-agent → infer.ollama           [0.0s]
3. response  infer.ollama → support-agent           [1.8s]
4. delegate  support-agent → container.log-analyst  [0.0s]
5. invoke    support-agent → container.log-analyst  [0.0s]
6. request   log-analyst → infer.ollama             [0.0s]
7. response  infer.ollama → log-analyst             [1.2s]
8. complete  log-analyst → support-agent            [0.0s]
9. request   support-agent → infer.ollama           [0.0s]
10. response infer.ollama → support-agent           [0.9s]
11. complete support-agent → cli                    [0.0s]
```

That's the full protocol trace. You see when the inference call went out and when it came back. You see the delegation to the sub-agent. You see every hop.

### What changes from ADR 061

| | ADR 061 | ADR 067 |
|---|---|---|
| Node granularity | One per invoke/complete pair | One per message |
| Messages captured | Invoke, Complete only | All five types |
| Worker state | `pending` map (invoke held in memory) | Stateless |
| Crash behavior | Loses buffered invokes | Loses nothing |
| `payload_in` / `payload_out` | Both in one node | Single `payload` per node |
| Fleet internals | Agent boundaries only | Every hop including inference and storage |

### Effect on Route (ADR 063)

Route groups messages into higher-level views. The raw DAG is messages; Route can project them into Stops (group invoke + requests + responses + complete for one agent) or show the full trace. Route is a projection of the DAG, and a richer DAG gives Route more to work with.

### Effect on git projection (ADR 064)

Each message is one git commit. The commit message format becomes:

```
<message_type>: <from> → <to>

<payload summary>

Session: <session_id>
Submission: <submission_id>
Message-Type: <invoke|request|response|complete|delegate>
```

`git log --oneline` reads like a protocol trace.

## Consequences

- The DAG captures the complete protocol — all five message types, not just two
- The worker is stateless — crash-proof by construction
- No pairing logic — simpler code, no in-memory buffering
- Each message is independently meaningful in the DAG
- `DagNode` simplifies: one payload, not two
- Route (ADR 063) can project messages into higher-level views like Stops
- Git projection (ADR 064) produces finer-grained commits — each message is a commit
- The `ObservableMessage` enum from ADR 044 maps directly to `MessageType`
