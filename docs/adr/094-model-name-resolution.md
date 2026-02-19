# ADR 094: Agent manifests reference models by registry name

## Status

Accepted

## Context

Agent manifests reference models by `model_path` URI:

```toml
[requirements.models]
inference_model = "openrouter://anthropic/claude-sonnet-4"
```

Changing a model's provider requires editing every agent manifest that
uses it. `Model.name` is derived from `model_path` via `name_from_model_path()`,
producing provider-native strings like `"anthropic/claude-sonnet-4"`.

## Decision

Agent manifests reference models by registry name:

```toml
[requirements.models]
inference_model = "claude-sonnet"
embedding_model = "nomic-embed"
```

`Model.name` uses the manifest's `name` field. `pfname()` (provider-friendly
name) derives the provider-native identifier from `model_path` for API calls.

When the alias and registry name are the same, an array shorthand is supported:

```toml
[requirements]
models = ["claude-sonnet", "nomic-embed"]
```

Both forms can coexist. The table form is for aliasing; the array form
is for the common case where no alias is needed.

## Consequences

- Switching a model's provider only changes the model manifest, not agent manifests.
- `Agent.requirements.models` values become `String` (registry names) instead of `ResourceId` (URIs).
- Registry resolves by name instead of by model_path URI.
- Workers resolve alias → registry name → Model, then use `pfname()` for provider API calls.
- Convention: agent manifests are the indirection layer between agent code and model identity.
