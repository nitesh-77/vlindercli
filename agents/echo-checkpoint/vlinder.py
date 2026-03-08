"""Vlinder Agent SDK — durable execution mode (ADR 111).

Single endpoint: POST /invoke. All requests are JSON with a handler name:

  {"handler": "on_invoke", "input": "user text"}
  {"handler": "handle_result", "result": {...}}

Action format (Agent -> Sidecar):

  {"action": "call", "url": "...", "json": {...}, "then": "handle_result"}
  {"action": "complete", "payload": "..."}
"""

import json
from http.server import HTTPServer, BaseHTTPRequestHandler


class CallResult:
    """Wraps the HTTP response from a ctx.call()."""

    def __init__(self, data):
        self._data = data

    def json(self):
        return self._data


class Context:
    """Execution context passed to every handler.

    Attributes:
        input:  User input text (set on invoke, None on callback).
        result: CallResult from the previous ctx.call() (None on invoke).
    """

    def __init__(self, input_text=None, result=None):
        self.input = input_text
        self.result = result
        self._action = None

    def call(self, url, json=None, then=None):
        """Hand off an HTTP call to the platform."""
        self._action = {
            "action": "call",
            "url": url,
            "json": json,
            "then": then.__name__,
        }

    def complete(self, payload):
        """Return the final result."""
        self._action = {
            "action": "complete",
            "payload": str(payload),
        }


class Agent:
    """Durable agent framework (ADR 111).

    Decorators:
        @agent.on_invoke -- entry point, registered as "on_invoke".
        @agent.handler   -- named checkpoint handler.
    """

    def __init__(self):
        self._handlers = {}

    def on_invoke(self, fn):
        self._handlers["on_invoke"] = fn
        return fn

    def handler(self, fn):
        self._handlers[fn.__name__] = fn
        return fn

    def _dispatch(self, event):
        """Dispatch to the named handler."""
        name = event.get("handler")
        if not name:
            return {"action": "complete", "payload": "missing 'handler' field"}

        handler = self._handlers.get(name)
        if not handler:
            return {"action": "complete", "payload": f"unknown handler '{name}'"}

        if "result" in event:
            ctx = Context(result=CallResult(event["result"]))
        elif "input" in event:
            ctx = Context(input_text=event["input"])
        else:
            ctx = Context()

        handler(ctx)

        if ctx._action is None:
            ctx.complete("handler did not set an action")
        return ctx._action

    def run(self, port=8080):
        agent_ref = self

        class Handler(BaseHTTPRequestHandler):
            def do_POST(self):
                length = int(self.headers.get("Content-Length", 0))
                body = self.rfile.read(length)

                if self.path == "/invoke":
                    try:
                        event = json.loads(body)
                        if not isinstance(event, dict) or "handler" not in event:
                            event = {"handler": "on_invoke", "input": body.decode()}
                    except json.JSONDecodeError:
                        event = {"handler": "on_invoke", "input": body.decode()}
                    action = agent_ref._dispatch(event)
                    response = json.dumps(action).encode()
                    self.send_response(200)
                    self.send_header("Content-Type", "application/json")
                    self.send_header("X-Vlinder-Mode", "durable")
                    self.send_header("Content-Length", str(len(response)))
                    self.end_headers()
                    self.wfile.write(response)
                else:
                    self.send_response(404)
                    self.end_headers()

            def do_GET(self):
                if self.path == "/health":
                    self.send_response(200)
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
