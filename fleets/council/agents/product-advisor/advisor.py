"""Council advisor — reads /persona.txt, calls OpenRouter for each query.

Shared code across all advisor agents. The persona file is baked into each
container image at build time, giving each advisor its distinct expertise.
"""

import json
import sys
import urllib.request
from http.server import HTTPServer, BaseHTTPRequestHandler

OPENROUTER_URL = "http://openrouter.vlinder.local/v1/chat/completions"
MODEL = "google/gemini-2.0-flash-001"


def log(msg):
    print(f"[advisor] {msg}", file=sys.stderr, flush=True)


def load_persona():
    """Read the persona system prompt from /persona.txt."""
    try:
        with open("/persona.txt") as f:
            return f.read().strip()
    except FileNotFoundError:
        log("WARNING: /persona.txt not found, using empty persona")
        return "You are a helpful advisor."


PERSONA = load_persona()


def chat(user_message, max_tokens=1024):
    """Call OpenRouter chat completions with the persona as system prompt."""
    payload = json.dumps({
        "model": MODEL,
        "messages": [
            {"role": "system", "content": PERSONA},
            {"role": "user", "content": user_message},
        ],
        "max_tokens": max_tokens,
    }).encode()

    req = urllib.request.Request(OPENROUTER_URL, data=payload, method="POST")
    req.add_header("Content-Type", "application/json")
    req.add_header("Host", "openrouter.vlinder.local")

    with urllib.request.urlopen(req) as resp:
        result = json.loads(resp.read())

    return result["choices"][0]["message"]["content"]


def parse_payload(raw):
    """Extract user input, stripping fleet context prefix if present."""
    if "\n\n" in raw:
        parts = raw.split("\n\n", 1)
        if parts[0].startswith("Fleet:"):
            return parts[1]
    return raw.strip()


class Handler(BaseHTTPRequestHandler):
    def do_GET(self):
        self.send_response(200)
        self.end_headers()

    def do_POST(self):
        length = int(self.headers.get("Content-Length", 0))
        body = self.rfile.read(length).decode()
        user_input = parse_payload(body)
        log(f"received {len(user_input)} chars")

        try:
            reply = chat(user_input)
            log(f"reply: {len(reply)} chars")
        except Exception as e:
            log(f"error: {e}")
            reply = f"(advisor error: {e})"

        data = reply.encode()
        self.send_response(200)
        self.send_header("Content-Length", str(len(data)))
        self.end_headers()
        self.wfile.write(data)

    def log_message(self, fmt, *args):
        pass


if __name__ == "__main__":
    log(f"starting on :8080, persona={len(PERSONA)} chars")
    HTTPServer(("0.0.0.0", 8080), Handler).serve_forever()
