"""Support orchestrator — triage agent that delegates to specialists.

Receives user questions, triages them first, then either:
- Answers general questions directly (what is Vlinder, how does it work, etc.)
- Delegates to log-analyst and code-analyst for troubleshooting

Troubleshooting categories:
1. Misconfiguration — user's setup is wrong
2. Bug — observed behavior contradicts design intent
3. Feature request — user wants something not yet supported
4. Author guide — user wants to build something on the platform
5. Out of scope — issue is unrelated to Vlinder
"""

import json
import os
import urllib.request
from http.server import HTTPServer, BaseHTTPRequestHandler

BRIDGE = os.environ.get("VLINDER_BRIDGE_URL", "")
INFER_MODEL = "default"


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


def infer(prompt, max_tokens=1024):
    """Call the inference bridge endpoint."""
    result = bridge_call("/infer", {
        "model": INFER_MODEL,
        "prompt": prompt,
        "max_tokens": max_tokens,
    })
    return result.decode()


def delegate(agent, input_text):
    """Delegate work to another agent. Returns a handle for polling."""
    result = bridge_call("/delegate", {
        "op": "delegate",
        "agent": agent,
        "input": input_text,
    })
    data = json.loads(result)
    return data["handle"]


def wait_for(handle):
    """Wait for a delegated task to complete. Returns the output."""
    result = bridge_call("/wait", {
        "op": "wait",
        "handle": handle,
    })
    data = json.loads(result)
    return data["output"]


# =============================================================================
# Triage
# =============================================================================

TRIAGE_PROMPT = """You are a triage router for Vlinder, a local-first AI agent orchestration platform.

Vlinder lets developers run AI agents locally using containers (Podman), message queues (NATS),
and local LLMs (Ollama). Agents are composed into fleets for multi-agent workflows.

Classify the user's message into exactly one of these categories. Reply with ONLY the category name.

GENERAL — The user is asking a general question, greeting, or wants to know about Vlinder.
          Examples: "hello", "what does this do?", "how do agents work?", "what is a fleet?"
TROUBLESHOOT — The user has a problem, error, or unexpected behavior they need help with.
              Examples: "my agent crashes", "I get a 404 error", "deployment fails", "logs show errors"

User message: {query}

Category:"""

GENERAL_PROMPT = """You are the Vlinder support agent. Answer the user's question in 2-4 sentences.

Vlinder is a local-first AI agent orchestration platform written in Rust.
Agents are OCI containers. Fleets compose multiple agents. NATS handles messaging.
Ollama runs local LLMs. SQLite provides storage. CLI: vlinder daemon, vlinder support,
vlinder agent run, vlinder fleet run, vlinder model add.

Rules:
- Be concise. 2-4 sentences maximum.
- Do NOT generate fake follow-up questions or conversations.
- Do NOT invent CLI commands that were not listed above.
- Stop after your single response.

Question: {query}

Answer:"""


def triage(query):
    """Decide whether a query is general or needs troubleshooting."""
    prompt = TRIAGE_PROMPT.format(query=query)
    result = infer(prompt, max_tokens=16).strip().upper()
    if "TROUBLESHOOT" in result:
        return "troubleshoot"
    return "general"


# =============================================================================
# Orchestration
# =============================================================================

SYNTHESIS_PROMPT = """You are the Vlinder support orchestrator. You have received two specialist reports
about a user's problem: one from the log analyst (runtime behavior) and one from the
code analyst (design intent).

Synthesize these reports and classify the situation into exactly ONE category:

1. MISCONFIGURATION — The user's setup is wrong. Include the correct configuration.
2. BUG — Observed behavior contradicts design intent. Include: title, description, reproduction steps.
3. FEATURE_REQUEST — The user wants something not yet supported. Include: title, description, rationale.
4. AUTHOR_GUIDE — The user wants to build something on the platform. Include: manifest format, container contract, bridge endpoints.
5. OUT_OF_SCOPE — The issue is unrelated to Vlinder. Acknowledge and redirect.

Format your response as:

[CATEGORY_NAME]

<your structured response based on the category>

Be concise and actionable. Cite evidence from both reports."""


def handle_general(query):
    """Answer a general question directly."""
    prompt = GENERAL_PROMPT.format(query=query)
    return infer(prompt, max_tokens=256)


def handle_troubleshoot(query):
    """Delegate to specialists and synthesize their reports."""
    # Delegate to both specialists in parallel
    try:
        log_handle = delegate("log-analyst", query)
    except Exception as e:
        log_handle = None
        log_report = f"(log analysis unavailable: {e})"

    try:
        code_handle = delegate("code-analyst", query)
    except Exception as e:
        code_handle = None
        code_report = f"(code analysis unavailable: {e})"

    # Wait for results
    if log_handle:
        try:
            log_report = wait_for(log_handle)
        except Exception as e:
            log_report = f"(log analysis failed: {e})"

    if code_handle:
        try:
            code_report = wait_for(code_handle)
        except Exception as e:
            code_report = f"(code analysis failed: {e})"

    # Synthesize with inference
    prompt = f"""{SYNTHESIS_PROMPT}

=== User's Problem ===
{query}

=== Log Analyst Report (runtime behavior) ===
{log_report}

=== Code Analyst Report (design intent) ===
{code_report}

Based on both reports, classify and respond:"""

    try:
        return infer(prompt)
    except Exception as e:
        return (
            f"[SUPPORT ERROR] Classification failed: {e}\n\n"
            f"--- Log Analysis ---\n{log_report}\n\n"
            f"--- Code Analysis ---\n{code_report}"
        )


def handle_query(query):
    """Triage the query, then route to the appropriate handler."""
    category = triage(query)
    if category == "troubleshoot":
        return handle_troubleshoot(query)
    return handle_general(query)


# =============================================================================
# HTTP server
# =============================================================================

class Handler(BaseHTTPRequestHandler):
    def do_GET(self):
        """Health check."""
        self.send_response(200)
        self.end_headers()

    def do_POST(self):
        """Handle a support query."""
        length = int(self.headers.get("Content-Length", 0))
        body = self.rfile.read(length).decode()

        # Strip fleet context prefix if present (injected by REPL)
        if "\n\n" in body:
            parts = body.split("\n\n", 1)
            if parts[0].startswith("Fleet:"):
                body = parts[1]

        result = handle_query(body)
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
