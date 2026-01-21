# ADR 001: Coding Agent Vision

## Status

Proposed

## Context

Current coding agents (Claude Code, Aider, Cursor) share a common architecture:
- Full filesystem access
- Arbitrary command execution
- Large cloud models
- LLM generates code directly

This creates security risks and "vibe coding" - systems built by people who don't understand them.

We believe:
- Programming as a skill needs to survive and evolve
- Vibe-coded systems will create massive maintenance burden
- Tools that augment programmers (not replace them) will be valuable
- Security should be architecture, not afterthought

## Decision

Build a coding agent with fundamentally different architecture:

**Self-contained agent.** Runtime provides generic host functions. Agent bundles its own code intelligence (tree-sitter, LSP-lite or rust-analyzer). No language-specific bindings in runtime.

**AST-first, LLM-second.** Tree-sitter for parsing. LSP for code intelligence. LLM for intent interpretation only - not raw code generation.

**Programmer makes decisions.** Tool executes precisely what programmer specifies. "Add timeout parameter to connect" - not "make the networking better."

**CPU-bound tools where they work.** Formatters for formatting. LSP for find-references. Tree-sitter for parsing. LLM only for inherently fuzzy problems.

**Sandboxed.** WASM agent. Filesystem scoped to project via WASI mounts.

## Architecture

```
┌─────────────────────────────────────────────────────────────┐
│  Coding Agent (WASM, self-contained)                        │
│                                                             │
│  ┌─────────────┐  ┌─────────────┐  ┌─────────────┐         │
│  │ Tree-sitter │  │ LSP-lite or │  │    LLM      │         │
│  │ (parsing)   │  │rust-analyzer│  │  (intent)   │         │
│  └─────────────┘  └─────────────┘  └─────────────┘         │
└─────────────────────────────────────────────────────────────┘
         │
         │ Host functions (generic)
         ▼
┌─────────────────────────────────────────────────────────────┐
│  Runtime (no language-specific code)                        │
│  - WASI filesystem (scoped to project)                      │
│  - infer(model, prompt)                                     │
│  - format_file(file, formatter)                             │
└─────────────────────────────────────────────────────────────┘
```

## What This Is

- Power tool for programmers
- Precise transformations (rename, extract, refactor)
- Code search and navigation
- Boilerplate automation

## What This Is NOT

- Vibe coding
- Autonomous agent
- Replacement for understanding code

## Open Questions

1. Does rust-analyzer compile to wasm32-wasip1?
2. What breaks without proc macro support?
3. Is tree-sitter + custom analysis sufficient?
4. Transformation language for LLM output?

## Thesis

The market races toward autonomous AI coding. We bet on:
- Vibe coding creates debt that needs maintenance
- Skilled programmers remain valuable
- Security becomes non-negotiable after first major breach
- Local-first grows

Contrarian positioning. Long-term play.
