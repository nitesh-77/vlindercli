"""Code analyst agent — searches source code and ADRs for design intent.

Receives a user's question, finds relevant code paths and documentation
in the mounted source tree, and uses inference to explain whether behavior
is by-design, a known limitation, or a gap.
"""

import glob
import json
import os
import urllib.request
from http.server import HTTPServer, BaseHTTPRequestHandler

BRIDGE = os.environ.get("VLINDER_BRIDGE_URL", "")
INFER_MODEL = "default"
SOURCE_DIR = "/source"
MAX_CONTEXT_CHARS = 8000
MAX_FILES = 15


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
# Source searching
# =============================================================================

def find_adrs():
    """Find all ADR documents."""
    pattern = os.path.join(SOURCE_DIR, "docs", "adr", "*.md")
    files = glob.glob(pattern)
    files.sort()
    return files


def find_source_files():
    """Find Rust source files."""
    pattern = os.path.join(SOURCE_DIR, "src", "**", "*.rs")
    files = glob.glob(pattern, recursive=True)
    files.sort()
    return files


def search_files(file_list, query):
    """Search files for lines matching query terms. Returns relevant snippets."""
    terms = query.lower().split()
    results = []

    for filepath in file_list:
        try:
            with open(filepath, "r", errors="replace") as f:
                content = f.read()
                lower_content = content.lower()

                # Score by how many query terms appear
                score = sum(1 for term in terms if term in lower_content)
                if score > 0:
                    rel_path = os.path.relpath(filepath, SOURCE_DIR)
                    results.append({
                        "file": rel_path,
                        "score": score,
                        "content": content,
                    })
        except (OSError, IOError):
            continue

    # Sort by relevance score, highest first
    results.sort(key=lambda r: r["score"], reverse=True)
    return results[:MAX_FILES]


def extract_relevant_sections(content, query, max_chars=2000):
    """Extract the most relevant sections from a file around query matches."""
    terms = query.lower().split()
    lines = content.splitlines()
    relevant_lines = set()

    for i, line in enumerate(lines):
        lower_line = line.lower()
        if any(term in lower_line for term in terms):
            # Include surrounding context (5 lines before and after)
            for j in range(max(0, i - 5), min(len(lines), i + 6)):
                relevant_lines.add(j)

    if not relevant_lines:
        # No matches — return the first N lines as context
        return "\n".join(lines[:30])

    sorted_lines = sorted(relevant_lines)
    sections = []
    current_section = []
    prev = -2

    for idx in sorted_lines:
        if idx - prev > 1 and current_section:
            sections.append("\n".join(current_section))
            current_section = []
        current_section.append(f"{idx + 1}: {lines[idx]}")
        prev = idx

    if current_section:
        sections.append("\n".join(current_section))

    result = "\n...\n".join(sections)
    return result[:max_chars]


def find_relevant_adrs(query):
    """Find ADRs relevant to the query."""
    adrs = find_adrs()
    return search_files(adrs, query)


def find_relevant_source(query):
    """Find source files relevant to the query."""
    sources = find_source_files()
    return search_files(sources, query)


def format_findings(findings, query):
    """Format search results for the inference prompt."""
    if not findings:
        return "(no relevant files found)"

    parts = []
    total_chars = 0
    for f in findings:
        section = extract_relevant_sections(f["content"], query)
        entry = f"--- {f['file']} ---\n{section}"
        if total_chars + len(entry) > MAX_CONTEXT_CHARS:
            break
        parts.append(entry)
        total_chars += len(entry)

    return "\n\n".join(parts)


# =============================================================================
# Analysis
# =============================================================================

def analyze(user_query):
    """Analyze source code and documentation for a user query."""
    # Search ADRs first (design intent)
    adr_findings = find_relevant_adrs(user_query)

    # Then search source code (implementation)
    source_findings = find_relevant_source(user_query)

    # Build the prompt
    prompt_parts = [
        "You are a code analyst for Vlinder, a local-first AI agent orchestration platform.",
        "Your job is to explain the design intent behind the platform's behavior.",
        "You have access to Architecture Decision Records (ADRs) and source code.",
        "",
        f"User's question: {user_query}",
        "",
        "=== Relevant ADRs (design decisions) ===",
        format_findings(adr_findings, user_query),
        "",
        "=== Relevant source code ===",
        format_findings(source_findings, user_query),
        "",
        "Based on the documentation and code, provide a concise analysis:",
        "1. Whether this behavior is by-design (cite the relevant ADR)",
        "2. If it's a known limitation (documented but not yet implemented)",
        "3. If it appears to be a gap (not covered by any ADR or code)",
        "",
        "Be specific — cite ADR numbers and source file paths.",
    ]

    try:
        return infer("\n".join(prompt_parts))
    except Exception as e:
        return f"[code analysis error] {e}"


# =============================================================================
# HTTP server
# =============================================================================

class Handler(BaseHTTPRequestHandler):
    def do_GET(self):
        """Health check."""
        self.send_response(200)
        self.end_headers()

    def do_POST(self):
        """Analyze source code and docs for the given query."""
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
