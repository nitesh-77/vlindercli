"""todoapp-orchestrator — delegation smoke test.

Delegates to all five test agents (echo-container, sqlite-kv-test,
sqlite-vec-test, ollama-test, openrouter-test) in parallel, collects
results, and returns a formatted report. Mirrors todoapp's full
capability surface via delegation:

  - Inference (OpenRouter) → openrouter-test
  - Embeddings (Ollama)    → ollama-test
  - KV storage             → sqlite-kv-test
  - Vector storage         → sqlite-vec-test
  - Basic I/O              → echo-container

Commands:
    smoke               Run all delegates and report results.
    echo <text>         Delegate to echo-container only.
    kv <cmd> <args...>  Delegate to sqlite-kv-test (e.g. kv put /x hello).
    vec <cmd> <args...> Delegate to sqlite-vec-test (e.g. vec store mykey data).
    infer <text>        Delegate to openrouter-test.
    embed <text>        Delegate to ollama-test (embedding endpoint).
    help                Show usage.
"""

import json
import time
import urllib.request
from http.server import HTTPServer, BaseHTTPRequestHandler

RUNTIME_HOST = "http://runtime.vlinder.local"


def delegate(agent, input_text):
    """Fire-and-forget delegation via runtime. Returns a handle."""
    data = json.dumps({"target": agent, "input": input_text}).encode()
    req = urllib.request.Request(f"{RUNTIME_HOST}/delegate", data=data, method="POST")
    req.add_header("Content-Type", "application/json")
    req.add_header("Host", "runtime.vlinder.local")
    with urllib.request.urlopen(req) as resp:
        result = json.loads(resp.read())
    return result["handle"]


def wait_for(handle):
    """Block until a delegated task completes. Returns the output."""
    data = json.dumps({"handle": handle}).encode()
    req = urllib.request.Request(f"{RUNTIME_HOST}/wait", data=data, method="POST")
    req.add_header("Content-Type", "application/json")
    req.add_header("Host", "runtime.vlinder.local")
    with urllib.request.urlopen(req) as resp:
        return resp.read().decode()


def parse_payload(raw):
    """Extract the latest user message from a harness payload."""
    lines = raw.strip().splitlines()
    if lines and lines[-1].startswith("User: "):
        return lines[-1][len("User: "):]
    return raw.strip()


# =========================================================================
# Ollama-test accepts "Option N:\n<input>" to skip its menu round-trip.
# =========================================================================

def ollama_embed_payload(text):
    """Build an option-prefix payload that selects the embed endpoint."""
    return f"Option 4:\n{text}"


# =========================================================================
# Commands
# =========================================================================

def run_smoke(user_input):
    """Delegate to all five test agents in parallel, collect results."""
    timestamp = str(int(time.time()))
    kv_path = f"/smoke/{timestamp}"
    kv_content = f"smoke-test-{timestamp}"
    vec_key = f"smoke-{timestamp}"

    results = {}
    errors = {}

    # Fire all five delegates (parallel fan-out)
    handles = {}
    targets = {
        "echo": ("echo-container", user_input),
        "kv-put": ("sqlite-kv-test", f"put {kv_path} {kv_content}"),
        "vec-store": ("sqlite-vec-test", f"store {vec_key} smoke test data"),
        "openrouter": ("openrouter-test", f"Reply in exactly 5 words: {user_input}"),
        "ollama-embed": ("ollama-test", ollama_embed_payload(user_input)),
    }

    for name, (agent, payload) in targets.items():
        try:
            handles[name] = delegate(agent, payload)
        except Exception as e:
            errors[name] = str(e)

    # Wait for all results (sequential collection)
    for name, handle in handles.items():
        try:
            results[name] = wait_for(handle)
        except Exception as e:
            errors[name] = str(e)

    # Fire follow-up reads (verify kv write persisted, vec can search)
    follow_up_handles = {}
    if "kv-put" in results:
        try:
            follow_up_handles["kv-get"] = delegate("sqlite-kv-test", f"get {kv_path}")
        except Exception as e:
            errors["kv-get"] = str(e)
    if "vec-store" in results:
        try:
            follow_up_handles["vec-search"] = delegate("sqlite-vec-test", f"search {vec_key}")
        except Exception as e:
            errors["vec-search"] = str(e)

    for name, handle in follow_up_handles.items():
        try:
            results[name] = wait_for(handle)
        except Exception as e:
            errors[name] = str(e)

    # Format report
    all_steps = ["echo", "kv-put", "kv-get", "vec-store", "vec-search",
                 "openrouter", "ollama-embed"]
    lines = ["=== Delegation Smoke Test ===", ""]
    for name in all_steps:
        if name in results:
            # Truncate long results for readability
            value = results[name].replace("\n", " ")
            if len(value) > 200:
                value = value[:200] + "..."
            lines.append(f"[PASS] {name}: {value}")
        elif name in errors:
            lines.append(f"[FAIL] {name}: {errors[name][:200]}")

    passed = sum(1 for n in all_steps if n in results)
    failed = sum(1 for n in all_steps if n in errors)
    lines.append("")
    lines.append(f"--- {passed} passed, {failed} failed ---")

    return "\n".join(lines)


def run_echo(text):
    """Delegate to echo-container."""
    handle = delegate("echo-container", text)
    return wait_for(handle)


def run_kv(args_text):
    """Delegate to sqlite-kv-test."""
    handle = delegate("sqlite-kv-test", args_text)
    return wait_for(handle)


def run_vec(args_text):
    """Delegate to sqlite-vec-test."""
    handle = delegate("sqlite-vec-test", args_text)
    return wait_for(handle)


def run_infer(text):
    """Delegate to openrouter-test (OpenRouter inference)."""
    handle = delegate("openrouter-test", text)
    return wait_for(handle)


def run_embed(text):
    """Delegate to ollama-test (Ollama embedding)."""
    handle = delegate("ollama-test", ollama_embed_payload(text))
    return wait_for(handle)


USAGE = """todoapp-orchestrator commands:
  smoke               Run all delegates and report results
  echo <text>         Delegate to echo-container
  kv <cmd> <args...>  Delegate to sqlite-kv-test
  vec <cmd> <args...> Delegate to sqlite-vec-test
  infer <text>        Delegate to openrouter-test (OpenRouter)
  embed <text>        Delegate to ollama-test (Ollama embedding)
  help                Show this message"""


def dispatch(text):
    parts = text.strip().split(None, 1)
    if not parts:
        return USAGE
    cmd = parts[0].lower()
    rest = parts[1] if len(parts) > 1 else ""

    if cmd == "smoke":
        return run_smoke(rest or "hello from orchestrator")
    elif cmd == "echo":
        return run_echo(rest or "ping")
    elif cmd == "kv":
        return run_kv(rest) if rest else "usage: kv <put|get|list|delete> <args...>"
    elif cmd == "vec":
        return run_vec(rest) if rest else "usage: vec <store|search|delete> <args...>"
    elif cmd == "infer":
        return run_infer(rest or "Say hello in exactly 5 words.")
    elif cmd == "embed":
        return run_embed(rest or "hello world")
    elif cmd == "help":
        return USAGE
    else:
        # Default: run smoke test with the full input
        return run_smoke(text)


# =========================================================================
# HTTP server
# =========================================================================

class Handler(BaseHTTPRequestHandler):
    def do_GET(self):
        self.send_response(200)
        self.end_headers()

    def do_POST(self):
        length = int(self.headers.get("Content-Length", 0))
        body = self.rfile.read(length).decode()
        user_input = parse_payload(body)
        response = dispatch(user_input)
        data = response.encode()
        self.send_response(200)
        self.send_header("Content-Length", str(len(data)))
        self.end_headers()
        self.wfile.write(data)

    def log_message(self, fmt, *args):
        pass


if __name__ == "__main__":
    print("todoapp-orchestrator listening on :8080", flush=True)
    HTTPServer(("0.0.0.0", 8080), Handler).serve_forever()
