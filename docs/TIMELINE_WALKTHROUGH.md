# Session Walkthrough: A Grocery List

A step-by-step example showing state at every turn, through forking
and switching back.

---

## Turn 1: "add buy milk"

```
$ vlinder agent deploy -p agents/todoapp
$ vlinder agent run todoapp
> add buy milk
Added: buy milk
```

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

State store:

```
sc1 → {"/todos.json" → ["buy milk"]}
 │
sc2 → {"/todos.json" → ["buy milk", "buy eggs"]}
 │
sc3 → {"/todos.json" → ["buy milk", "buy eggs", "buy carrots"]}
```

---

## State continuity: re-running picks up where we left off

```
$ vlinder agent run todoapp
Resuming from state sc3…
```

The platform found `State: sc3` from the last completed turn.
The agent is initialized with state `sc3`:

```
> list
1. buy milk
2. buy eggs
3. buy carrots
> exit
```

---

## Fork: "what if I hadn't added carrots?"

We want to go back to the state after "buy eggs" — state `sc2`:

```
$ vlinder session fork <session-id> --from sc2 --name fix
```

This creates a branch called `fix` from the state after eggs were added.

---

## On the fork: "add buy bread"

```
$ vlinder agent run todoapp --branch fix
Resuming from state sc2…
```

The agent now has the state from after "buy eggs":

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

The state store now has four state commits:

```
sc1 → {"/todos.json" → ["buy milk"]}
 │
sc2 → {"/todos.json" → ["buy milk", "buy eggs"]}
 ├── sc3 → {"/todos.json" → ["buy milk", "buy eggs", "buy carrots"]}  (main)
 └── sc4 → {"/todos.json" → ["buy milk", "buy eggs", "buy bread"]}    (fix)
```

`sc3` and `sc4` both have `sc2` as their parent. No data was copied —
the fork just created a new state commit from an existing parent.

---

## Check both branches

```
$ vlinder session branches <session-id>
main             → milk, eggs, carrots
fix              → milk, eggs, bread
```

---

## Promote the fix

```
$ vlinder session promote fix
Old main sealed as broken-2026-03-11.
```

```
$ vlinder agent run todoapp
Resuming from state sc4…
> list
1. buy milk
2. buy eggs
3. buy bread
```

Bread instead of carrots. The old timeline is sealed, never deleted.

---

## Summary

| Step | Command | State |
|------|---------|-------|
| Turn 1 | `add buy milk` | sc1: milk |
| Turn 2 | `add buy eggs` | sc2: milk, eggs |
| Turn 3 | `add buy carrots` | sc3: milk, eggs, carrots |
| Resume | `agent run todoapp` | reads sc3 |
| Fork | `session fork --from sc2 --name fix` | — |
| Fork turn | `add buy bread` | sc4: milk, eggs, bread |
| Promote | `session promote fix` | sc4 becomes canonical |

The state store is a content-addressed append-only database. Forking
reads from a different point in the same store. State lookup follows
the branch. Everything else follows.
