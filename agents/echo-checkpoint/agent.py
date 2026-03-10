"""Echo agent — durable mode (ADR 111).

Embeds user input via Ollama, returns the embedding dimensions as proof
the round-trip worked.
"""

from vlinder import Agent

agent = Agent()


@agent.on_invoke
def handle_invoke(ctx):
    ctx.call(
        "http://ollama.vlinder.local:3544/api/embed",
        json={
            "model": "nomic-embed-text",
            "input": ctx.input,
        },
        then=handle_result,
    )


@agent.handler
def handle_result(ctx):
    result = ctx.result.json()
    embeddings = result.get("embeddings", [[]])
    dims = len(embeddings[0]) if embeddings else 0
    ctx.complete(f"Embedded -> {dims} dimensions")


if __name__ == "__main__":
    agent.run()
