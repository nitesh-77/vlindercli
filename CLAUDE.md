# Claude Working Guidelines for VlinderAI

## Workflow

1. Discuss the next domain concept
2. Write ADR (minimal, one decision)
3. Commit ADR
4. Write tests (TDD red phase)
5. Implement (TDD green phase)
6. Commit code

## Decision Making

- Whittle decisions down to the smallest possible increment
- One ADR captures one domain decision
- Don't overreach - focus on what's needed now
- Domain insights first, implementation details later

## ADRs

- Each ADR should be minimal and focused
- Strip out anything that can be deferred
- If an ADR mentions multiple decisions, split them
- Future decisions get their own ADRs when we actually need to make them

## Implementation (TDD)

- Write tests first
- Tests should express the domain model
- Red: write failing test
- Green: minimal code to make it pass
- Refactor: clean up if needed
- Run `cargo test` to verify

## Git

- Never mention Claude in commit messages
- No "Co-Authored-By" lines
- Commit messages should read as if written by the user
