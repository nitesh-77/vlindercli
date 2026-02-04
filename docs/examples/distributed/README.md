# Distributed Deployment Examples

This directory contains example configurations for running Vlinder across multiple machines.

## Architecture Overview

```
┌─────────────────────────────────────────────────────────────────┐
│                         NATS Server                              │
│                    nats://nats.local:4222                        │
└─────────────────────────────────────────────────────────────────┘
        │                    │                    │
        ▼                    ▼                    ▼
┌───────────────┐   ┌───────────────┐   ┌───────────────┐
│   Machine 1   │   │   Machine 2   │   │   Machine 3   │
│               │   │               │   │               │
│  • Registry   │   │  • Inference  │   │  • Object     │
│  • WASM       │   │    (Ollama)   │   │    Storage    │
│    Agents     │   │  • Embedding  │   │  • Vector     │
│               │   │    (Ollama)   │   │    Storage    │
└───────────────┘   └───────────────┘   └───────────────┘
```

## Prerequisites

1. **NATS Server** with JetStream enabled, accessible from all machines:
   ```bash
   nats-server -js -p 4222
   ```

2. **Ollama** running on the inference machine:
   ```bash
   ollama serve
   ```

3. **Vlinder binary** installed on all machines

## Configuration Files

Each machine runs with its own config file:

- `machine1-registry-wasm.toml` - Registry service + WASM agent runtimes
- `machine2-inference.toml` - Inference and embedding workers (Ollama)
- `machine3-storage.toml` - Object and vector storage workers

## Quick Start

### Machine 1: Registry + Agents

```bash
export VLINDER_DIR=/var/lib/vlinder
cp machine1-registry-wasm.toml $VLINDER_DIR/config.toml
vlinder daemon
```

### Machine 2: Inference

```bash
export VLINDER_DIR=/var/lib/vlinder
cp machine2-inference.toml $VLINDER_DIR/config.toml
vlinder daemon
```

### Machine 3: Storage

```bash
export VLINDER_DIR=/var/lib/vlinder
cp machine3-storage.toml $VLINDER_DIR/config.toml
vlinder daemon
```

## Manual Worker Control

You can also run individual workers manually for debugging or scaling:

```bash
# Run an additional WASM agent worker
VLINDER_WORKER_ROLE=agent-wasm vlinder daemon

# Run an additional inference worker
VLINDER_WORKER_ROLE=inference-ollama vlinder daemon
```

Available worker roles:
- `registry` - Registry gRPC service
- `agent-wasm` - WASM agent runtime
- `inference-ollama` - Ollama inference service
- `embedding-ollama` - Ollama embedding service
- `storage-object-sqlite` - SQLite object storage
- `storage-object-memory` - In-memory object storage
- `storage-vector-sqlite` - SQLite-vec vector storage
- `storage-vector-memory` - In-memory vector storage

## Environment Variables

All config values can be overridden via environment variables:

| Variable | Description | Default |
|----------|-------------|---------|
| `VLINDER_DISTRIBUTED_ENABLED` | Enable distributed mode | `false` |
| `VLINDER_DISTRIBUTED_REGISTRY_ADDR` | Registry gRPC address | `http://127.0.0.1:9090` |
| `VLINDER_QUEUE_BACKEND` | Queue backend (`memory` or `nats`) | `memory` |
| `VLINDER_QUEUE_NATS_URL` | NATS server URL | `nats://localhost:4222` |
| `VLINDER_WORKERS_AGENT_WASM` | Number of WASM workers | `1` |
| `VLINDER_WORKERS_INFERENCE_OLLAMA` | Number of inference workers | `1` |
| `VLINDER_WORKER_ROLE` | Run as specific worker (for manual control) | - |

## Scaling

To scale a specific service, increase its worker count in the config:

```toml
[distributed.workers.inference]
ollama = 4  # Run 4 inference workers
```

Or run additional workers manually on the same or different machines.

## Monitoring

Check worker status:
```bash
ps aux | grep vlinder
```

View NATS queue depth:
```bash
nats stream info VLINDER
```

## Troubleshooting

**Workers not processing messages:**
- Ensure NATS is accessible from all machines
- Check that `queue.backend = "nats"` is set
- Verify `queue.nats_url` points to the correct server

**Registry connection failures:**
- Ensure registry worker is running
- Check `distributed.registry_addr` matches the registry machine's address

**Ollama inference failures:**
- Verify Ollama is running on the inference machine
- Check `ollama.endpoint` in the config
