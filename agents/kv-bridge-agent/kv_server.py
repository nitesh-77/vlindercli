"""KV bridge test agent — stores input in KV, reads it back via the provider hostname."""

import json
import urllib.request
from http.server import HTTPServer, BaseHTTPRequestHandler


def kv_call(path, body):
    """POST to the sqlite-kv provider and return the response body."""
    url = f"http://sqlite-kv.vlinder.local{path}"
    data = json.dumps(body).encode()
    req = urllib.request.Request(url, data=data, method="POST")
    req.add_header("Content-Type", "application/json")
    with urllib.request.urlopen(req) as resp:
        return resp.read()


class Handler(BaseHTTPRequestHandler):
    def do_GET(self):
        """Health check."""
        self.send_response(200)
        self.end_headers()

    def do_POST(self):
        """Store input in KV, read it back, return the result."""
        length = int(self.headers.get("Content-Length", 0))
        body = self.rfile.read(length)

        # Store in KV — no base64, plain string content
        content = body.decode("utf-8", errors="replace")
        kv_call("/put", {"path": "/test.txt", "content": content})

        # Read back from KV
        result = kv_call("/get", {"path": "/test.txt"})

        self.send_response(200)
        self.send_header("Content-Length", str(len(result)))
        self.end_headers()
        self.wfile.write(result)

    def log_message(self, format, *args):
        """Suppress request logging."""
        pass


if __name__ == "__main__":
    HTTPServer(("0.0.0.0", 8080), Handler).serve_forever()
