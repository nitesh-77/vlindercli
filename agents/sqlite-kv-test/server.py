"""sqlite-kv-test agent — exercises sqlite-kv storage via provider hostname.

Subcommands (first word of user input):
    put <path> <content...>   Store content at path.
    get <path>                Read content at path.
    list [prefix]             List files, optionally filtered by prefix.
    delete <path>             Delete file at path.
    help                      Show usage.
"""

import json
import urllib.request
from http.server import HTTPServer, BaseHTTPRequestHandler

KV_BASE = "http://sqlite-kv.vlinder.local"


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


def handle_put(args):
    if len(args) < 2:
        return "usage: put <path> <content...>"
    path = args[0]
    content = " ".join(args[1:])
    status, body = post_json(f"{KV_BASE}/put", {
        "path": path,
        "content": content,
    })
    if status == 200:
        return f"stored path={path} content={content!r}"
    return f"error ({status}): {body}"


def handle_get(args):
    if len(args) < 1:
        return "usage: get <path>"
    path = args[0]
    status, body = post_json(f"{KV_BASE}/get", {"path": path})
    if status != 200:
        return f"error ({status}): {body}"
    if not body:
        return f"path={path}: (empty)"
    return f"path={path}: {body}"


def handle_list(args):
    prefix = args[0] if args else "/"
    status, body = post_json(f"{KV_BASE}/list", {"path": prefix})
    if status != 200:
        return f"error ({status}): {body}"
    try:
        files = json.loads(body)
    except json.JSONDecodeError:
        return f"bad response: {body}"
    if not files:
        return "no files"
    lines = [f"  {f}" for f in files]
    return f"found {len(files)} file(s):\n" + "\n".join(lines)


def handle_delete(args):
    if len(args) < 1:
        return "usage: delete <path>"
    path = args[0]
    status, body = post_json(f"{KV_BASE}/delete", {"path": path})
    if status == 200:
        return f"deleted path={path}"
    return f"error ({status}): {body}"


USAGE = """sqlite-kv-test agent commands:
  put <path> <content...>   Store content at path
  get <path>                Read content at path
  list [prefix]             List files under prefix
  delete <path>             Delete file at path
  help                      Show this message"""


def dispatch(text):
    parts = text.strip().split()
    if not parts:
        return USAGE
    cmd = parts[0].lower()
    args = parts[1:]
    if cmd == "put":
        return handle_put(args)
    elif cmd == "get":
        return handle_get(args)
    elif cmd == "list":
        return handle_list(args)
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
    print("sqlite-kv-test agent listening on :8080", flush=True)
    HTTPServer(("0.0.0.0", 8080), Handler).serve_forever()
