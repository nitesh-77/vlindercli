"""Council orchestrator — multi-round deliberation with consensus detection.

Delegates the user's question to three advisor agents in parallel, collects
their responses, checks for consensus via LLM, and iterates up to 3 rounds.
Each round enriches the context with prior advisor responses so advisors can
build on each other's ideas.
"""

import json
import sys
import urllib.request
from http.server import HTTPServer, BaseHTTPRequestHandler

RUNTIME_HOST = "http://runtime.vlinder.local:3544"
OPENROUTER_URL = "http://openrouter.vlinder.local:3544/v1/chat/completions"
MODEL = "google/gemini-2.0-flash-001"
MAX_ROUNDS = 3

ADVISORS = ["sales-advisor", "product-advisor", "architect-advisor"]


def log(msg):
    print(f"[orchestrator] {msg}", file=sys.stderr, flush=True)


# =============================================================================
# Delegation (runtime.vlinder.local)
# =============================================================================

def delegate(agent, input_text):
    """Fire-and-forget delegation via runtime. Returns a handle."""
    data = json.dumps({"target": agent, "input": input_text}).encode()
    req = urllib.request.Request(f"{RUNTIME_HOST}/delegate", data=data, method="POST")
    req.add_header("Content-Type", "application/json")
    req.add_header("Host", "runtime.vlinder.local")
    with urllib.request.urlopen(req) as resp:
        result = json.loads(resp.read())
    return result["handle"]


def wait_for(handle):
    """Block until a delegated task completes. Returns the output."""
    data = json.dumps({"handle": handle}).encode()
    req = urllib.request.Request(f"{RUNTIME_HOST}/wait", data=data, method="POST")
    req.add_header("Content-Type", "application/json")
    req.add_header("Host", "runtime.vlinder.local")
    with urllib.request.urlopen(req) as resp:
        return resp.read().decode()


# =============================================================================
# Inference (openrouter.vlinder.local)
# =============================================================================

def chat(system_prompt, user_message, max_tokens=1024):
    """Call OpenRouter chat completions."""
    payload = json.dumps({
        "model": MODEL,
        "messages": [
            {"role": "system", "content": system_prompt},
            {"role": "user", "content": user_message},
        ],
        "max_tokens": max_tokens,
    }).encode()

    req = urllib.request.Request(OPENROUTER_URL, data=payload, method="POST")
    req.add_header("Content-Type", "application/json")
    req.add_header("Host", "openrouter.vlinder.local")

    with urllib.request.urlopen(req) as resp:
        result = json.loads(resp.read())

    return result["choices"][0]["message"]["content"]


# =============================================================================
# Deliberation logic
# =============================================================================

def fan_out(prompt):
    """Delegate to all advisors in parallel, wait for all responses."""
    handles = {}
    for advisor in ADVISORS:
        try:
            handles[advisor] = delegate(advisor, prompt)
            log(f"delegated to {advisor}")
        except Exception as e:
            log(f"delegate to {advisor} failed: {e}")
            handles[advisor] = None

    responses = {}
    for advisor in ADVISORS:
        if handles[advisor] is None:
            responses[advisor] = f"(delegation failed)"
            continue
        try:
            responses[advisor] = wait_for(handles[advisor])
            log(f"{advisor} responded: {len(responses[advisor])} chars")
        except Exception as e:
            log(f"wait for {advisor} failed: {e}")
            responses[advisor] = f"(wait failed: {e})"

    return responses


def format_responses(responses):
    """Format advisor responses into a readable summary."""
    parts = []
    for advisor, response in responses.items():
        label = advisor.replace("-", " ").title()
        parts.append(f"=== {label} ===\n{response}")
    return "\n\n".join(parts)


def check_consensus(question, all_rounds):
    """Ask the LLM whether the advisors have reached consensus."""
    system = (
        "You are a neutral facilitator for a council of three advisors "
        "(sales, product, architect) deliberating on a strategic question "
        "about vlindercli, an AI agent platform.\n\n"
        "Analyze the advisor responses and determine if there is meaningful "
        "consensus — they don't need to agree on everything, but should "
        "converge on a clear recommendation.\n\n"
        "Reply in exactly this format:\n"
        "CONSENSUS: Yes (or No)\n"
        "SUMMARY: <one paragraph synthesizing the council's recommendation>"
    )

    rounds_text = ""
    for i, responses in enumerate(all_rounds, 1):
        rounds_text += f"\n--- Round {i} ---\n{format_responses(responses)}\n"

    user = f"Question: {question}\n{rounds_text}"

    return chat(system, user, max_tokens=512)


def build_round_prompt(question, all_rounds):
    """Build the prompt for advisors in subsequent rounds."""
    if not all_rounds:
        return question

    parts = [
        f"Original question: {question}",
        "",
        "Previous round responses from the council:",
    ]
    for i, responses in enumerate(all_rounds, 1):
        parts.append(f"\n--- Round {i} ---")
        parts.append(format_responses(responses))

    parts.append("")
    parts.append(
        "Consider the other advisors' perspectives and refine your recommendation. "
        "Where do you agree? Where do you disagree? What's your updated advice?"
    )

    return "\n".join(parts)


def deliberate(question):
    """Run the multi-round deliberation loop."""
    all_rounds = []

    for round_num in range(1, MAX_ROUNDS + 1):
        log(f"=== Round {round_num}/{MAX_ROUNDS} ===")

        prompt = build_round_prompt(question, all_rounds)
        responses = fan_out(prompt)
        all_rounds.append(responses)

        log(f"checking consensus after round {round_num}")
        try:
            verdict = check_consensus(question, all_rounds)
        except Exception as e:
            log(f"consensus check failed: {e}")
            verdict = "CONSENSUS: No\nSUMMARY: (consensus check failed)"

        log(f"verdict: {verdict[:100]}...")

        if "CONSENSUS: Yes" in verdict.upper().replace(" ", ""):
            # Extract just the summary
            if "SUMMARY:" in verdict:
                summary = verdict.split("SUMMARY:", 1)[1].strip()
            else:
                summary = verdict
            return format_final(question, all_rounds, summary, round_num)

    # No consensus after MAX_ROUNDS — synthesize anyway
    log("max rounds reached, forcing synthesis")
    if "SUMMARY:" in verdict:
        summary = verdict.split("SUMMARY:", 1)[1].strip()
    else:
        summary = verdict

    return format_final(question, all_rounds, summary, MAX_ROUNDS)


def format_final(question, all_rounds, summary, rounds_used):
    """Format the final council output."""
    parts = [
        f"Council of Elders — {rounds_used} round{'s' if rounds_used > 1 else ''} of deliberation",
        "=" * 60,
        "",
        summary,
        "",
        "=" * 60,
        "Detailed advisor responses from final round:",
        "",
        format_responses(all_rounds[-1]),
    ]
    return "\n".join(parts)


# =============================================================================
# HTTP server
# =============================================================================

def parse_payload(raw):
    """Extract user input, stripping fleet context prefix if present."""
    if "\n\n" in raw:
        parts = raw.split("\n\n", 1)
        if parts[0].startswith("Fleet:"):
            return parts[1]
    return raw.strip()


class Handler(BaseHTTPRequestHandler):
    def do_GET(self):
        self.send_response(200)
        self.end_headers()

    def do_POST(self):
        length = int(self.headers.get("Content-Length", 0))
        body = self.rfile.read(length).decode()
        question = parse_payload(body)
        log(f"question: {question[:100]}...")

        try:
            result = deliberate(question)
        except Exception as e:
            log(f"deliberation failed: {e}")
            result = f"Council deliberation failed: {e}"

        data = result.encode()
        self.send_response(200)
        self.send_header("Content-Length", str(len(data)))
        self.end_headers()
        self.wfile.write(data)

    def log_message(self, fmt, *args):
        pass


if __name__ == "__main__":
    log("council orchestrator starting on :8080")
    HTTPServer(("0.0.0.0", 8080), Handler).serve_forever()
