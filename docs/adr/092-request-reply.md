# ADR 092: Request-Reply on MessageQueue

## Status

Accepted

## Context

Three call sites in the codebase implement the same pattern: send a message, spin until the correlated reply arrives.

### QueueBridge (ADR 076)

`send_service_request()` is a 35-line poll loop — send, poll, sleep, log.
It also has `wait()`, which blocks on `receive_complete_on_subject()` for
delegation results. Same shape, different message type.

### Harness

`invoke()` sends an `InvokeMessage`. Then the *caller* runs a loop calling
`tick()` and `poll()` until the `CompleteMessage` arrives. The request-reply
is smeared across three methods and the call boundary. The caller interleaves
domain logic (did the reply arrive?) with UI logic (update spinner, check
ctrl+c).

### The pattern

All are: send a message, block until the correlated reply arrives.

| Call site | Sends | Receives |
|---|---|---|
| QueueBridge service calls | `RequestMessage` | `ResponseMessage` |
| Harness invoke | `InvokeMessage` | `CompleteMessage` |

The message types differ. The blocking loop is identical.

### NATS has this built in

NATS `request()` sends a message with an auto-generated reply inbox and
blocks until one response arrives. The poll loop, sleep interval, and
timeout handling are all internal to the client.

## Decision

Add two default methods on `MessageQueue`, backed by a shared
`send_and_wait()` function:

```rust
pub type Acknowledgement = Box<dyn FnOnce() -> Result<(), QueueError> + Send>;

fn call_service(&self, msg: RequestMessage) -> Result<ResponseMessage, QueueError> { ... }
fn run_agent(&self, msg: InvokeMessage) -> Result<CompleteMessage, QueueError> { ... }
```

Both delegate to a private `send_and_wait()` that owns the poll loop:

```rust
fn send_and_wait<T>(
    send: impl FnOnce() -> Result<(), QueueError>,
    receive: impl Fn() -> Result<(T, Acknowledgement), QueueError>,
) -> Result<T, QueueError> {
    send()?;
    loop {
        match receive() {
            Ok((reply, ack)) => { let _ = ack(); return Ok(reply); }
            Err(QueueError::Timeout) => { sleep(1ms); }
            Err(e) => return Err(e),
        }
    }
}
```

Two methods instead of one generic `request()` because the send/receive
pairs differ in type: service calls use `send_request`/`receive_response`,
agent invocations use `send_invoke`/`receive_complete`. Named methods make
the intent clear at each call site.

The default implementation polls. NATS can override with native
request-reply. InMemory uses the default.

### Consequences for each call site

**QueueBridge `send_service_request()`** — the 35-line poll loop collapses
to `queue.call_service(request)`. State cursor update remains inline.

**QueueBridge `wait()`** — same pattern, but for `CompleteMessage` on a
delegation reply subject. Not in this increment.

**Harness** — gains `run_agent()` which builds the invoke, calls
`queue.run_agent()`, and returns the result. `build_invoke()` is extracted
from `invoke()` so both the blocking `run_agent()` and non-blocking
`invoke()` share construction logic. Command callers (agent, fleet,
timeline) collapse from invoke/tick/poll loops to a single
`harness.run_agent()` call.

### What this ADR does not decide

- Worker-side `serve()` pattern (the mirror of request-reply for responders)
- Timeout policy — the default loops forever; real timeouts are a separate decision
- Streaming — request-reply is one-shot; streaming needs a different pattern

## Consequences

- Poll loops in QueueBridge and command callers converge on two methods
- NATS can override with native request-reply — zero polling, zero sleep
- Harness callers separate domain (invoke and wait) from UI (spinner, ctrl+c)
- The `MessageQueue` trait gains higher-level primitives that match how NATS works
- Existing `send_request` / `receive_response` remain for callers that need two-phase messaging
