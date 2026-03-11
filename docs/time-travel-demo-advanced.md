# Time Travel Demo — Advanced

The full time-travel workflow: repair a broken turn **and** recover
good work that happened after the failure.

See `docs/time-travel-demo.md` for the simple version.

## Prerequisites

Same as the simple demo:

```bash
git clone https://github.com/vlindercli/vlindercli.git
cd vlindercli
just build

ollama pull nomic-embed-text
export VLINDER_OPENROUTER_API_KEY=<your-key>
just build-todoapp
```

```bash
VLINDER=./target/debug/vlinder
```

---

## The plan

```
  main:  milk ── bread ── ERROR ── butter ── coffee
                    │
                    │  fork here, then repair
                    │
  fix:              └── eggs ── butter' ── coffee'
                                              │
                                              └── promoted to main
```

Items 1, 2, 4, 5 succeed. Item 3 fails because Ollama is down.
We go back, fix item 3, recover the good work (items 4 and 5)
from the broken session, and promote.

---

## Step 1: Deploy and add two items

```bash
$VLINDER agent deploy -p agents/todoapp
$VLINDER agent run todoapp
```

```
> add milk
Added: milk

> add bread
Added: bread
```

---

## Step 2: Kill Ollama, trigger an error

Open `http://localhost:11434` in a browser — "Ollama is running".

```bash
brew services stop ollama
```

Refresh — connection refused.

```
> add eggs
Error: embedding failed — connection refused
```

---

## Step 3: Restart Ollama, finish the list

```bash
brew services start ollama
```

```
> add butter
Added: butter

> add coffee
Added: coffee

> exit
```

---

## Step 4: Inspect the session

```bash
$VLINDER session list todoapp
```

Five turns. The third is the failure. Butter and coffee succeeded
after the error — we want to keep that work.

---

## Step 5: Fork from the last good turn

Find the state hash from the bread turn (last good point before the error):

```bash
$VLINDER session fork <session-id> --from <bread-state-hash> --name fix-eggs
```

---

## Step 6: Repair — re-add the failed item

```bash
$VLINDER agent run todoapp --branch fix-eggs
```

```
Resuming from state <bread-state-hash>…
> list
1. milk
2. bread

> add eggs
Added: eggs

> add butter
Added: butter

> add coffee
Added: coffee

> exit
```

All five items are now cleanly recorded on the `fix-eggs` branch.

---

## Step 7: Promote

```bash
$VLINDER session promote fix-eggs
```

```
Old main sealed as broken-2026-03-11.
```

---

## Step 8: Verify

```bash
$VLINDER agent run todoapp
```

```
> list
1. milk
2. bread
3. eggs
4. butter
5. coffee
```

Five items. No error. Clean history.

The broken session is still there — sealed, never deleted.

---

## What happened

| Step | Command | What it does |
|------|---------|-------------|
| Inspect | `session list` | See all sessions for an agent |
| Fork | `session fork --from --name` | Branch from a known-good state |
| Run on branch | `agent run --branch` | Resume agent from fork point |
| Promote | `session promote` | Make the fix branch canonical, seal the old one |

No data was lost. Both timelines exist. You chose which one is canonical.
