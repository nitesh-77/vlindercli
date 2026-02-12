"""Vlinder Agent SDK — state machine agent framework (ADR 075).

Agents are pure state machines. The platform calls POST /handle with events,
the agent returns actions. The platform executes service calls on the agent's
behalf and calls the agent again with the result. Every step is independently
addressable for time-travel debugging.

Usage:
    from vlinder import Agent

    agent = Agent()

    @agent.on_invoke
    def handle(ctx):
        ctx.state["name"] = ctx.input
        ctx.kv_get("/greeting.txt")

    @agent.on_kv_get
    def got_greeting(ctx):
        ctx.complete(f"Hello {ctx.state['name']}, greeting: {ctx.data.decode()}")

    if __name__ == "__main__":
        agent.run()
"""

import base64
import json
from http.server import HTTPServer, BaseHTTPRequestHandler


class Context:
    """Event context passed to handler functions.

    Provides typed methods for requesting platform services.
    Each method sets the action to return to the platform.
    The handler must call exactly one action method.

    Attributes set from the event (None if not applicable to this event type):
        input    — user input text (Invoke)
        data     — bytes from storage (KvGet, converted from int array)
        paths    — list of paths (KvList)
        existed  — bool (KvDelete, VectorDelete)
        text     — inference result (Infer)
        vector   — embedding vector (Embed)
        matches  — search results (VectorSearch)
        output   — delegation result (Delegate)
        message  — error description (Error)
    """

    def __init__(self, event):
        self.type = event["type"]
        self.state = event.get("state", {})
        self._action = None

        # Event-specific fields
        self.input = event.get("input")
        self.data = event.get("data")
        self.paths = event.get("paths")
        self.existed = event.get("existed")
        self.text = event.get("text")
        self.vector = event.get("vector")
        self.matches = event.get("matches")
        self.output = event.get("output")
        self.message = event.get("message")

        # Convert KvGet data from JSON int array to bytes
        if self.data is not None and isinstance(self.data, list):
            self.data = bytes(self.data)

    # -- Service request methods --

    def kv_get(self, path):
        """Request a value from KV storage."""
        self._action = {"action": "KvGet", "path": path, "state": self.state}

    def kv_put(self, path, content):
        """Write a value to KV storage. Content is base64-encoded for transport."""
        if isinstance(content, bytes):
            encoded = base64.b64encode(content).decode()
        else:
            encoded = base64.b64encode(content.encode()).decode()
        self._action = {"action": "KvPut", "path": path, "content": encoded,
                        "state": self.state}

    def kv_list(self, prefix):
        """List keys under a prefix."""
        self._action = {"action": "KvList", "prefix": prefix, "state": self.state}

    def kv_delete(self, path):
        """Delete a key from KV storage."""
        self._action = {"action": "KvDelete", "path": path, "state": self.state}

    def vector_store(self, key, vector, metadata):
        """Store a vector with metadata."""
        self._action = {"action": "VectorStore", "key": key, "vector": vector,
                        "metadata": metadata, "state": self.state}

    def vector_search(self, vector, limit=5):
        """Search for similar vectors."""
        self._action = {"action": "VectorSearch", "vector": vector, "limit": limit,
                        "state": self.state}

    def vector_delete(self, key):
        """Delete a stored vector."""
        self._action = {"action": "VectorDelete", "key": key, "state": self.state}

    def infer(self, model, prompt, max_tokens=256):
        """Request LLM inference."""
        self._action = {"action": "Infer", "model": model, "prompt": prompt,
                        "max_tokens": max_tokens, "state": self.state}

    def embed(self, model, text):
        """Request text embedding."""
        self._action = {"action": "Embed", "model": model, "text": text,
                        "state": self.state}

    def delegate(self, agent, input_text):
        """Delegate work to another agent."""
        self._action = {"action": "Delegate", "agent": agent, "input": input_text,
                        "state": self.state}

    def complete(self, payload):
        """Return the final result to the platform."""
        self._action = {"action": "Complete", "payload": str(payload),
                        "state": self.state}


class Agent:
    """State machine agent framework.

    Register handlers for each event type using decorator methods.
    Call run() to start the HTTP server on port 8080.
    """

    def __init__(self):
        self._handlers = {}

    # -- Handler registration decorators --

    def on_invoke(self, fn):
        self._handlers["Invoke"] = fn
        return fn

    def on_kv_get(self, fn):
        self._handlers["KvGet"] = fn
        return fn

    def on_kv_put(self, fn):
        self._handlers["KvPut"] = fn
        return fn

    def on_kv_list(self, fn):
        self._handlers["KvList"] = fn
        return fn

    def on_kv_delete(self, fn):
        self._handlers["KvDelete"] = fn
        return fn

    def on_vec_store(self, fn):
        self._handlers["VectorStore"] = fn
        return fn

    def on_vec_search(self, fn):
        self._handlers["VectorSearch"] = fn
        return fn

    def on_vec_delete(self, fn):
        self._handlers["VectorDelete"] = fn
        return fn

    def on_infer(self, fn):
        self._handlers["Infer"] = fn
        return fn

    def on_embed(self, fn):
        self._handlers["Embed"] = fn
        return fn

    def on_delegate(self, fn):
        self._handlers["Delegate"] = fn
        return fn

    def on_error(self, fn):
        self._handlers["Error"] = fn
        return fn

    # -- Internal dispatch --

    def _handle(self, event):
        ctx = Context(event)

        # Auto-preserve input in state on invoke
        if ctx.type == "Invoke":
            ctx.state["_input"] = ctx.input

        handler = self._handlers.get(ctx.type)
        if handler is None:
            if ctx.type == "Error":
                ctx.complete(f"Error: {ctx.message}")
            else:
                ctx.complete(f"Unhandled event: {ctx.type}")
        else:
            handler(ctx)

        if ctx._action is None:
            ctx.complete(f"Handler for {ctx.type} did not set an action")

        return ctx._action

    # -- HTTP server --

    def run(self, port=8080):
        """Start the agent HTTP server."""
        agent_ref = self

        class Handler(BaseHTTPRequestHandler):
            def do_POST(self):
                if self.path == "/handle":
                    length = int(self.headers.get("Content-Length", 0))
                    body = self.rfile.read(length)
                    event = json.loads(body)
                    action = agent_ref._handle(event)
                    response = json.dumps(action).encode()
                    self.send_response(200)
                    self.send_header("Content-Type", "application/json")
                    self.send_header("Content-Length", str(len(response)))
                    self.end_headers()
                    self.wfile.write(response)
                else:
                    self.send_response(404)
                    self.end_headers()

            def do_GET(self):
                if self.path == "/health":
                    self.send_response(200)
                    self.send_header("Content-Type", "text/plain")
                    self.end_headers()
                    self.wfile.write(b"ok")
                else:
                    self.send_response(404)
                    self.end_headers()

            def log_message(self, format, *args):
                pass

        server = HTTPServer(("0.0.0.0", port), Handler)
        print(f"Agent listening on :{port}")
        server.serve_forever()
