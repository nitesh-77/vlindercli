"""OpenRouter test agent — uses the OpenAI SDK to talk to openrouter.vlinder.local."""

import sys
import time
from http.server import HTTPServer, BaseHTTPRequestHandler

from openai import OpenAI


client = OpenAI(
    base_url="http://openrouter.vlinder.local/v1",
    api_key="unused",  # auth is handled by the platform
)


def log(msg):
    print(f"[agent] {msg}", file=sys.stderr, flush=True)


class Handler(BaseHTTPRequestHandler):
    def do_GET(self):
        """Health check."""
        self.send_response(200)
        self.end_headers()

    def do_POST(self):
        """Read user message, call chat completions, return assistant reply."""
        length = int(self.headers.get("Content-Length", 0))
        user_message = self.rfile.read(length).decode()
        log(f"POST received, {length} bytes")

        try:
            log("calling chat.completions.create ...")
            t0 = time.monotonic()
            completion = client.chat.completions.create(
                model="anthropic/claude-sonnet-4",
                messages=[{"role": "user", "content": user_message}],
                max_tokens=2048,
            )
            elapsed = time.monotonic() - t0
            reply = completion.choices[0].message.content
            log(f"completion ok, {len(reply)} chars, {elapsed:.2f}s")
            self.send_response(200)
            self.send_header("Content-Length", str(len(reply)))
            self.end_headers()
            self.wfile.write(reply.encode())
        except Exception as e:
            elapsed = time.monotonic() - t0
            error_msg = str(e)
            log(f"completion error after {elapsed:.2f}s: {error_msg[:200]}")
            self.send_response(502)
            self.send_header("Content-Length", str(len(error_msg)))
            self.end_headers()
            self.wfile.write(error_msg.encode())

    def log_message(self, format, *args):
        """Suppress default request logging."""
        pass


if __name__ == "__main__":
    log("starting on port 8080")
    HTTPServer(("0.0.0.0", 8080), Handler).serve_forever()
