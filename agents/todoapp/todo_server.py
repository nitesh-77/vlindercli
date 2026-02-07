"""Todo app agent — natural language todo list powered by OpenRouter + Ollama.

Commands (matched by prefix, case-insensitive):
  add <text>      — Add a new todo item (also embeds it for search)
  list            — Show all todos
  done <number>   — Mark a todo as done
  remove <number> — Remove a todo
  clear           — Remove all completed todos
  search <query>  — Find related todos by meaning (vector search)
  <anything else> — Ask the LLM for advice about your todos
"""

import base64
import json
import os
import urllib.request
from http.server import HTTPServer, BaseHTTPRequestHandler

BRIDGE = os.environ.get("VLINDER_BRIDGE_URL", "")
INFER_MODEL = "claude-sonnet"
EMBED_MODEL = "nomic-embed"
TODOS_PATH = "/todos.json"


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


def embed(text):
    """Get embedding vector for text via Ollama."""
    result = bridge_call("/embed", {
        "model": EMBED_MODEL,
        "text": text,
    })
    # Bridge returns the raw float array as JSON
    return json.loads(result)


def kv_get(path):
    """Read a value from KV storage. Returns None if not found."""
    try:
        result = bridge_call("/kv/get", {"path": path})
        return result
    except Exception:
        return None


def kv_put(path, content):
    """Write a value to KV storage."""
    encoded = base64.b64encode(content).decode()
    bridge_call("/kv/put", {"path": path, "content": encoded})


def vector_store(key, vector, metadata):
    """Store a vector with metadata."""
    bridge_call("/vector/store", {
        "key": key,
        "vector": vector,
        "metadata": metadata,
    })


def vector_search(vector, limit=5):
    """Search for similar vectors. Returns JSON array of results."""
    result = bridge_call("/vector/search", {
        "vector": vector,
        "limit": limit,
    })
    return json.loads(result)


def vector_delete(key):
    """Delete a vector by key."""
    bridge_call("/vector/delete", {"key": key})


# =============================================================================
# Todo storage
# =============================================================================

def load_todos():
    """Load todos from KV storage."""
    data = kv_get(TODOS_PATH)
    if data is None or data == b"":
        return []
    try:
        return json.loads(data)
    except (json.JSONDecodeError, ValueError):
        return []


def save_todos(todos):
    """Save todos to KV storage."""
    kv_put(TODOS_PATH, json.dumps(todos).encode())


def format_todos(todos):
    """Format todo list for display."""
    if not todos:
        return "No todos yet. Try: add buy groceries"
    lines = []
    for i, todo in enumerate(todos, 1):
        status = "x" if todo.get("done") else " "
        lines.append(f"  [{status}] {i}. {todo['text']}")
    return "\n".join(lines)


def todo_key(index):
    """Vector storage key for a todo by index."""
    return f"todo-{index}"


# =============================================================================
# Command handling
# =============================================================================

def handle_command(raw_input):
    """Parse and execute a todo command. Returns response text."""
    text = raw_input.strip()
    if not text:
        return "Send a command: add, list, done, remove, clear, search, or ask a question."

    lower = text.lower()

    # --- add ---
    if lower.startswith("add "):
        item_text = text[4:].strip()
        if not item_text:
            return "Usage: add <todo text>"
        todos = load_todos()
        todos.append({"text": item_text, "done": False})
        save_todos(todos)

        # Embed and store for semantic search
        try:
            vector = embed(item_text)
            vector_store(todo_key(len(todos) - 1), vector, item_text)
        except Exception as e:
            pass  # Non-fatal — search won't find this item

        return f"Added: {item_text}\n\n{format_todos(todos)}"

    # --- list ---
    if lower in ("list", "ls", "show"):
        todos = load_todos()
        return format_todos(todos)

    # --- search ---
    if lower.startswith("search "):
        query = text[7:].strip()
        if not query:
            return "Usage: search <query>"
        return search_todos(query)

    # --- done ---
    if lower.startswith("done "):
        return toggle_done(text[5:].strip(), True)

    # --- undo ---
    if lower.startswith("undo "):
        return toggle_done(text[5:].strip(), False)

    # --- remove ---
    if lower.startswith("remove ") or lower.startswith("rm "):
        parts = text.split(maxsplit=1)
        return remove_todo(parts[1].strip() if len(parts) > 1 else "")

    # --- clear ---
    if lower in ("clear", "clean"):
        todos = load_todos()
        remaining = [t for t in todos if not t.get("done")]
        removed_count = len(todos) - len(remaining)
        # Re-index vectors for remaining todos
        reindex_vectors(remaining)
        save_todos(remaining)
        return f"Cleared {removed_count} completed todo(s).\n\n{format_todos(remaining)}"

    # --- anything else: ask the LLM ---
    return ask_llm(text)


def search_todos(query):
    """Find todos similar to the query using vector search."""
    try:
        query_vector = embed(query)
        results = vector_search(query_vector, limit=5)
    except Exception as e:
        return f"[search error] {e}"

    if not results:
        return "No matching todos found."

    lines = ["Search results:"]
    for r in results:
        metadata = r.get("metadata", r.get("key", "?"))
        lines.append(f"  - {metadata}")
    return "\n".join(lines)


def toggle_done(num_str, done):
    """Mark a todo as done or not done."""
    try:
        idx = int(num_str) - 1
    except ValueError:
        return f"Usage: {'done' if done else 'undo'} <number>"

    todos = load_todos()
    if idx < 0 or idx >= len(todos):
        return f"No todo #{idx + 1}. You have {len(todos)} todo(s)."

    todos[idx]["done"] = done
    save_todos(todos)
    verb = "Completed" if done else "Reopened"
    return f"{verb}: {todos[idx]['text']}\n\n{format_todos(todos)}"


def remove_todo(num_str):
    """Remove a todo by number."""
    try:
        idx = int(num_str) - 1
    except ValueError:
        return "Usage: remove <number>"

    todos = load_todos()
    if idx < 0 or idx >= len(todos):
        return f"No todo #{idx + 1}. You have {len(todos)} todo(s)."

    removed = todos.pop(idx)
    save_todos(todos)

    # Re-index vectors after removal
    reindex_vectors(todos)

    return f"Removed: {removed['text']}\n\n{format_todos(todos)}"


def reindex_vectors(todos):
    """Re-embed and store all todos with correct indices."""
    # Delete old vectors then re-store
    for i, todo in enumerate(todos):
        try:
            vector = embed(todo["text"])
            vector_store(todo_key(i), vector, todo["text"])
        except Exception:
            pass


def ask_llm(question):
    """Send the question + current todos to the LLM for advice."""
    todos = load_todos()
    todo_list = format_todos(todos) if todos else "(empty list)"

    prompt = (
        f"You are a helpful todo list assistant. "
        f"Here are the user's current todos:\n{todo_list}\n\n"
        f"The user asks: {question}\n\n"
        f"Give a brief, helpful response."
    )

    try:
        return infer(prompt, max_tokens=256)
    except Exception as e:
        return f"[LLM error] {e}"


# =============================================================================
# HTTP server
# =============================================================================

class Handler(BaseHTTPRequestHandler):
    def do_GET(self):
        """Health check."""
        self.send_response(200)
        self.end_headers()

    def do_POST(self):
        """Handle an invocation."""
        length = int(self.headers.get("Content-Length", 0))
        body = self.rfile.read(length).decode()

        result = handle_command(body)
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
