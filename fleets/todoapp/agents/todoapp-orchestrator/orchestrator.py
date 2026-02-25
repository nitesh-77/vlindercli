"""todoapp-orchestrator — exercises delegate/wait via echo-container.

Delegates user input to echo-container and returns the result.
Supports single delegation, parallel fan-out, and sequential chaining.

Commands:
    echo <text>     Delegate once to echo-container.
    fan <text>      Fan out to echo-container 3x in parallel, collect all.
    chain <text>    Chain 3 sequential delegations (output feeds next input).
    help            Show usage.

Anything else delegates once to echo-container with the raw input.
"""

import json
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
# Commands
# =========================================================================

def run_echo(text):
    """Single delegation to echo-container."""
    handle = delegate("echo-container", text)
    return wait_for(handle)


def run_fan(text):
    """Fan out 3 delegations in parallel, collect results."""
    handles = []
    for i in range(3):
        handles.append(delegate("echo-container", f"[{i+1}] {text}"))

    lines = ["=== Fan-out (3 parallel delegations) ===", ""]
    for i, handle in enumerate(handles):
        try:
            result = wait_for(handle).replace("\n", " ")
            if len(result) > 200:
                result = result[:200] + "..."
            lines.append(f"[{i+1}] {result}")
        except Exception as e:
            lines.append(f"[{i+1}] ERROR: {e}")
    return "\n".join(lines)


def run_chain(text):
    """Chain 3 sequential delegations — each output feeds the next input."""
    lines = ["=== Chain (3 sequential delegations) ===", ""]
    current = text
    for i in range(3):
        try:
            handle = delegate("echo-container", current)
            current = wait_for(handle)
            display = current.replace("\n", " ")
            if len(display) > 200:
                display = display[:200] + "..."
            lines.append(f"step {i+1}: {display}")
        except Exception as e:
            lines.append(f"step {i+1}: ERROR: {e}")
            break
    return "\n".join(lines)


USAGE = """todoapp-orchestrator commands:
  echo <text>   Delegate once to echo-container
  fan <text>    Fan out 3 parallel delegations
  chain <text>  Chain 3 sequential delegations
  help          Show this message"""


def dispatch(text):
    parts = text.strip().split(None, 1)
    if not parts:
        return USAGE
    cmd = parts[0].lower()
    rest = parts[1] if len(parts) > 1 else ""

    if cmd == "echo":
        return run_echo(rest or "ping")
    elif cmd == "fan":
        return run_fan(rest or "hello")
    elif cmd == "chain":
        return run_chain(rest or "hello")
    elif cmd == "help":
        return USAGE
    else:
        # Default: single echo delegation with raw input
        return run_echo(text)


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
