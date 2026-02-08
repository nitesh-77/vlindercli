# Timeline Walkthrough: A Grocery List

A step-by-step example showing the conversation store's git log at every
point. The agent is a todo app that stores items via `kv_put`.

All `git log` commands run inside `~/.vlinder/conversations/`.

---

## Turn 1: "add buy milk"

```
$ vlinder agent run -p agents/todoapp
> add buy milk
Added: buy milk
```

Two commits appear — one for the user's input, one for the agent's response:

```
$ git log --oneline --graph --all
* 6228441 (HEAD -> main) agent
* d30357d user
```

The agent commit carries a `State:` trailer pointing into the state store.
The state store now holds:

```
state commit sc1
  └→ snapshot {"/todos.json" → hash_of('["buy milk"]')}
       └→ value '["buy milk"]'
```

---

## Turn 2: "add buy eggs"

```
> add buy eggs
Added: buy eggs
```

```
$ git log --oneline --graph --all
* 15d739d (HEAD -> main) agent
* 2a1ceef user
* 6228441 agent
* d30357d user
```

State store:

```
sc1 → {"/todos.json" → ["buy milk"]}
 │
sc2 → {"/todos.json" → ["buy milk", "buy eggs"]}
```

Both `sc1` and `sc2` exist. The old value `["buy milk"]` is still in the
database — `sc2` just points to a new value.

---

## Turn 3: "add buy carrots"

```
> add buy carrots
Added: buy carrots
> exit
```

```
$ git log --oneline --graph --all
* 0b1f368 (HEAD -> main) agent
* 9f4c36e user
* 15d739d agent
* 2a1ceef user
* 6228441 agent
* d30357d user
```

State store:

```
sc1 → {"/todos.json" → ["buy milk"]}
 │
sc2 → {"/todos.json" → ["buy milk", "buy eggs"]}
 │
sc3 → {"/todos.json" → ["buy milk", "buy eggs", "buy carrots"]}
```

Six commits in the conversation store. Three state commits in the state
store. Clean alternation: user, agent, user, agent, user, agent.

---

## State continuity: re-running picks up where we left off

```
$ vlinder agent run -p agents/todoapp
Resuming from state sc3…
```

What happened: the CLI ran `git log -- *_todoapp_*.json` in the
conversation store. It scanned backwards from HEAD:

```
0b1f368 agent — subject is "agent" ✓ — has "State: sc3" ✓ → use sc3
```

The agent is initialized with state `sc3`. It reads its todos:

```
> list
1. buy milk
2. buy eggs
3. buy carrots
> exit
```

The git log hasn't changed — no new commits were created because we
only read, didn't write:

```
$ git log --oneline --graph --all
* 0b1f368 (HEAD -> main) agent
* 9f4c36e user
* 15d739d agent
* 2a1ceef user
* 6228441 agent
* d30357d user
```

---

## Fork: "what if I hadn't added carrots?"

We want to go back to the state after "buy eggs". Looking at the
timeline, that's commit `15d739d` (state `sc2`):

```
$ vlinder timeline log
Session: ses-abc12345
  d30357d  user  add buy milk
  6228441 agent  Added: buy milk             State: sc1…
  2a1ceef  user  add buy eggs
  15d739d agent  Added: buy eggs             State: sc2…
  9f4c36e  user  add buy carrots
  0b1f368 agent  Added: buy carrots          State: sc3…
```

Fork:

```
$ vlinder timeline fork 15d739d
Forked timeline at 15d739d → branch fork-15d739d
```

What happened: the conversation store ran
`git checkout -b fork-15d739d 15d739d`. HEAD moved to the new branch.

```
$ git log --oneline --graph --all
* 0b1f368 (main) agent
* 9f4c36e user
* 15d739d (HEAD -> fork-15d739d) agent
* 2a1ceef user
* 6228441 agent
* d30357d user
```

The fork branch points at an ancestor of main. The carrots commits
(`9f4c36e`, `0b1f368`) are still there on `main`, but HEAD is no longer
on main. The divergence is real but not yet visible as a branch in the
graph — that happens when we add commits to the fork.

---

## On the fork: "add buy bread"

```
$ vlinder agent run -p agents/todoapp
Resuming from state sc2…
```

The CLI scanned `git log` from the fork branch's HEAD (`15d739d`), found
`State: sc2`. The agent now has the state from after "buy eggs":

```
> list
1. buy milk
2. buy eggs
```

No carrots. They exist in the state store but `sc2`'s snapshot doesn't
reference them.

```
> add buy bread
Added: buy bread
> exit
```

Now the fork is visible in the graph:

```
$ git log --oneline --graph --all
* 3acba82 (HEAD -> fork-15d739d) agent
* ec7f26b user
| * 0b1f368 (main) agent
| * 9f4c36e user
|/
* 15d739d agent
* 2a1ceef user
* 6228441 agent
* d30357d user
```

Two branches, diverging from `15d739d`:

- **Left (fork):** milk, eggs, bread
- **Right (main):** milk, eggs, carrots

The state store now has four state commits:

```
sc1 → {"/todos.json" → ["buy milk"]}
 │
sc2 → {"/todos.json" → ["buy milk", "buy eggs"]}
 ├── sc3 → {"/todos.json" → ["buy milk", "buy eggs", "buy carrots"]}  (main)
 └── sc4 → {"/todos.json" → ["buy milk", "buy eggs", "buy bread"]}    (fork)
```

`sc3` and `sc4` both have `sc2` as their parent. Same structure as the
git branches in the conversation store. No data was copied — the fork
just created a new state commit from an existing parent.

---

## Switching back to main

```
$ cd ~/.vlinder/conversations && git checkout main
Switched to branch 'main'
```

```
$ git log --oneline --graph --all
* 3acba82 (fork-15d739d) agent
* ec7f26b user
| * 0b1f368 (HEAD -> main) agent
| * 9f4c36e user
|/
* 15d739d agent
* 2a1ceef user
* 6228441 agent
* d30357d user
```

Same graph, but `HEAD` now points to `main`. The fork branch is still
there — nothing was deleted.

```
$ vlinder agent run -p agents/todoapp
Resuming from state sc3…
> list
1. buy milk
2. buy eggs
3. buy carrots
```

Carrots are back. Bread is gone. The state store has everything; both
timelines coexist. The only thing that changed is which state commit
the platform reads from, determined by which git branch HEAD is on.

---

## Summary

| Step | Command | Git log | State |
|------|---------|---------|-------|
| Turn 1 | `add buy milk` | 2 commits, linear | sc1: milk |
| Turn 2 | `add buy eggs` | 4 commits, linear | sc2: milk, eggs |
| Turn 3 | `add buy carrots` | 6 commits, linear | sc3: milk, eggs, carrots |
| Resume | `agent run` | 6 commits, unchanged | reads sc3 from git log |
| Fork | `timeline fork 15d739d` | 6 commits, HEAD moved | — |
| Fork turn | `add buy bread` | 8 commits, branched | sc4: milk, eggs, bread |
| Switch back | `git checkout main` | 8 commits, HEAD moved | reads sc3 |

The conversation store is a git repo. The state store is SQLite with
git's object model. They link via the `State:` trailer. Forking is
`git checkout -b`. State lookup is `git log`. Everything else follows.
