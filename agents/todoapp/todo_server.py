"""
Todo app agent — natural language todo list powered by various models.
HTTP API version. The agent runs as a simple Flask server.
"""

import json
import os
import requests
from flask import Flask, request, Response

# =============================================================================
# Flask App and Vlinder Client Setup
# =============================================================================

app = Flask(__name__)

# The sidecar's internal API address is provided by an environment variable.
# Defaulting to localhost:9000 for local testing.
SIDECAR_API_ROOT = os.environ.get("VLINDER_SIDECAR_API", "http://127.0.0.1:9000")

INFER_MODEL = "anthropic/claude-sonnet-4"
EMBED_MODEL = "nomic-embed-text"
TODOS_PATH = "/todos.json"


class VlinderClient:
    """A simple client for the Vlinder sidecar's internal HTTP API."""
    def _post(self, path, payload):
        url = f"{SIDECAR_API_ROOT}{path}"
        try:
            response = requests.post(url, json=payload, timeout=60)
            response.raise_for_status()
            return response
        except requests.RequestException as e:
            # Propagate errors as a 502 Bad Gateway response
            error_message = f"Sidecar API request failed: {e}"
            app.logger.error(error_message)
            raise ConnectionError(error_message)

    def kv_get(self, path):
        response = self._post("/services/kv/get", {"path": path})
        return response.content

    def kv_put(self, path, content):
        self._post("/services/kv/put", {"path": path, "content": content})

    def vector_store(self, key, vector, metadata):
        self._post("/services/vector/store", {"key": key, "vector": vector, "metadata": metadata})

    def vector_search(self, vector, limit=5):
        response = self._post("/services/vector/search", {"vector": vector, "limit": limit})
        return response.json()
    
    def vector_delete(self, key):
        self._post("/services/vector/delete", {"key": key})

    def infer(self, model, prompt, max_tokens=256):
        response = requests.post(
            "http://openrouter.vlinder.local/v1/chat/completions",
            json={
                "model": model,
                "messages": [{"role": "user", "content": prompt}],
                "max_tokens": max_tokens,
            },
            timeout=60,
        )
        response.raise_for_status()
        return response.json()["choices"][0]["message"]["content"]

    def embed(self, model, text):
        response = requests.post(
            "http://ollama.vlinder.local/api/embed",
            json={"model": model, "input": text},
            timeout=60,
        )
        response.raise_for_status()
        return response.json()["embeddings"][0]

client = VlinderClient()

# =============================================================================
# Pure Helpers
# =============================================================================

def parse_payload(raw_input):
    lines = raw_input.strip().splitlines()
    if lines and lines[-1].startswith("User: "):
        current = lines[-1][len("User: "):]
        history = lines[:-1]
        return current, history
    return raw_input.strip(), []

def format_todos(todos):
    if not todos:
        return "No todos yet. Try: add buy groceries"
    lines = []
    for i, todo in enumerate(todos, 1):
        status = "x" if todo.get("done") else " "
        lines.append(f"  [{status}] {i}. {todo['text']}")
    return "\n".join(lines)

def parse_todos(data):
    if not data or len(data) == 0:
        return []
    try:
        return json.loads(data)
    except (json.JSONDecodeError, ValueError):
        return []

# =============================================================================
# Main Endpoints
# =============================================================================

@app.route("/health", methods=["GET"])
def health_check():
    return "ok", 200

@app.route("/invoke", methods=["POST"])
def invoke():
    try:
        raw_input = request.data.decode('utf-8')
        current_input, history = parse_payload(raw_input)
        text = current_input.strip()
        lower = text.lower()

        if not text:
            return "Send a command: add, list, done, remove, clear, search, or ask a question.", 200

        # --- Search Command ---
        if lower.startswith("search "):
            query = text[7:].strip()
            if not query:
                return "Usage: search <query>", 200
            
            query_vector = client.embed(EMBED_MODEL, query)
            matches = client.vector_search(query_vector, 5)
            
            if not matches:
                return "No matching todos found.", 200
            
            lines = ["Search results:"]
            for r in matches:
                metadata = r.get("metadata", r.get("key", "?"))
                lines.append(f"  - {metadata}")
            return "\n".join(lines), 200

        # For all other commands, we need the todo list first
        todos_data = client.kv_get(TODOS_PATH)
        todos = parse_todos(todos_data)

        # --- List Command ---
        if lower in ("list", "ls", "show"):
            return format_todos(todos), 200

        # --- Add Command ---
        if lower.startswith("add "):
            item_text = text[4:].strip()
            if not item_text:
                return "Usage: add <todo text>", 200
            todos.append({"text": item_text, "done": False})
            client.kv_put(TODOS_PATH, json.dumps(todos))
            embedding = client.embed(EMBED_MODEL, item_text)
            client.vector_store(item_text, embedding, item_text)
            return f"Added: {item_text}\n\n{format_todos(todos)}", 200

        # --- Done/Undo Commands ---
        if lower.startswith("done ") or lower.startswith("undo "):
            is_done = lower.startswith("done ")
            num_str = text.split(maxsplit=1)[1]
            try:
                idx = int(num_str) - 1
            except ValueError:
                return f"Usage: {'done' if is_done else 'undo'} <number>", 200
            if idx < 0 or idx >= len(todos):
                return f"No todo #{idx + 1}. You have {len(todos)} todo(s).", 200
            
            todos[idx]["done"] = is_done
            client.kv_put(TODOS_PATH, json.dumps(todos))
            verb = "Completed" if is_done else "Reopened"
            return f"{verb}: {todos[idx]['text']}\n\n{format_todos(todos)}", 200

        # --- Remove Command ---
        if lower.startswith("remove ") or lower.startswith("rm "):
            parts = text.split(maxsplit=1)
            if len(parts) < 2:
                return "Usage: remove <number>", 200
            try:
                idx = int(parts[1]) - 1
            except ValueError:
                return "Usage: remove <number>", 200
            if idx < 0 or idx >= len(todos):
                return f"No todo #{idx + 1}. You have {len(todos)} todo(s).", 200
            
            removed = todos.pop(idx)
            client.kv_put(TODOS_PATH, json.dumps(todos))
            client.vector_delete(removed["text"])
            return f"Removed: {removed['text']}\n\n{format_todos(todos)}", 200

        # --- Clear Command ---
        if lower in ("clear", "clean"):
            remaining = [t for t in todos if not t.get("done")]
            removed_count = len(todos) - len(remaining)
            client.kv_put(TODOS_PATH, json.dumps(remaining))
            # Note: This doesn't remove vectors of cleared items for simplicity
            return f"Cleared {removed_count} completed todo(s).\n\n{format_todos(remaining)}", 200
        
        # --- Default to LLM (Ask) ---
        todo_list_str = format_todos(todos) if todos else "(empty list)"
        prompt_parts = [
            "You are a helpful todo list assistant.",
            f"Here are the user's current todos:\n{todo_list_str}",
        ]
        if history:
            prompt_parts.append("Recent conversation:\n" + "\n".join(history))
        prompt_parts.append(f"The user asks: {text}")
        prompt_parts.append("Give a brief, helpful response.")
        
        prompt = "\n\n".join(prompt_parts)
        response_text = client.infer(INFER_MODEL, prompt, 256)
        return response_text, 200

    except ConnectionError as e:
        return str(e), 502 # Bad Gateway
    except Exception as e:
        app.logger.error(f"Unhandled exception in /invoke: {e}")
        return f"Internal Agent Error: {e}", 500

if __name__ == "__main__":
    app.run(host="0.0.0.0", port=8080)