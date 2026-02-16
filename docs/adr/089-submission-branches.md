# ADR 089: DAG Event Strategy

## Status

Draft

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

## Decision

Deferred.
