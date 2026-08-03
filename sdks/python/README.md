# openmirai

Python SDK for [OpenMirai](https://github.com/Data-Mirai/openmirai) — agentic graph execution.

## Install

```bash
pip install openmirai
```

Optional YAML loading:

```bash
pip install "openmirai[yaml]"
```

## Quickstart

```python
from openmirai import Engine, Agent

engine = Engine(provider="ollama")  # or "claude", "openai", "gemini", "groq", "nvidia", "openrouter"
agent = Agent.from_file("my-agent.yaml")
result = engine.run(agent, input={"query": "hello"})
print(result.output)
```

The SDK is a thin client over the engine's HTTP API. Run the engine locally:

```bash
mirai serve --port 3000
```

Then point the SDK at it (defaults to `http://localhost:3000`).

## License

MIT
