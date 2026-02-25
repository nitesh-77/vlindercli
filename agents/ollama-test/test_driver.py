"""Ollama test agent — exercises all four Ollama endpoints.

On the first turn, presents a menu asking the user to pick an endpoint:
  1. OpenAI-compatible  (/v1/chat/completions)
  2. Ollama native chat (/api/chat)
  3. Ollama native generate (/api/generate)
  4. Ollama native embed (/api/embed)

Subsequent turns use the chosen endpoint to answer queries (1-3)
or embed text (4).
"""

import sys
import time
import json
from http.server import HTTPServer, BaseHTTPRequestHandler

import requests
from openai import OpenAI


OLLAMA_BASE = "http://ollama.vlinder.local"
MODEL = "phi3:latest"
EMBED_MODEL = "nomic-embed-text"

MENU = (
    "I will answer your query, but before that, I just need this one "
    "small thing from you...\n"
    "\n"
    "Which endpoint should I use?\n"
    "  1. OpenAI-compatible (/v1/chat/completions)\n"
    "  2. Ollama native chat (/api/chat)\n"
    "  3. Ollama native generate (/api/generate)\n"
    "  4. Ollama native embed (/api/embed)\n"
    "\n"
    "Please reply with 1, 2, 3, or 4."
)

openai_client = OpenAI(
    base_url=f"{OLLAMA_BASE}/v1",
    api_key="unused",
)


def log(msg):
    print(f"[agent] {msg}", file=sys.stderr, flush=True)


def parse_history(payload):
    """Parse the harness-provided conversation payload into turns.

    Returns list of (role, text) tuples. Role is 'user' or 'agent'.
    Continuation lines (not starting with User:/Agent:) are appended
    to the previous turn.
    """
    turns = []
    for line in payload.strip().split("\n"):
        if line.startswith("User: "):
            turns.append(("user", line[len("User: "):]))
        elif line.startswith("Agent: "):
            turns.append(("agent", line[len("Agent: "):]))
        elif turns:
            # Continuation of the previous turn (multi-line response)
            role, text = turns[-1]
            turns[-1] = (role, text + "\n" + line)
    return turns


def find_choice(turns):
    """Walk the history to find the user's endpoint choice (1-4).

    The menu is the first agent turn. The choice is the user reply after it.
    """
    saw_menu = False
    for role, text in turns:
        if role == "agent" and "Which endpoint should I use?" in text:
            saw_menu = True
        elif role == "user" and saw_menu:
            stripped = text.strip()
            if stripped in ("1", "2", "3", "4"):
                return int(stripped)
            saw_menu = False  # wasn't a valid choice, keep looking
    return None


def call_openai_compat(user_message):
    """Endpoint 1: /v1/chat/completions via OpenAI SDK."""
    log("using /v1/chat/completions")
    t0 = time.monotonic()
    completion = openai_client.chat.completions.create(
        model=MODEL,
        messages=[{"role": "user", "content": user_message}],
        max_tokens=2048,
    )
    elapsed = time.monotonic() - t0
    reply = completion.choices[0].message.content
    log(f"openai-compat ok, {len(reply)} chars, {elapsed:.2f}s")
    return reply


def call_native_chat(user_message):
    """Endpoint 2: /api/chat via raw HTTP."""
    log("using /api/chat")
    t0 = time.monotonic()
    resp = requests.post(
        f"{OLLAMA_BASE}/api/chat",
        json={
            "model": MODEL,
            "messages": [{"role": "user", "content": user_message}],
            "stream": False,
        },
    )
    resp.raise_for_status()
    data = resp.json()
    elapsed = time.monotonic() - t0
    reply = data["message"]["content"]
    log(f"native chat ok, {len(reply)} chars, {elapsed:.2f}s")
    return reply


def call_native_generate(user_message):
    """Endpoint 3: /api/generate via raw HTTP."""
    log("using /api/generate")
    t0 = time.monotonic()
    resp = requests.post(
        f"{OLLAMA_BASE}/api/generate",
        json={
            "model": MODEL,
            "prompt": user_message,
            "stream": False,
        },
    )
    resp.raise_for_status()
    data = resp.json()
    elapsed = time.monotonic() - t0
    reply = data["response"]
    log(f"native generate ok, {len(reply)} chars, {elapsed:.2f}s")
    return reply


def call_embed(user_message):
    """Endpoint 4: /api/embed via raw HTTP."""
    log("using /api/embed")
    t0 = time.monotonic()
    resp = requests.post(
        f"{OLLAMA_BASE}/api/embed",
        json={
            "model": EMBED_MODEL,
            "input": user_message,
        },
        timeout=60,
    )
    resp.raise_for_status()
    data = resp.json()
    elapsed = time.monotonic() - t0
    embedding = data["embeddings"][0]
    dims = len(embedding)
    preview = ", ".join(f"{v:.4f}" for v in embedding[:5])
    log(f"embed ok, {dims} dimensions, {elapsed:.2f}s")
    return f"Embedded ({dims} dimensions, {elapsed:.2f}s):\n[{preview}, ...]"


ENDPOINT_FNS = {
    1: call_openai_compat,
    2: call_native_chat,
    3: call_native_generate,
    4: call_embed,
}

ENDPOINT_NAMES = {
    1: "OpenAI-compatible (/v1/chat/completions)",
    2: "Ollama native chat (/api/chat)",
    3: "Ollama native generate (/api/generate)",
    4: "Ollama native embed (/api/embed)",
}


class Handler(BaseHTTPRequestHandler):
    def do_GET(self):
        """Health check."""
        self.send_response(200)
        self.end_headers()

    def do_POST(self):
        """Handle invoke from the sidecar."""
        length = int(self.headers.get("Content-Length", 0))
        payload = self.rfile.read(length).decode()
        log(f"POST received, {length} bytes")

        try:
            turns = parse_history(payload)
            choice = find_choice(turns)

            if choice is None:
                # First turn or no valid choice yet — show the menu.
                reply = MENU
            else:
                # We have a choice. Collect user messages and figure out
                # which ones came before vs. after the menu choice.
                user_messages = [t for r, t in turns if r == "user"]

                # Find the original question (first user message, before
                # the menu was shown) and messages after the choice.
                original_question = user_messages[0] if user_messages else ""
                post_choice_messages = []
                found_choice = False
                for role, text in turns:
                    if role == "agent" and "Which endpoint should I use?" in text:
                        found_choice = False
                    elif role == "user" and not found_choice:
                        if text.strip() in ("1", "2", "3", "4"):
                            found_choice = True
                            continue
                    if found_choice and role == "user":
                        post_choice_messages.append(text)

                fn = ENDPOINT_FNS[choice]

                if not post_choice_messages:
                    # First turn after the choice — answer the original question.
                    llm_reply = fn(original_question)
                    reply = (
                        f"Using {ENDPOINT_NAMES[choice]}.\n\n{llm_reply}"
                    )
                else:
                    # Subsequent turns — use the latest message.
                    llm_reply = fn(post_choice_messages[-1])
                    reply = llm_reply

            self.send_response(200)
            self.send_header("Content-Length", str(len(reply)))
            self.end_headers()
            self.wfile.write(reply.encode())

        except Exception as e:
            error_msg = str(e)
            log(f"error: {error_msg[:200]}")
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
