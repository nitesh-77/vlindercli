"""Support orchestrator — grounded triage agent that delegates to specialists.

Receives user questions, detects greetings, triages the rest, then either:
- Answers questions grounded in retrieved documentation (code-analyst)
- Troubleshoots problems using both specialists' retrieved data

This is the ONLY agent in the support fleet that calls the LLM.
Specialists (code-analyst, log-analyst) are pure retrieval — no LLM.
"""

import json
import os
import urllib.request
from http.server import HTTPServer, BaseHTTPRequestHandler

BRIDGE = os.environ.get("VLINDER_BRIDGE_URL", "")
INFER_MODEL = "default"
MAX_REPORT_CHARS = 3000

GREETING_WORDS = {"hello", "hi", "hey", "greetings", "howdy", "sup", "yo"}

GREETING_RESPONSE = """Welcome to Vlinder support! I can help you with:

- **Understanding Vlinder** — what it does, how agents/fleets/queues work
- **Troubleshooting** — errors, crashes, configuration issues
- **Building agents** — manifest format, container contract, bridge API

What would you like to know?"""


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


def infer(prompt, max_tokens=256):
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
# Greeting detection
# =============================================================================

def is_greeting(query):
    """Detect greetings by keyword match on short messages.

    Only triggers on messages with 3 or fewer words to avoid false positives
    on queries like "hi, my agent crashes on startup".
    """
    words = query.strip().lower().split()
    if len(words) > 3:
        return False
    return any(w.strip(".,!?") in GREETING_WORDS for w in words)


# =============================================================================
# Triage (few-shot, minimal tokens)
# =============================================================================

TRIAGE_PROMPT = """Classify the user message as QUESTION or TROUBLESHOOT. Reply with ONE word only.

User: what is a fleet?
QUESTION

User: how do agents communicate?
QUESTION

User: what commands are available?
QUESTION

User: my agent crashes on startup
TROUBLESHOOT

User: I get a connection refused error
TROUBLESHOOT

User: logs show timeout errors
TROUBLESHOOT

User: {query}
"""


def triage(query):
    """Classify a query as question or troubleshoot using few-shot prompt."""
    prompt = TRIAGE_PROMPT.format(query=query)
    try:
        result = infer(prompt, max_tokens=8).strip().upper()
    except Exception:
        return "question"

    if "TROUBLESHOOT" in result:
        return "troubleshoot"
    return "question"


# =============================================================================
# Question handler (grounded in retrieved docs)
# =============================================================================

QUESTION_PROMPT = """You are the Vlinder support agent. Answer the user's question using ONLY the documentation below. If the documentation doesn't cover the topic, say so honestly.

Be concise: 2-5 sentences. Do NOT invent features, commands, or APIs not mentioned in the docs.

Example Q&A:

Q: What is Vlinder?
A: Vlinder is a local-first AI agent orchestration platform. It runs AI agents as OCI containers, coordinates them through message queues (NATS or in-memory), and uses local LLMs via Ollama for inference. Everything runs on your machine — no cloud services required.

Q: What is a fleet?
A: A fleet is a group of agents that work together on a task. Fleets are defined in TOML manifests that specify which agents to include and how they connect. The support fleet, for example, composes a triage agent with specialist analysts.

Q: How do I create an agent?
A: Define an agent.toml manifest with the agent's name, runtime, and container image. The agent runs as an HTTP server inside the container — it receives work via POST requests and returns results. Mount any files the agent needs as read-only volumes.

=== Retrieved Documentation ===
{docs}

Q: {query}
A:"""

NO_DOCS_RESPONSE = """I don't have enough context to answer that question well. Here are some topics I can help with:

- **Architecture** — agents, fleets, queues, messaging
- **Configuration** — config.toml, agent manifests, model setup
- **Commands** — vlinder daemon, vlinder support, vlinder agent run, vlinder fleet run
- **Troubleshooting** — errors, crashes, log analysis

Try asking about one of these specifically!"""


def handle_question(query):
    """Answer a question grounded in code-analyst's retrieved documentation."""
    # Delegate to code-analyst for doc retrieval
    try:
        code_handle = delegate("code-analyst", query)
        docs = wait_for(code_handle)
    except Exception:
        docs = ""

    if not docs or len(docs.strip()) < 50:
        return NO_DOCS_RESPONSE

    prompt = QUESTION_PROMPT.format(docs=docs[:6000], query=query)
    try:
        return infer(prompt, max_tokens=256)
    except Exception:
        # LLM failed — return raw retrieval as fallback
        return f"Here's what I found in the documentation:\n\n{docs[:3000]}"


# =============================================================================
# Troubleshoot handler (grounded in both specialists)
# =============================================================================

TROUBLESHOOT_PROMPT = """You are the Vlinder support agent. Diagnose the user's problem using the specialist reports below.

The log analyst report shows runtime data (log entries, errors, config). The code analyst report shows design documentation (ADRs, source code). Use both to identify the issue.

Provide a concise diagnosis in 4-6 sentences:
1. What the evidence shows
2. The likely cause
3. What to try next

Example:

User problem: "agent not receiving messages"
Diagnosis: The logs show the agent container started successfully but no messages were dispatched to it. The code analyst found ADR 044 which documents that agents must declare their queue subscriptions in agent.toml. Check that your agent's manifest includes the correct queue bindings — without them, the dispatcher won't route messages to your agent. Run `vlinder agent inspect <name>` to verify the resolved configuration.

=== User's Problem ===
{query}

=== Log Analyst Report (runtime data) ===
{log_report}

=== Code Analyst Report (documentation) ===
{code_report}

Diagnosis:"""

SPECIALISTS_UNAVAILABLE = """I'm having trouble reaching the specialist agents right now. Please make sure the support fleet is fully running:

1. Check that all containers are up: `podman ps`
2. Restart the fleet: `vlinder fleet run support`
3. Try your question again

If the problem persists, check the logs at `~/.vlinder/logs/` for errors."""


def handle_troubleshoot(query):
    """Delegate to both specialists and synthesize their reports."""
    log_report = None
    code_report = None

    # Delegate to both specialists in parallel
    log_handle = None
    code_handle = None

    try:
        log_handle = delegate("log-analyst", query)
    except Exception as e:
        log_report = f"(log analysis unavailable: {e})"

    try:
        code_handle = delegate("code-analyst", query)
    except Exception as e:
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

    # If both specialists are unavailable, return canned error
    if log_report and log_report.startswith("(") and code_report and code_report.startswith("("):
        return SPECIALISTS_UNAVAILABLE

    # Truncate reports to fit prompt budget
    log_text = (log_report or "(no log data)")[:MAX_REPORT_CHARS]
    code_text = (code_report or "(no code data)")[:MAX_REPORT_CHARS]

    prompt = TROUBLESHOOT_PROMPT.format(
        query=query,
        log_report=log_text,
        code_report=code_text,
    )

    try:
        return infer(prompt, max_tokens=512)
    except Exception:
        # LLM failed — return raw specialist reports as fallback
        return (
            f"--- Log Analysis ---\n{log_text}\n\n"
            f"--- Code Analysis ---\n{code_text}"
        )


# =============================================================================
# Orchestration
# =============================================================================

def handle_query(query):
    """Route: greeting → canned, question → grounded Q&A, troubleshoot → synthesis."""
    if is_greeting(query):
        return GREETING_RESPONSE

    category = triage(query)
    if category == "troubleshoot":
        return handle_troubleshoot(query)
    return handle_question(query)


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
