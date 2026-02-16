# ADR 089: Submission Branches

## Status

Proposed

## Context

The DAG worker receives events off the queue and writes them as git commits. Today, every event lands linearly on `main`. The developer has no visibility into where an event falls on the DAG — which submission it belongs to, whether that submission succeeded or failed, or how events relate to each other.

`SubmissionId` exists as a correlation key scattered across messages, but there's no structural representation. The DAG is flat.

### Each event is a tick of the state machine

Each event in a submission's lifecycle (invoke, request, response, complete) does something to the main timeline. What it does is a configuration choice, not a platform decision.

We built a submission-branched DAG worker to validate the concept (branch `error-message-adr`). The experiment taught us:

1. Events can be mapped to specific DAG representations.
2. If the platform isn't conscious about it, it ends up choosing the representation for the user.
3. The user should choose it — they know their agent's trust profile.
4. But if they don't choose, it should fall in place as a sensible default.

### Git platforms provide governance

Git is infrastructure. Vlinder is the platform on top.

The conversations repo is a real git repo. The developer's existing tools, policies, and governance just work:

- GitHub branch protection on `main` → merge policy
- Pull requests → review of agent outcomes
- GitHub Actions → automated checks on submission branches
- Required reviewers → human-in-the-loop
- CODEOWNERS → who's responsible for which agent's outcomes
- Notifications → agent submission needs review

Vlinder doesn't need to build governance. Merge policies, branch protection, review workflows, access control — these are solved problems.

### Identities are git authors

Agent IDs and service IDs are already git commit authors (ADR 069). Push to a git platform and those become real identities:

- `git blame` → which agent/service produced this outcome
- Contributor graphs → which agents are most active
- CODEOWNERS → assign responsibility per agent to a human developer

Every domain entity that writes a git commit needs an identity: every agent in the registry, every platform service (infer, kv, vec, embed), and the harness. One identity per entity.

The developer issues identities and stores them in the secret store. Vlinder reads identities from the secret store and writes commits with matching authors. Vlinder is an identity consumer, not an identity provider.

If the developer hasn't configured an identity, Vlinder falls back to auto-generated defaults.

## Decision

Deferred. The experiment validated that submission branches work mechanically (see `error-message-adr` branch). What remains is deciding how the developer configures what each event does to the timeline — and what the sensible default is.
