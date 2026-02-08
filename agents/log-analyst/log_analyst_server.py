"""Log analyst agent — searches vlinder logs for error patterns and root causes.

Receives a user's error description, searches the mounted log directory for
relevant entries, correlates timestamps, and uses inference to interpret
log sequences and extract root causes.
"""

import glob
import json
import os
import urllib.request
from http.server import HTTPServer, BaseHTTPRequestHandler

BRIDGE = os.environ.get("VLINDER_BRIDGE_URL", "")
INFER_MODEL = "default"
LOGS_DIR = "/logs"
MAX_LOG_LINES = 200
MAX_FILES = 10


# =============================================================================
# Bridge helpers
# =============================================================================

def bridge_call(path, body):
    """POST to a bridge endpoint and return the response body."""
    url = f"{BRIDGE}{path}"
    data = json.dumps(body).encode()
    req = urllib.request.Request(url, data=data, method="POST")
    req.add_header("Content-Type", "application/json")
    with urllib.request.urlopen(req) as resp:
        return resp.read()


def infer(prompt, max_tokens=512):
    """Call the inference bridge endpoint."""
    result = bridge_call("/infer", {
        "model": INFER_MODEL,
        "prompt": prompt,
        "max_tokens": max_tokens,
    })
    return result.decode()


# =============================================================================
# Log searching
# =============================================================================

def find_log_files():
    """Find all log files in the mounted logs directory."""
    patterns = ["*.log", "*.txt", "*.jsonl"]
    files = []
    for pattern in patterns:
        files.extend(glob.glob(os.path.join(LOGS_DIR, "**", pattern), recursive=True))
    # Sort by modification time, newest first
    files.sort(key=lambda f: os.path.getmtime(f), reverse=True)
    return files[:MAX_FILES]


def search_logs(query):
    """Search log files for lines matching the query terms."""
    terms = query.lower().split()
    matches = []
    files = find_log_files()

    for filepath in files:
        try:
            with open(filepath, "r", errors="replace") as f:
                for i, line in enumerate(f, 1):
                    lower_line = line.lower()
                    if any(term in lower_line for term in terms):
                        matches.append({
                            "file": os.path.relpath(filepath, LOGS_DIR),
                            "line": i,
                            "content": line.strip()[:500],
                        })
        except (OSError, IOError):
            continue

    return matches[:MAX_LOG_LINES]


def read_recent_errors():
    """Read the most recent error-level log entries."""
    error_terms = ["error", "err", "panic", "fatal", "failed", "exception"]
    matches = []
    files = find_log_files()

    for filepath in files:
        try:
            with open(filepath, "r", errors="replace") as f:
                lines = f.readlines()
                # Check last 500 lines of each file
                for i, line in enumerate(lines[-500:], max(1, len(lines) - 500)):
                    lower_line = line.lower()
                    if any(term in lower_line for term in error_terms):
                        matches.append({
                            "file": os.path.relpath(filepath, LOGS_DIR),
                            "line": i,
                            "content": line.strip()[:500],
                        })
        except (OSError, IOError):
            continue

    return matches[:MAX_LOG_LINES]


def format_log_entries(entries):
    """Format log entries for inclusion in the inference prompt."""
    if not entries:
        return "(no matching log entries found)"
    lines = []
    for entry in entries:
        lines.append(f"[{entry['file']}:{entry['line']}] {entry['content']}")
    return "\n".join(lines)


# =============================================================================
# Analysis
# =============================================================================

def analyze(user_query):
    """Analyze logs in response to a user query."""
    # Search for query-specific entries
    query_matches = search_logs(user_query)

    # Also get recent errors for context
    recent_errors = read_recent_errors()

    # Build the prompt
    prompt_parts = [
        "You are a log analyst for Vlinder, a local-first AI agent orchestration platform.",
        "Your job is to analyze runtime logs and identify what actually happened.",
        "",
        f"User's question: {user_query}",
        "",
        "=== Log entries matching the query ===",
        format_log_entries(query_matches),
        "",
        "=== Recent errors ===",
        format_log_entries(recent_errors),
        "",
        "Based on these logs, provide a concise analysis:",
        "1. What happened at runtime (the sequence of events)",
        "2. Any error patterns or correlations you notice",
        "3. The likely root cause based on the log evidence",
        "",
        "Be specific — cite log file names and line numbers. If the logs don't contain",
        "relevant information, say so clearly.",
    ]

    try:
        return infer("\n".join(prompt_parts))
    except Exception as e:
        return f"[log analysis error] {e}"


# =============================================================================
# HTTP server
# =============================================================================

class Handler(BaseHTTPRequestHandler):
    def do_GET(self):
        """Health check."""
        self.send_response(200)
        self.end_headers()

    def do_POST(self):
        """Analyze logs for the given query."""
        length = int(self.headers.get("Content-Length", 0))
        body = self.rfile.read(length).decode()

        result = analyze(body)
        response = result.encode()

        self.send_response(200)
        self.send_header("Content-Length", str(len(response)))
        self.end_headers()
        self.wfile.write(response)

    def log_message(self, format, *args):
        """Suppress request logging."""
        pass


if __name__ == "__main__":
    HTTPServer(("0.0.0.0", 8080), Handler).serve_forever()
