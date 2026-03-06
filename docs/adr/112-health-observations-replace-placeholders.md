# ADR 112: Health Observations Replace Diagnostic Placeholders

**Status:** Draft

## Context

`RuntimeDiagnostics::placeholder()` and `ServiceDiagnostics::placeholder()`
fabricate fake diagnostic data — zero durations, "unknown" container IDs,
dummy service types. They exist because real observational data isn't
flowing into the diagnostics structs at the point where messages are
constructed.

Separately, `HealthSnapshot` and `HealthWindow` capture real observations
— latency, status codes, timestamps — but are disconnected from the
message diagnostics pipeline. They sit in the same file by coincidence,
unused by the code paths that build `RuntimeDiagnostics` and
`ServiceDiagnostics`.

These are the same concept. The placeholders are gaps where health
observations should go. The health types are the data that fills those
gaps.

## Decision

Health observations feed diagnostics. When the runtime builds a
`RuntimeDiagnostics` for a CompleteMessage, it reads from captured
health observations instead of calling `placeholder()`. Same for
service workers building `ServiceDiagnostics`.

The placeholder methods are deleted. Any code path that currently calls
`placeholder()` must instead supply real observations or propagate the
absence explicitly (e.g., `Option<RuntimeDiagnostics>`).

## Consequences

- `placeholder()` methods are removed — no more fabricated data in the DAG
- Health observations become part of the diagnostics pipeline, not a
  separate concern
- Code paths that can't supply real data must handle the absence
  explicitly rather than masking it
- `diagnostics.rs` splits naturally once health observations and message
  diagnostics are unified — the file structure follows from the design,
  not the other way around
