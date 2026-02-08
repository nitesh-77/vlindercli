"""Log analyst agent — searches runtime state for error patterns and root causes.

Receives a user's error description. Has read-only access to the full
~/.vlinder directory: logs, conversation store (timeline SHAs), and
config. Joins these three perspectives to correlate what happened (logs),
what was tracked (timeline), and how the system is configured (config.toml).
"""

import glob
import json
import os
import subprocess
import urllib.request
from http.server import HTTPServer, BaseHTTPRequestHandler

BRIDGE = os.environ.get("VLINDER_BRIDGE_URL", "")
INFER_MODEL = "default"
VLINDER_DIR = "/vlinder"
LOGS_DIR = os.path.join(VLINDER_DIR, "logs")
CONFIG_PATH = os.path.join(VLINDER_DIR, "config.toml")
CONVERSATIONS_DIR = os.path.join(VLINDER_DIR, "conversations")
MAX_LOG_RECORDS = 200
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
# JSONL log parsing
# =============================================================================

def find_log_files():
    """Find JSONL log files in the mounted logs directory."""
    files = glob.glob(os.path.join(LOGS_DIR, "*.jsonl"))
    files.sort(key=lambda f: os.path.getmtime(f), reverse=True)
    return files[:MAX_FILES]


def parse_jsonl_logs():
    """Parse all JSONL log files into structured records."""
    records = []
    for filepath in find_log_files():
        try:
            with open(filepath, "r", errors="replace") as f:
                for line in f:
                    line = line.strip()
                    if not line:
                        continue
                    try:
                        record = json.loads(line)
                        record["_file"] = os.path.basename(filepath)
                        records.append(record)
                    except json.JSONDecodeError:
                        continue
        except (OSError, IOError):
            continue
    return records


def search_logs(query):
    """Search structured log records by field values or text.

    Supports field:value syntax (e.g. 'level:ERROR', 'sha:abc123',
    'agent:echo-agent', 'event:dispatch.started') and plain text search.
    """
    records = parse_jsonl_logs()
    matches = []

    # Parse query into field filters and text terms
    field_filters = {}
    text_terms = []
    for token in query.split():
        if ":" in token:
            key, value = token.split(":", 1)
            field_filters[key] = value.lower()
        else:
            text_terms.append(token.lower())

    for record in records:
        # Check field filters
        field_match = True
        for key, value in field_filters.items():
            record_value = str(record.get(key, "")).lower()
            # Also check inside span fields
            if not record_value or value not in record_value:
                span_value = ""
                for span in record.get("spans", []):
                    span_value = str(span.get(key, "")).lower()
                    if value in span_value:
                        break
                if value not in span_value:
                    field_match = False
                    break

        if not field_match:
            continue

        # Check text terms against the serialized record
        if text_terms:
            record_str = json.dumps(record).lower()
            if not all(term in record_str for term in text_terms):
                continue

        matches.append(record)

    return matches[:MAX_LOG_RECORDS]


def read_recent_errors():
    """Read log records at ERROR level or above."""
    records = parse_jsonl_logs()
    return [
        r for r in records
        if r.get("level", "").upper() in ("ERROR", "WARN")
    ][:MAX_LOG_RECORDS]


def format_log_records(records):
    """Format structured log records for inclusion in the inference prompt."""
    if not records:
        return "(no matching log records found)"
    lines = []
    for r in records:
        timestamp = r.get("timestamp", "?")
        level = r.get("level", "?")
        target = r.get("target", "")
        message = r.get("fields", {}).get("message", "")
        event = r.get("fields", {}).get("event", "")
        sha = ""
        agent = ""
        # Extract span fields
        for span in r.get("spans", []):
            sha = sha or span.get("sha", "")
            agent = agent or span.get("agent", "")

        parts = [f"[{timestamp}]", level]
        if event:
            parts.append(f"event={event}")
        if sha:
            parts.append(f"sha={sha[:8]}")
        if agent:
            parts.append(f"agent={agent}")
        if target:
            parts.append(f"({target})")
        parts.append(message)
        lines.append(" ".join(parts))
    return "\n".join(lines)


# =============================================================================
# Config and timeline
# =============================================================================

def read_config():
    """Read config.toml from the mounted vlinder directory."""
    try:
        with open(CONFIG_PATH, "r") as f:
            return f.read()
    except (OSError, IOError):
        return "(config.toml not found)"


def read_recent_timeline():
    """Read recent conversation timeline entries from the git store."""
    if not os.path.isdir(CONVERSATIONS_DIR):
        return "(no conversations directory)"

    lines = []
    for session_dir in sorted(glob.glob(os.path.join(CONVERSATIONS_DIR, "*")), reverse=True)[:3]:
        if not os.path.isdir(session_dir):
            continue
        try:
            result = subprocess.run(
                ["git", "log", "--oneline", "-10"],
                cwd=session_dir, capture_output=True, text=True, timeout=5,
            )
            if result.returncode == 0 and result.stdout.strip():
                name = os.path.basename(session_dir)
                lines.append(f"--- {name} ---")
                lines.append(result.stdout.strip())
        except (OSError, subprocess.TimeoutExpired):
            continue

    return "\n".join(lines) if lines else "(no timeline entries)"


# =============================================================================
# Analysis
# =============================================================================

def analyze(user_query):
    """Analyze logs, config, and timeline in response to a user query."""
    query_matches = search_logs(user_query)
    recent_errors = read_recent_errors()
    config = read_config()
    timeline = read_recent_timeline()

    prompt_parts = [
        "You are a runtime state analyst for Vlinder, a local-first AI agent orchestration platform.",
        "You have access to structured JSONL logs, system configuration, and conversation timeline.",
        "Your job is to correlate these data sources and identify what actually happened.",
        "",
        f"User's question: {user_query}",
        "",
        "=== Structured log entries matching the query ===",
        format_log_records(query_matches),
        "",
        "=== Recent errors/warnings ===",
        format_log_records(recent_errors),
        "",
        "=== System configuration (config.toml) ===",
        config[:2000],
        "",
        "=== Recent conversation timeline ===",
        timeline[:1000],
        "",
        "Based on all available data, provide a concise analysis:",
        "1. What happened at runtime (sequence of events, correlated by SHA)",
        "2. Any error patterns or correlations across logs, config, and timeline",
        "3. The likely root cause based on the evidence",
        "",
        "Be specific — cite event names, SHAs, agent names, and timestamps.",
        "If the data doesn't contain relevant information, say so clearly.",
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
