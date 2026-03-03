"""sqlite-vec-test agent — exercises sqlite-vec vector storage via provider hostname.

Subcommands (first word of user input):
    store <key> <metadata...>   Store a deterministic 768-dim vector for <key>.
    search <key> [limit]        Search using the vector for <key>. Default limit 5.
    delete <key>                Delete embedding by key.
    help                        Show usage.

Vectors are derived deterministically from the key string so that
store + search with the same key produces an exact match.
"""

import hashlib
import json
import math
import urllib.request
from http.server import HTTPServer, BaseHTTPRequestHandler

VEC_BASE = "http://sqlite-vec.vlinder.local:3544"
DIMS = 768


def key_to_vector(key):
    """Deterministic 768-dim vector from a string key.

    Uses SHA-256 to seed a simple expansion: hash the key, then for each
    dimension compute sin/cos of the hash bytes. Produces a repeatable
    vector that is different for different keys.
    """
    h = hashlib.sha256(key.encode()).digest()
    vec = []
    for i in range(DIMS):
        byte_idx = i % len(h)
        val = h[byte_idx] / 255.0
        if i % 2 == 0:
            vec.append(math.sin(val * (i + 1)))
        else:
            vec.append(math.cos(val * (i + 1)))
    return vec


def post_json(url, payload):
    """POST JSON to a URL and return (status, body_text)."""
    data = json.dumps(payload).encode()
    req = urllib.request.Request(
        url,
        data=data,
        headers={"Content-Type": "application/json"},
        method="POST",
    )
    try:
        with urllib.request.urlopen(req, timeout=60) as resp:
            return resp.status, resp.read().decode()
    except urllib.error.HTTPError as e:
        return e.code, e.read().decode()
    except Exception as e:
        return 0, str(e)


def parse_payload(raw):
    """Extract the latest user message from a harness payload.

    The harness builds conversation history as:
        User: first message
        Agent: first response
        User: second message
    The last 'User: ' line is the current input.
    """
    lines = raw.strip().splitlines()
    if lines and lines[-1].startswith("User: "):
        return lines[-1][len("User: "):]
    return raw.strip()


def handle_store(args):
    if len(args) < 2:
        return "usage: store <key> <metadata...>"
    key = args[0]
    metadata = " ".join(args[1:])
    vector = key_to_vector(key)
    status, body = post_json(f"{VEC_BASE}/store", {
        "key": key,
        "vector": vector,
        "metadata": metadata,
    })
    if status == 200:
        return f"stored key={key} metadata={metadata!r}"
    return f"error ({status}): {body}"


def handle_search(args):
    if len(args) < 1:
        return "usage: search <key> [limit]"
    key = args[0]
    limit = int(args[1]) if len(args) > 1 else 5
    vector = key_to_vector(key)
    status, body = post_json(f"{VEC_BASE}/search", {
        "vector": vector,
        "limit": limit,
    })
    if status != 200:
        return f"error ({status}): {body}"
    try:
        results = json.loads(body)
    except json.JSONDecodeError:
        return f"bad response: {body}"
    if not results:
        return "no results"
    lines = []
    for r in results:
        lines.append(f"  {r['key']}  dist={r['distance']:.6f}  meta={r['metadata']!r}")
    return f"found {len(results)} result(s):\n" + "\n".join(lines)


def handle_delete(args):
    if len(args) < 1:
        return "usage: delete <key>"
    key = args[0]
    status, body = post_json(f"{VEC_BASE}/delete", {"key": key})
    if status == 200:
        return f"delete key={key}: {body}"
    return f"error ({status}): {body}"


USAGE = """sqlite-vec-test agent commands:
  store <key> <metadata...>   Store embedding for key
  search <key> [limit]        Search by key's vector
  delete <key>                Delete embedding
  help                        Show this message"""


def dispatch(text):
    parts = text.strip().split()
    if not parts:
        return USAGE
    cmd = parts[0].lower()
    args = parts[1:]
    if cmd == "store":
        return handle_store(args)
    elif cmd == "search":
        return handle_search(args)
    elif cmd == "delete":
        return handle_delete(args)
    elif cmd == "help":
        return USAGE
    else:
        return f"unknown command: {cmd}\n\n{USAGE}"


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
    print("sqlite-vec-test agent listening on :8080", flush=True)
    HTTPServer(("0.0.0.0", 8080), Handler).serve_forever()
