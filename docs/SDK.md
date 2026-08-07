# SDK Clients

OpenMirai includes Python and TypeScript packages that wrap the HTTP API. They
are thin clients, not alternate engine implementations, and their current
surface is smaller than the server API.

## Support matrix

| Capability | Python | TypeScript |
|---|---:|---:|
| Create an agent through `from-spec` | Yes | Yes |
| Synchronous execute | Yes | Yes |
| SSE stream | Yes | Yes |
| CLI fallback | Yes | No |
| YAML file load | Yes with PyYAML | No |
| JSON/object construction | `from_dict` | `fromDict`; file loader supports JSON |
| `X-API-Key` server authentication | Not currently sent | Not currently sent |
| Full graph/agent/session/orchestrator API | No | No |
| Cancellation API | No | No |

The `api_key`/`apiKey` constructor option describes provider configuration in
the SDK shape but is not added as `X-API-Key` by current HTTP calls. Both SDKs
therefore fail against a normally authenticated remote `mirai serve` unless a
trusted proxy supplies the header or the SDK is fixed. Do not disable server
authentication to accommodate the SDK in production.

## Python

```python
from openmirai import Agent, Engine

agent = Agent.from_file("agent.yaml")
engine = Engine(server_url="http://127.0.0.1:3000")
result = engine.run(agent, input={"question": "hello"})
print(result.status, result.state)
```

`Agent.from_file()` accepts `.yaml`/`.yml` and requires PyYAML. `Engine.run()`
tries HTTP first and catches any exception before falling back to a temporary
file and the `mirai` CLI. This broad fallback can hide the difference between
server/auth errors and local CLI execution; set expectations and observe which
path ran.

The fallback writes JSON text to a `.yaml` file. JSON is syntactically valid
YAML, so the Rust YAML parser can consume it despite canonical authoring being
YAML-only.

Streaming requires `requests`, registers the AgentSpec, then parses SSE lines.
It inherits the server streaming limitations and uses a 300-second Requests
timeout.

## TypeScript

```typescript
import { Agent, Engine } from 'openmirai';

const agent = Agent.fromDict({
  name: 'demo',
  graph: {
    nodes: [{ id: 'start', tool_type: 'trigger/manual' }],
    edges: [],
  },
});
const engine = new Engine({ serverUrl: 'http://127.0.0.1:3000' });
const result = await engine.run(agent, { input: { question: 'hello' } });
```

The current `Agent.fromFile()` reads JSON only even though its older examples
show YAML. Use `fromDict` after parsing YAML in the application, or add YAML
support to the SDK. Node 18+ is required. Execute uses `AbortSignal.timeout`;
aborting the HTTP request does not create a server-side durable cancellation.

## Result model

Both SDKs expose status, state, trace, transcript, and optional error. Python's
`result.output` convenience returns the entire graph state, not only a declared
AgentSpec output field; the TypeScript result currently has no `output`
property despite an outdated package comment that shows one.

Stream events expose an event name and decoded JSON data. The server JSON
payload itself contains nested `event` and `data`; consumers should test the
actual object shape and tolerate additions.

## Versioning and parity

Pin the SDK version with the server binary and run E2E tests. Package version
equality does not guarantee full endpoint, auth, error, or streaming parity.
Provider/model options are not transmitted to the already running server for
each execution; server startup configuration selects its LLM factory.

## Production requirements before remote use

- add explicit server API-key configuration and `X-API-Key` headers;
- distinguish HTTP failures from Python CLI fallback;
- add typed AgentSpec models generated from the schema;
- add OpenAPI-generated or contract-tested endpoint clients;
- preserve structured error bodies/status codes;
- add cancellation/session correlation;
- add reconnect/terminal-event tests for SSE;
- document proxy, TLS, timeout, and retry behavior;
- align TypeScript YAML behavior with the engine contract.

## Related documentation

- [AgentSpec](AGENT_SPEC.md)
- [HTTP API](backend/API.md)
- [Streaming](backend/STREAMING.md)
- [Compatibility](COMPATIBILITY.md)
- [Python package README](../sdks/python/README.md)
- [TypeScript package README](../sdks/typescript/README.md)
