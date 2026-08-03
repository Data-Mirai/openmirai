# openmirai

TypeScript SDK for [OpenMirai](https://github.com/Data-Mirai/openmirai) — agentic graph execution.

## Install

```bash
npm install openmirai
```

Requires Node 18+.

## Quickstart

```typescript
import { Engine, Agent } from 'openmirai';

const engine = new Engine({ provider: 'ollama' }); // or 'claude', 'openai', 'gemini', 'groq', 'nvidia', 'openrouter'
const agent = Agent.fromFile('my-agent.yaml');
const result = await engine.run(agent, { input: { query: 'hello' } });
console.log(result.output);

// Streaming
for await (const event of engine.stream(agent, { input: { query: 'hello' } })) {
  console.log(event.event, event.data);
}
```

The SDK is a thin client over the engine's HTTP API. Run the engine locally:

```bash
mirai serve --port 3000
```

Then point the SDK at it (defaults to `http://localhost:3000`, override via `serverUrl`).

## License

MIT
