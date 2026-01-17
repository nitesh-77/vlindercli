# Claude Working Guidelines for VlinderAI

## Decision Making

- Whittle decisions down to the smallest possible increment
- One ADR captures one domain decision
- Don't overreach - focus on what's needed now
- Implementation details come later, domain insights come first

## ADRs

- Each ADR should be minimal and focused
- Strip out anything that can be deferred
- If an ADR mentions multiple decisions, split them
- Future decisions get their own ADRs when we actually need to make them

## Building

- Start with the smallest thing that proves the domain model
- Don't optimize before writing code
- Preserve optionality - avoid lock-in to specific implementations
- Extend later when needed, not before

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
