"""Code analyst agent — pure retrieval from source code and ADRs.

Receives a user's question, finds relevant code paths and documentation
in the mounted source tree, and returns formatted excerpts. No LLM calls —
the support orchestrator handles all inference.
"""

import glob
import os
from http.server import HTTPServer, BaseHTTPRequestHandler

SOURCE_DIR = "/source"
MAX_CONTEXT_CHARS = 8000
FOUNDATION_BUDGET = 3000
MAX_FILES = 15

FOUNDATION_DOCS = ["VISION.md", "DOMAIN_MODEL.md", "README.md"]

STOP_WORDS = {
    "a", "an", "the", "is", "it", "in", "on", "at", "to", "of", "for",
    "and", "or", "but", "not", "with", "from", "by", "as", "be", "was",
    "were", "been", "are", "am", "do", "does", "did", "has", "have", "had",
    "will", "would", "could", "should", "may", "might", "can", "shall",
    "this", "that", "these", "those", "what", "which", "who", "whom",
    "how", "why", "when", "where", "i", "me", "my", "we", "us", "our",
    "you", "your", "he", "she", "they", "them", "its", "his", "her",
    "their", "about", "if", "so", "up", "out", "no", "yes", "all",
    "any", "some", "just", "only", "very", "also", "than", "then",
    "now", "here", "there", "each", "every", "into", "over", "after",
    "before", "between", "through", "during", "without",
    # Question fragments
    "does", "tell", "explain", "describe", "show", "give", "please",
    "know", "work", "works", "thing", "things", "something", "anything",
    "software", "program", "tool", "app", "application",
}


# =============================================================================
# Search term extraction
# =============================================================================

def extract_search_terms(query):
    """Extract meaningful search terms from a query, filtering stop words.

    Strips punctuation, lowercases, and removes common English stop words.
    Returns a list of terms suitable for content matching.
    """
    # Strip punctuation and split
    cleaned = ""
    for ch in query:
        if ch.isalnum() or ch in (" ", "-", "_"):
            cleaned += ch
        else:
            cleaned += " "

    terms = []
    for word in cleaned.lower().split():
        if word and word not in STOP_WORDS and len(word) > 1:
            terms.append(word)
    return terms


# =============================================================================
# Foundation documents
# =============================================================================

def read_foundation_docs():
    """Read VISION.md, DOMAIN_MODEL.md, and README.md from source root.

    These provide general project context and are always included in results.
    Returns formatted text within the foundation budget.
    """
    parts = []
    total = 0

    for name in FOUNDATION_DOCS:
        path = os.path.join(SOURCE_DIR, name)
        try:
            with open(path, "r", errors="replace") as f:
                content = f.read()
        except (OSError, IOError):
            continue

        budget_remaining = FOUNDATION_BUDGET - total
        if budget_remaining <= 0:
            break

        truncated = content[:budget_remaining]
        entry = f"--- {name} ---\n{truncated}"
        parts.append(entry)
        total += len(entry)

    return "\n\n".join(parts)


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


def search_files(file_list, terms):
    """Search files for lines matching search terms. Returns relevant snippets."""
    if not terms:
        return []

    results = []

    for filepath in file_list:
        try:
            with open(filepath, "r", errors="replace") as f:
                content = f.read()
                lower_content = content.lower()

                # Score by how many search terms appear
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


def extract_relevant_sections(content, terms, max_chars=2000):
    """Extract the most relevant sections from a file around term matches."""
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


def find_relevant_adrs(terms):
    """Find ADRs relevant to the search terms."""
    adrs = find_adrs()
    return search_files(adrs, terms)


def find_relevant_source(terms):
    """Find source files relevant to the search terms."""
    sources = find_source_files()
    return search_files(sources, terms)


def format_findings(findings, terms):
    """Format search results as readable excerpts."""
    if not findings:
        return "(no relevant files found)"

    parts = []
    total_chars = 0
    for f in findings:
        section = extract_relevant_sections(f["content"], terms)
        entry = f"--- {f['file']} ---\n{section}"
        if total_chars + len(entry) > MAX_CONTEXT_CHARS:
            break
        parts.append(entry)
        total_chars += len(entry)

    return "\n\n".join(parts)


# =============================================================================
# Analysis (pure retrieval — no LLM)
# =============================================================================

def analyze(user_query):
    """Retrieve relevant source code and documentation for a user query.

    Returns formatted excerpts directly — no LLM synthesis.
    Foundation docs (VISION.md, DOMAIN_MODEL.md, README.md) are always included.
    """
    terms = extract_search_terms(user_query)

    # Always include foundation docs for general context
    foundation = read_foundation_docs()

    # Search ADRs and source code with filtered terms
    adr_findings = find_relevant_adrs(terms)
    source_findings = find_relevant_source(terms)

    parts = [
        f"=== Query: {user_query} ===",
        "",
        "=== Foundation (VISION / DOMAIN MODEL / README) ===",
        foundation,
    ]

    adr_text = format_findings(adr_findings, terms)
    if adr_text != "(no relevant files found)":
        parts.append("")
        parts.append("=== Relevant ADRs (design decisions) ===")
        parts.append(adr_text)

    source_text = format_findings(source_findings, terms)
    if source_text != "(no relevant files found)":
        parts.append("")
        parts.append("=== Relevant source code ===")
        parts.append(source_text)

    if not terms:
        parts.append("")
        parts.append("(No specific search terms extracted — returning foundation docs only)")

    return "\n".join(parts)


# =============================================================================
# HTTP server
# =============================================================================

class Handler(BaseHTTPRequestHandler):
    def do_GET(self):
        """Health check."""
        self.send_response(200)
        self.end_headers()

    def do_POST(self):
        """Retrieve source code and docs for the given query."""
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
