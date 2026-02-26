"""S3 mount test agent — reads files from a mounted S3 bucket.

Supports two commands:
  ls [path]       List directory contents
  cat <path>      Print file contents

All paths are resolved under /knowledge/ (the S3 mount point).
"""

import os
from http.server import HTTPServer, BaseHTTPRequestHandler

MOUNT_ROOT = "/knowledge"


def safe_path(user_path):
    """Resolve user_path under MOUNT_ROOT. Returns None if it escapes."""
    if not user_path:
        return MOUNT_ROOT
    joined = os.path.normpath(os.path.join(MOUNT_ROOT, user_path))
    if not joined.startswith(MOUNT_ROOT):
        return None
    return joined


def handle_ls(args):
    path = safe_path(args[0] if args else "")
    if path is None:
        return "error: invalid path"
    if not os.path.isdir(path):
        return f"error: not a directory: {path}"
    try:
        entries = sorted(os.listdir(path))
        return "\n".join(entries) if entries else "(empty)"
    except OSError as e:
        return f"error: {e}"


def handle_cat(args):
    if not args:
        return "usage: cat <path>"
    path = safe_path(args[0])
    if path is None:
        return "error: invalid path"
    if not os.path.isfile(path):
        return f"error: not a file: {path}"
    try:
        with open(path) as f:
            return f.read()
    except OSError as e:
        return f"error: {e}"


def parse_payload(raw):
    """Extract the latest user message from conversation history."""
    lines = raw.strip().splitlines()
    for line in reversed(lines):
        if line.startswith("User: "):
            return line[len("User: "):]
    return raw.strip()


def dispatch(user_input):
    parts = user_input.strip().split()
    if not parts:
        return "commands: ls [path], cat <path>"

    cmd = parts[0].lower()
    args = parts[1:]

    if cmd == "ls":
        return handle_ls(args)
    elif cmd == "cat":
        return handle_cat(args)
    else:
        return "commands: ls [path], cat <path>"


class Handler(BaseHTTPRequestHandler):
    def do_GET(self):
        self.send_response(200)
        self.end_headers()

    def do_POST(self):
        length = int(self.headers.get("Content-Length", 0))
        payload = self.rfile.read(length).decode()
        user_input = parse_payload(payload)
        response = dispatch(user_input).encode()
        self.send_response(200)
        self.send_header("Content-Length", str(len(response)))
        self.end_headers()
        self.wfile.write(response)

    def log_message(self, fmt, *args):
        pass


if __name__ == "__main__":
    HTTPServer(("0.0.0.0", 8080), Handler).serve_forever()
