"""Provider test agent — calls openrouter.vlinder.local and returns the response."""

from http.server import HTTPServer, BaseHTTPRequestHandler
from urllib.request import urlopen, Request
from urllib.error import URLError


class Handler(BaseHTTPRequestHandler):
    def do_GET(self):
        """Health check."""
        self.send_response(200)
        self.end_headers()

    def do_POST(self):
        """Call POST http://openrouter.vlinder.local/ and return the response."""
        try:
            length = int(self.headers.get("Content-Length", 0))
            data = self.rfile.read(length)
            req = Request("http://openrouter.vlinder.local/", data=data, method="POST")
            with urlopen(req, timeout=5) as resp:
                body = resp.read()
            self.send_response(200)
            self.send_header("Content-Length", str(len(body)))
            self.end_headers()
            self.wfile.write(body)
        except URLError as e:
            msg = f"provider unreachable: {e}".encode()
            self.send_response(502)
            self.send_header("Content-Length", str(len(msg)))
            self.end_headers()
            self.wfile.write(msg)

    def log_message(self, format, *args):
        """Suppress request logging."""
        pass


if __name__ == "__main__":
    HTTPServer(("0.0.0.0", 8080), Handler).serve_forever()
