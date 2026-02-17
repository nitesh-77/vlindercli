# ADR 092: Request-Reply on MessageQueue

## Status

Draft

## Context

Three call sites in the codebase implement the same pattern: send a message, spin until the correlated reply arrives.

### Sidecar (ADR 091)

The sidecar's `proxy_request()` sends a `RequestMessage` through the queue and blocks until the `ResponseMessage` comes back. Before the refactor, this was a hand-rolled poll loop with `send_request` + `receive_response` + sleep + poll counter + diagnostic logging. After adding `MessageQueue::request()`, it collapsed to one call.

### QueueBridge (ADR 076)

`send_service_request()` is the same loop — 78 lines of send, poll, sleep, log. It also has `wait()`, which blocks on `receive_complete_on_subject()` for delegation results. Same shape, different message type.

### Harness

`invoke()` sends an `InvokeMessage`. Then the *caller* runs a loop calling `tick()` and `poll()` until the `CompleteMessage` arrives. The request-reply is smeared across three methods and the call boundary. The caller interleaves domain logic (did the reply arrive?) with UI logic (update spinner, check ctrl+c).

### The pattern

All three are: send a message, block until the correlated reply arrives. The differences are superficial:

| Call site | Sends | Receives |
|---|---|---|
| Sidecar / QueueBridge service calls | `RequestMessage` | `ResponseMessage` |
| QueueBridge delegation wait | `DelegateMessage` | `CompleteMessage` |
| Harness invoke | `InvokeMessage` | `CompleteMessage` |

The message types differ. The blocking loop is identical.

### NATS has this built in

NATS `request()` sends a message with an auto-generated reply inbox and blocks until one response arrives. The poll loop, sleep interval, and timeout handling are all internal to the client. The responder sees a `reply` field on the incoming message and publishes to it. No special server-side support needed.

## Decision

Add `request()` as a default method on `MessageQueue`.

```rust
fn request(&self, msg: RequestMessage) -> Result<ResponseMessage, QueueError> {
    self.send_request(msg.clone())?;
    loop {
        match self.receive_response(&msg) {
            Ok((response, ack)) => { let _ = ack(); return Ok(response); }
            Err(QueueError::Timeout) => { sleep(1ms); }
            Err(e) => return Err(e),
        }
    }
}
```

The default implementation polls. NATS overrides with native request-reply. InMemory uses the default.

### Consequences for each call site

**Sidecar** — already migrated. `proxy_request()` calls `queue.request()`.

**QueueBridge `send_service_request()`** — the 35-line poll loop collapses to one call. The method keeps its request-building logic but drops the polling, sleep, poll counter, and stale-poll warnings.

**QueueBridge `wait()`** — same pattern, but for `CompleteMessage` on a delegation reply subject. Needs its own method or a generalized version. Not in this increment.

**Harness** — `invoke()` sends, `tick()` + `poll()` receive, the caller owns the loop. With `request()` semantics, the harness gains `invoke_and_wait()`: send invoke, block until complete. The caller spawns a thread for UI (spinner, ctrl+c) and calls `invoke_and_wait()` on the main thread. Domain and UI concerns separate cleanly.

### What this ADR does not decide

- Worker-side `serve()` pattern (the mirror of request-reply for responders)
- Timeout policy for `request()` — the default loops forever; real timeouts are a separate decision
- Streaming — request-reply is one-shot; streaming needs a different pattern

## Consequences

- Three copies of the same poll loop converge on one method
- NATS can override with native request-reply — zero polling, zero sleep
- Harness callers separate domain (invoke and wait) from UI (spinner, ctrl+c) via threads
- The `MessageQueue` trait gains a higher-level primitive that matches how NATS actually works
- Existing `send_request` / `receive_response` remain for callers that genuinely need two-phase messaging
