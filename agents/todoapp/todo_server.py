"""Todo app agent — natural language todo list powered by OpenRouter + Ollama.

State machine version (ADR 075). Each service call is a separate step.
The agent tracks its progress via ctx.state["step"].

Commands (matched by prefix, case-insensitive):
  add <text>      — Add a new todo item (also embeds it for search)
  list            — Show all todos
  done <number>   — Mark a todo as done
  remove <number> — Remove a todo
  clear           — Remove all completed todos
  search <query>  — Find related todos by meaning (vector search)
  <anything else> — Ask the LLM for advice about your todos
"""

import json
from vlinder import Agent

agent = Agent()

INFER_MODEL = "claude-sonnet"
EMBED_MODEL = "nomic-embed"
TODOS_PATH = "/todos.json"


# =============================================================================
# Pure helpers (no service calls)
# =============================================================================

def parse_payload(raw_input):
    """Extract the current command and conversation history from the payload."""
    lines = raw_input.strip().splitlines()
    if lines and lines[-1].startswith("User: "):
        current = lines[-1][len("User: "):]
        history = lines[:-1]
        return current, history
    return raw_input.strip(), []


def format_todos(todos):
    """Format todo list for display."""
    if not todos:
        return "No todos yet. Try: add buy groceries"
    lines = []
    for i, todo in enumerate(todos, 1):
        status = "x" if todo.get("done") else " "
        lines.append(f"  [{status}] {i}. {todo['text']}")
    return "\n".join(lines)


def parse_todos(data):
    """Parse todos from KvGet response bytes."""
    if not data or len(data) == 0:
        return []
    try:
        return json.loads(data)
    except (json.JSONDecodeError, ValueError):
        return []


# =============================================================================
# Event handlers
# =============================================================================

@agent.on_invoke
def handle_invoke(ctx):
    current_input, history = parse_payload(ctx.input)
    text = current_input.strip()

    if not text:
        ctx.complete("Send a command: add, list, done, remove, clear, search, or ask a question.")
        return

    ctx.state["raw_input"] = text
    ctx.state["history"] = history
    lower = text.lower()

    # Search needs embedding first, not todos
    if lower.startswith("search "):
        query = text[7:].strip()
        if not query:
            ctx.complete("Usage: search <query>")
            return
        ctx.state["cmd"] = "search"
        ctx.state["query"] = query
        ctx.state["step"] = "embed_query"
        ctx.embed(EMBED_MODEL, query)
        return

    # Parse command, then load todos
    if lower.startswith("add "):
        ctx.state["cmd"] = "add"
        ctx.state["item_text"] = text[4:].strip()
    elif lower in ("list", "ls", "show"):
        ctx.state["cmd"] = "list"
    elif lower.startswith("done "):
        ctx.state["cmd"] = "done"
        ctx.state["num"] = text[5:].strip()
    elif lower.startswith("undo "):
        ctx.state["cmd"] = "undo"
        ctx.state["num"] = text[5:].strip()
    elif lower.startswith("remove ") or lower.startswith("rm "):
        parts = text.split(maxsplit=1)
        ctx.state["cmd"] = "remove"
        ctx.state["num"] = parts[1].strip() if len(parts) > 1 else ""
    elif lower in ("clear", "clean"):
        ctx.state["cmd"] = "clear"
    else:
        ctx.state["cmd"] = "ask"

    ctx.state["step"] = "load_todos"
    ctx.kv_get(TODOS_PATH)


@agent.on_kv_get
def handle_kv_get(ctx):
    todos = parse_todos(ctx.data)
    cmd = ctx.state["cmd"]

    if cmd == "list":
        ctx.complete(format_todos(todos))
        return

    if cmd == "add":
        item_text = ctx.state["item_text"]
        if not item_text:
            ctx.complete("Usage: add <todo text>")
            return
        todos.append({"text": item_text, "done": False})
        ctx.state["todos_json"] = json.dumps(todos)
        ctx.state["response"] = f"Added: {item_text}\n\n{format_todos(todos)}"
        ctx.state["step"] = "save_after_add"
        ctx.kv_put(TODOS_PATH, json.dumps(todos))
        return

    if cmd in ("done", "undo"):
        is_done = cmd == "done"
        try:
            idx = int(ctx.state["num"]) - 1
        except ValueError:
            ctx.complete(f"Usage: {cmd} <number>")
            return
        if idx < 0 or idx >= len(todos):
            ctx.complete(f"No todo #{idx + 1}. You have {len(todos)} todo(s).")
            return
        todos[idx]["done"] = is_done
        verb = "Completed" if is_done else "Reopened"
        ctx.state["response"] = f"{verb}: {todos[idx]['text']}\n\n{format_todos(todos)}"
        ctx.state["step"] = "save_final"
        ctx.kv_put(TODOS_PATH, json.dumps(todos))
        return

    if cmd == "remove":
        try:
            idx = int(ctx.state["num"]) - 1
        except ValueError:
            ctx.complete("Usage: remove <number>")
            return
        if idx < 0 or idx >= len(todos):
            ctx.complete(f"No todo #{idx + 1}. You have {len(todos)} todo(s).")
            return
        removed = todos.pop(idx)
        ctx.state["removed_text"] = removed["text"]
        ctx.state["response"] = f"Removed: {removed['text']}\n\n{format_todos(todos)}"
        ctx.state["step"] = "save_after_remove"
        ctx.kv_put(TODOS_PATH, json.dumps(todos))
        return

    if cmd == "clear":
        remaining = [t for t in todos if not t.get("done")]
        removed_count = len(todos) - len(remaining)
        ctx.state["response"] = f"Cleared {removed_count} completed todo(s).\n\n{format_todos(remaining)}"
        ctx.state["step"] = "save_final"
        ctx.kv_put(TODOS_PATH, json.dumps(remaining))
        return

    if cmd == "ask":
        todo_list = format_todos(todos) if todos else "(empty list)"
        question = ctx.state["raw_input"]
        history = ctx.state.get("history", [])
        parts = [
            "You are a helpful todo list assistant.",
            f"Here are the user's current todos:\n{todo_list}",
        ]
        if history:
            parts.append("Recent conversation:\n" + "\n".join(history))
        parts.append(f"The user asks: {question}")
        parts.append("Give a brief, helpful response.")
        ctx.state["step"] = "ask_llm"
        ctx.infer(INFER_MODEL, "\n\n".join(parts), 256)
        return


@agent.on_kv_put
def handle_kv_put(ctx):
    step = ctx.state.get("step", "")

    if step == "save_after_add":
        # Embed the new item for semantic search
        item_text = ctx.state["item_text"]
        ctx.state["step"] = "embed_new_item"
        ctx.embed(EMBED_MODEL, item_text)
        return

    if step == "save_after_remove":
        # Delete the vector for the removed item
        removed_text = ctx.state["removed_text"]
        ctx.state["step"] = "delete_removed_vector"
        ctx.vector_delete(removed_text)
        return

    # save_final — done
    ctx.complete(ctx.state.get("response", "Done"))


@agent.on_embed
def handle_embed(ctx):
    step = ctx.state.get("step", "")

    if step == "embed_query":
        # Got query embedding, now search
        ctx.state["step"] = "search_vectors"
        ctx.vector_search(ctx.vector, 5)
        return

    if step == "embed_new_item":
        # Got item embedding, store it (use text as key for easy deletion)
        item_text = ctx.state["item_text"]
        ctx.state["step"] = "store_new_vector"
        ctx.vector_store(item_text, ctx.vector, item_text)
        return


@agent.on_vec_store
def handle_vec_store(ctx):
    ctx.complete(ctx.state.get("response", "Done"))


@agent.on_vec_search
def handle_vec_search(ctx):
    if not ctx.matches:
        ctx.complete("No matching todos found.")
        return
    lines = ["Search results:"]
    for r in ctx.matches:
        metadata = r.get("metadata", r.get("key", "?"))
        lines.append(f"  - {metadata}")
    ctx.complete("\n".join(lines))


@agent.on_vec_delete
def handle_vec_delete(ctx):
    ctx.complete(ctx.state.get("response", "Done"))


@agent.on_infer
def handle_infer(ctx):
    ctx.complete(ctx.text)


@agent.on_error
def handle_error(ctx):
    step = ctx.state.get("step", "")
    # Embedding/vector errors after add are non-fatal — search just won't find the item
    if step in ("embed_new_item", "store_new_vector", "delete_removed_vector"):
        ctx.complete(ctx.state.get("response", "Done"))
        return
    ctx.complete(f"Error: {ctx.message}")


if __name__ == "__main__":
    agent.run()
