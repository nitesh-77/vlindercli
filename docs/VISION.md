# VlinderAI

Local-first AI agent runtime with portable agent packaging.

## The Bet

A handful of good models can power an exponential number of agents.

Model building requires massive resources - data, compute, expertise. But agent building should be accessible to many. The ecosystem grows when more people can build agents on top of fewer, well-optimized models.

## Target Hardware

Consumer machines:
- Mac (Apple Silicon)
- Gaming rigs (NVIDIA GPU)
- Mini PCs (Beelink, Intel NUC)
- NAS with compute

Initial audience: homelab enthusiasts, r/selfhosted, r/LocalLLaMA types.

This constraint shapes everything. We optimize for local, not cloud. Constrained memory, not infinite scale.

## Abstraction Levels

```
Model builders     (few)      → train and optimize models
Agent builders     (many)     → compose agents using models
Users              (most)     → use agents to accomplish goals
```

VlinderAI focuses on the **agent builder**. We make it easy to build agents by:
- Abstracting away model loading, GPU scheduling, memory management
- Letting agent builders focus on business logic, prompts, and behavior
- Handling the hard local-hardware problems in the runtime

**Agent builders think in:** model names, system prompts, temperatures.

**Agent builders don't think about:** Rust crates, C++ bindings, GGUF formats, quantization.

```rust
// What agent builders write:
Model::inference("phi3")
    .system_prompt("You summarize articles concisely")
    .temperature(0.7)

// What they DON'T write:
LlamaEngine::load(Path::new("models/phi-3-mini-4k-instruct-q4.gguf"))
    .with_context_params(...)
```

The runtime handles the translation from "phi3" to the actual model file, loader, and hardware configuration.

## Problem

Hugging Face is npm without package.json — all libraries, no applications. Users can download models, but a model does nothing on its own. There's no composability, no way to download something and just run it, no way to chain inputs and outputs without writing code.

VlinderAI is the application layer. Download an agent, run it — like `vagrant up` but for AI workflows.

## Core Concept

An agent is not just a model. An agent is:

- **Model reference** — abstract requirement (e.g., "instruction-7b"), shared across agents
- **Code** — tools, integrations, actions (wasm)
- **Behavior** — prompts, constraints
- **Capability declaration** — what it does, what permissions it needs
- **Memory namespace** — scoped vector storage for RAG
- **Object storage** — scoped file storage
- **Secrets** — scoped credential storage

A single model on disk can power multiple agents with different capabilities.

---

## Core Principle: Text Boundaries

Text is the universal interface. Every agent boundary is text in, text out.

**Why this matters:**

1. **Composability.** Agents don't know what comes before or after them. The orchestrator can chain any agents without adapters or type conversion.

2. **Isolation.** No shared memory, no object references, no typed interfaces between agents. Each agent is a true black box.

3. **Distribution.** Text crosses network boundaries. An agent doesn't know if the orchestrator is local or remote. A future control plane could route across vlindercli instances on different machines — same abstraction, different transport.

4. **Human alignment.** The system speaks the same language the user does. No translation layer between human conversation and agent communication.

5. **Heterogeneous models.** Vision models, speech models, language models — all have different internal representations. Agents translate to/from text at the boundary. The user only sees text.

**The tradeoff:** Text is slower and lossier than binary protocols. An image passed as base64 text is inefficient. Structured data serialized to text and back adds overhead.

**Why it's worth it:** The constraint enables everything else. Typed interfaces between agents would lock in co-location, require version compatibility, create coupling. Text boundaries keep agents independent — swappable, distributable, composable.

This is a deliberate architectural constraint, not a convenience.

---

## Wire Format

Agent communication is text. The orchestrator and runtime are multi-format aware. Agents don't have to be.

**Supported formats:**
- [TOON](https://github.com/toon-format/toon) — compact, structured, token-efficient
- JSON — universal, familiar
- Plain text — when structure isn't needed

Agents output whatever format is natural for them. JSON because it's easy, plain text because it's simple. The orchestrator parses it regardless.

Format optimization happens at the infrastructure layer, not in every agent. Agent authors don't need to learn TOON or think about wire efficiency.

**Example: Agent outputs JSON**
```json
{
  "status": "success",
  "episodes": [{"title": "Planet Money #847", "duration": "23:41"}]
}
```

**Example: Agent outputs plain text**
```
Found 3 new episodes. Latest is "Planet Money #847" about t-shirt manufacturing.
```

**Example: Agent outputs TOON**
```
episodes[1]{title,duration}:
  Planet Money #847,23:41
```

The orchestrator understands all of these. The constraint is text boundaries, not a specific format.

---

## Architecture

### Runtime (`vlindercli`)

The runtime is the foundation. Implemented in Rust.

**Responsibilities:**
- Load/unload agents
- Own model inference (agents call runtime, not models directly)
- Manage model lifecycle on consumer hardware
- Provide scoped storage APIs (vector, object, secrets)
- Enforce permissions
- Persist conversation histories

**Interfaces:**
- CLI mode: interactive terminal, single conversation thread
- Daemon mode: web UI over local port, multiple conversation threads

Both modes use identical runtime logic. Stateless request handling — everything needed to continue a conversation is in persisted storage or conversation history.

### Orchestrator

A privileged agent loaded on startup. Everything else is dynamic.

**Responsibilities:**
- Route user intent to appropriate agent
- Build DAGs for multi-agent workflows
- Install agents dynamically when capabilities needed
- Handle permission prompts
- Own user personality and accumulated memory

**Key behaviors:**
- Queries installed agent capabilities to determine routing
- Can query artifactory for uninstalled capabilities
- Constructs prompts, passes to agents, collects text outputs
- Uses own namespaced storage as scratch space for workflow coordination

**The orchestrator is optional.** Any agent can run standalone:
```
vlindercli run podcast-agent
```

### Agents

Packaged as a Vlinderbox containing:
- Model reference (abstract, e.g., "instruction-7b")
- Wasm binary (tool logic, prompt construction)
- Behavior definition (system prompts, constraints)
- Capability declaration (routing hints + permission manifest)

**Principles:**
- Self-contained: owns full domain workflow, constructs own prompts
- Black box: receives text, returns text, no shared state with other agents
- Standalone: works without orchestrator
- Single responsibility: does one thing; when in doubt, split into two agents

**Inheritance:** Vlinderbox can specify parent and override parts (like Vagrant/Docker).

**Versioning:** Semantic versioning. CLI notifies of updates, prompts on next use.

### Wasm Sandbox

Agent code runs in wasm for:
- Sandboxing (arbitrary code, contained)
- Language agnosticism (anything compiling to wasm)
- Portability

Wasm handles real logic: HTTP calls, RSS parsing, data transformation, prompt construction. Inference calls go to runtime.

---

## Inference

### Boundary

Runtime owns all model inference. Agents call synchronous API:

```rust
// Conceptual interface
fn infer(
    model_class: String,      // e.g., "instruction-7b", "vision", "speech-to-text"
    prompt: String,
    params: InferenceParams,  // temperature, max_tokens, stop_sequences
) -> Result<String, InferenceError>
```

Agent specifies model class and parameters. Runtime resolves to loaded model, handles inference, returns result.

No streaming at this boundary. Agents get complete responses. Streaming only at outer edge (user-facing UI).

### Model Management

Different capabilities need different models (language, vision, speech).

**Download:** Agent install downloads required model if not present locally.

**Deduplication:** Models stored once. Multiple agents referencing same model share one file on disk.

**Lifecycle (initial approach):**
- Naive scheduling: load when needed, unload when done
- User waits during swaps — price of local AI

**Lifecycle (future optimization):**
- DAG lookahead: preload next model while current inference runs
- Overlap compute and I/O

---

## Storage

### Per-Agent Scoped Storage

Each agent gets isolated access to:

| Type | Purpose |
|------|---------|
| Vector store | RAG, embeddings, semantic search |
| Object store | Files, artifacts, binary data |
| Secret store | Credentials from OAuth flows |

Agents only access own namespace. Orchestrator has own namespace for workflow scratch space.

### Secrets Model

Agents handle own auth flows:
```
"I need access to your YouTube account. Say yes and I will open a browser window."
```

User completes OAuth in browser. Agent stores credentials.

**Critical constraint:** Agent can only read secrets it wrote. No cross-agent credential access. Scoping automatic.

### Conversation History

Runtime persists conversation histories. Each thread (CLI session or daemon conversation) has own history. Loaded with each request, appended after response.

---

## Permissions

Capability declaration serves dual purpose:
1. Routing hint for orchestrator
2. Permission manifest for runtime

**Model:**
- Network: open by default
- Default permissions: granted at install
- Sensitive permissions (mic, camera, scoped filesystem): prompted dynamically

```
"I will need to access your mic to listen to this song. Can I do that?"
```

**Upgrades:** New version requiring new permissions prompts before activation.

---

## Orchestrator Routing

### Intent Matching

Orchestrator uses model to match user intent to agent capabilities. Agent capability declarations form context.

**Approach:** Test with off-the-shelf models first. May work without custom training. Must be reliable.

### DAG Execution

Complex requests decompose to dependency graph:
1. Orchestrator constructs prompt for step 1
2. Passes to agent, receives text
3. Uses output to construct prompt for step 2
4. Repeat

Agents are unaware of each other. Text in, text out at every boundary.

**Errors:** Surface conversationally.
```
"I'm not able to do that at the moment. Network appears down."
```

### Dynamic Installation

```
"Can you tell me what's in this picture?"
"Let me download the computer vision agent, give me a moment."
"This is a cat."
```

Orchestrator queries artifactory for capabilities, offers to install.

---

## Packaging: Vlinderbox Format

Portable, self-contained agent definition.

**Contents:**
```
my-agent/
├── vlinderbox.yaml    # Manifest: model reqs, capabilities, permissions
├── agent.wasm         # Compiled agent logic
├── behavior.yaml      # Prompts, constraints
└── assets/            # Static files if needed
```

**Model references:** Abstract (e.g., "instruction-7b-class"), resolved at install.

**Inheritance:** Specify parent, override parts.

---

## Distribution: Artifactory

Registry of Vlinderboxes.

**Structure:**
- Official agents: bootstrap ecosystem
- Community agents: anyone publishes

**Trust:** Based on declared permissions, not publisher. Official is not privileged.

**Initial implementation:** GitHub repo with JSON index.
```
vlindercli install github:username/agent-name
```

---

## User Experience

### Sample Session

```
$ vlindercli
"Welcome back. What can I do for you?"
"I need to buy tomatoes when I grocery shop next."
"Understood."
"BTW, I thought of a new agent idea: a fake news verifier."
"Cool idea! Let me note it down."
```

Later:
```
$ vlindercli
"Welcome back. What can I do for you?"
"What was that agent idea I mentioned?"
"You wanted a fake news verifier."
"Going to the store. Anything I should know?"
"You need to buy tomatoes."
```

### Personality

Orchestrator owns personality. Other agents are pure functionality. Same agents, different orchestrator = different conversational style.

### Memory as Moat

Orchestrator's vector store accumulates over time. Preferences, ideas, history. Local and owned, but sticky.

---

## Implementation Language

Rust.

---

## Deferred

- Remote access (Tailscale-style access from other devices)
- Multi-device sync
- Conversational agent authoring
- Advanced scheduling optimizations
- Threaded CLI conversations

---

## First Milestone

**Note-taker agent, standalone (no orchestrator).**

Proves:
- Runtime loads wasm agent
- Agent calls runtime for inference
- Vector storage works for RAG
- CLI interface works
- Actually useful

Components needed:
1. Minimal runtime with inference API
2. Vector storage with scoped namespace
3. Wasm host that can call runtime
4. Single agent: note-taker
5. CLI interface

Orchestrator comes after this works.

---

## Development Guidance

When implementing, prioritize:
1. Get one agent working end-to-end before generalizing
2. Naive implementations first, optimize when needed
3. Interface boundaries matter more than internals
4. Test assumptions early (wasm overhead, intent matching, Ollama control)

When making architecture decisions, remember:
- Agents are self-contained black boxes (text in, text out)
- Runtime owns inference, storage, permissions
- Orchestrator is optional — agents work standalone
- Everything is stateless; state lives in storage

