# ADR 118: Wire-Format Payloads

**Status:** Accepted

## Context

Service call payloads (`RequestMessage.payload` and `ResponseMessage.payload`) are `Vec<u8>` — opaque byte arrays. The platform extracts application-level fields (status code, state hash, diagnostics) and places them on the message struct or NATS headers. The raw HTTP response from the actual service (Ollama, OpenRouter, KV store) is discarded; only the body survives.

This creates several problems:

1. **Information loss.** HTTP headers from service responses (token counts, rate limits, content-type, cache status) are dropped at the worker boundary. The agent never sees them. The DAG doesn't record them.

2. **Redundant protocol fields.** `ResponseMessage.status_code` duplicates what the HTTP response already carries. `ResponseMessage.correlation_id` duplicates what the routing key provides. These fields exist because the payload is opaque — the platform can't read them from the bytes.

3. **Worker ceremony.** Workers must extract the body, discard HTTP metadata, set status codes and state as separate fields, and build diagnostics manually — when the upstream HTTP response already contains everything.

4. **No metering.** Inference calls are expensive real-world interactions. The platform sits between the agent and every service but cannot meter, audit, or bill because the response metadata is thrown away.

5. **Replay fidelity.** Time travel requires reproducing the exact service interaction. With only the body stored, replaying from the DAG cannot reconstruct the full HTTP exchange.

### Observation

The sidecar is the single point of contact between the agent and reality. Every side effect passes through it. It already sees the full HTTP exchange.

Every current service interaction is HTTP: Ollama, OpenRouter, KV (via provider server), vector storage, embedding, Lambda invocations.

## Decision

### 1. Payloads are wire-format captures

`RequestMessage.payload` and `ResponseMessage.payload` carry the serialized HTTP request/response — status line, headers, and body. The payload is self-describing: any HTTP parser can read it without knowing vlinder's internal types.

The implementation uses `http::Response<Vec<u8>>` from the standard `http` crate, serialized via `http-serde-ext`. A `WireResponse` wrapper in `vlinder-core::domain::wire` provides the serde integration. This replaces the current `Vec<u8>` body-only payload.

### 2. Remove redundant message fields

Fields that duplicate information in the wire-format payload are removed from the message structs:

- `ResponseMessage.status_code` — in the HTTP response status line
- `ResponseMessage.correlation_id` — in the routing key (ADR 096)

### 3. Worker crates capture the full HTTP exchange

Worker crates call the upstream HTTP service and capture the complete response — status, headers, and body. The crate serializes it as a `WireResponse` for the message payload and extracts domain-specific metrics (token counts, model info, dimensions) for diagnostics. The crate owns both responsibilities because it has the domain expertise to parse the upstream response format.

The provider server unwraps the `WireResponse` envelope before returning to the agent — agents see a normal HTTP response.

### 4. Metering reads from captured headers

The platform can extract metering data (token counts, model info, rate limits, latency) from the captured HTTP response headers. This enables auditing and billing without modifying workers.

## Consequences

- Service call payloads become self-describing HTTP captures
- `ResponseMessage.status_code` and `correlation_id` to be removed (tracked in WIP branch)
- Worker crates capture full HTTP responses and extract domain-specific metrics
- The DAG captures full HTTP exchanges, enabling exact replay
- Database workers (Dolt/Doltgres) require a separate protocol-aware provider host — see ADR 119
- Metering and auditing become possible by reading captured headers
- Payload size increases by ~200-500 bytes per message (HTTP headers). This is noise relative to actual payload sizes (inference responses are 1-10KB+). Payloads are content-addressed, so identical responses are stored once regardless of how many DAG nodes reference them
- Consumers that need individual fields (e.g., status code) must deserialize the envelope first. This is a single serde call on a flat struct — no HTTP frame parsing. The provider server already deserializes the payload today to extract the body; the envelope adds negligible cost
- Latency impact is negligible — inference calls dominate by 3-4 orders of magnitude
- Target users are engineering teams productionising AI agents — 10s to 100s of agent runs per day, not millions. The debugging and replay value of full HTTP capture far outweighs the marginal overhead at this scale
