# Compatibility and Upgrade Policy

This document distinguishes the versioned surfaces in OpenMirai 0.7.0 and
states what consumers can safely assume today. It describes current behavior;
the project has not yet declared a long-term-support window.

## Versioned surfaces

| Surface | Current marker | Compatibility meaning |
|---|---|---|
| Engine/CLI release | `VERSION` (`0.7.0`) | SemVer release identity compiled into runtime metadata. |
| Rust crates | Cargo package versions | Must match `VERSION` at release. |
| HTTP API | `/api/v1` | Route namespace. It does not independently version every response field. |
| Agent specification | `version: v1` | Declarative format marker; currently copied through without negotiation. |
| Tool node | node `version`, default `1.0.0` | Metadata today; registry dispatch primarily keys on `tool_type`. |
| SQLite schema | `_schema_version` | Incremental migration level managed by the database layer. |
| Python/TypeScript SDK | package version | Intended to track the engine release but feature/auth parity is not guaranteed automatically. |

## SemVer expectations

Before 1.0, minor releases may still refine public contracts. The release
guide currently intends patch releases for fixes, minor releases for
backward-compatible features, and major releases for breaking API or AgentSpec
changes. Because several contracts are not generated, verify release notes and
run integration tests when upgrading even between minor versions.

## AgentSpec compatibility

- YAML files are the canonical on-disk format.
- HTTP `from-spec` accepts the equivalent JSON object.
- Missing fields receive Serde defaults.
- Unknown fields are currently ignored rather than rejected.
- `version` is not used to select a parser or migration routine.
- Top-level outputs are descriptive and several config fields are not applied
  by the default host; do not infer new runtime behavior from schema presence.

Consumers should validate against
[`agent-spec.schema.json`](../schemas/agent-spec.schema.json), run
`mirai validate`, and retain execution tests for important agents.

## HTTP API compatibility

`/api/v1` is the stable namespace, but there is no generated OpenAPI contract
or deprecation middleware in 0.7.0. Clients should:

- ignore unknown response fields;
- tolerate additional SSE event types;
- treat documented status codes and terminal statuses as the contract;
- send explicit `Content-Type: application/json` and `X-API-Key` when auth is enabled;
- avoid depending on currently ignored fields such as execute
  `entry_node_id`;
- pin engine and SDK versions together for deployment testing.

## Persistence compatibility

The server migrates SQLite on open. Back up the database before upgrading and
never downgrade against a database already migrated by a newer binary unless
that release explicitly documents downgrade support. The project does not
currently provide automated downgrade migrations.

Only final run history is durably restored by the default server. Agent/graph
registries, agent KV memory, scheduler state, and in-flight executions do not
become compatible/durable merely because the database schema migrates.

## Provider and model compatibility

Provider protocols and model identifiers can change outside OpenMirai's
release cycle. Pin the provider, model, base URL, and container image. A server
health response does not prove that a provider or model is available.

OpenAI-compatible endpoints are protocol integrations, not a promise that
every vendor extension or model feature is supported.

## SDK compatibility

The Python SDK may fall back from HTTP to the CLI on broad exceptions. The
TypeScript SDK uses HTTP only. Neither SDK currently sends `X-API-Key`, so they
are not compatible with an authenticated remote server without SDK changes or
an authenticated proxy arrangement. See [SDK guide](SDK.md).

## Upgrade procedure

1. Read `CHANGELOG.md` and release notes.
2. Verify `VERSION`, crate, and SDK versions match the intended artifact.
3. Back up SQLite and persistent `~/.openmirai` state.
4. Validate critical AgentSpecs with the new binary.
5. Run representative CLI, synchronous HTTP, streaming, MCP, and persistence tests.
6. Deploy a canary with the same provider/model/network policy as production.
7. Verify `/version`, readiness, run persistence, logs, and rollback artifacts.
8. Roll out one active replica at a time under the current state model.

## Deprecation checklist for maintainers

A breaking or deprecated field should be documented in the changelog, schema,
API reference, compatibility guide, SDKs, and examples. Maintain aliases for a
stated period where safe, emit actionable warnings, and add migration examples.

## Related documentation

- [AgentSpec reference](AGENT_SPEC.md)
- [Operations](infra/OPERATIONS.md)
- [Releasing](../RELEASING.md)
- [Database schema](database/SCHEMA.md)
- [HTTP API](backend/API.md)
