# Maintaining the Documentation

This file defines how OpenMirai documentation stays aligned with the code. The
repository is the source of truth; prose and machine-readable contracts must be
updated in the same change that alters a public surface.

## Sources of truth

| Contract | Primary source | Documentation that must follow it |
|---|---|---|
| AgentSpec fields and defaults | `engine/src/core/agent_spec.rs` and related spec types | `AGENT_SPEC.md`, `schemas/agent-spec.schema.json`, examples |
| Runtime graph behavior | `engine/src/core/runner/`, node executors, and data mapping | `SYSTEM_LIFECYCLE.md`, `ARCHITECTURE.md`, `USAGE.md` |
| HTTP routes and payloads | `engine/src/server/mod.rs` and route handlers | `backend/API.md`, `backend/openapi.yaml` |
| CLI syntax and file loading | `cli/src/main.rs` and CLI modules | `CLI.md`, `CLI.es.md`, `USAGE.md` |
| Persistence | store implementations and migrations | `database/SCHEMA.md`, `database/STORAGE.md`, `infra/OPERATIONS.md` |
| Deployment behavior | build files, server startup, environment reads | `infra/INFRA.md`, `infra/CLOUD-DEPLOYMENT.md`, `SECURITY.md` |
| SDK behavior | `sdks/python/` and `sdks/typescript/` | `SDK.md` |

Machine-readable files in this repository are maintained contracts, not
generated artifacts. A code change is incomplete when its schema or OpenAPI
description is stale.

## Capability language

Use one of these labels when a subsystem is not uniformly available:

- **wired**: reachable through a documented entry point and exercised end to
  end;
- **library-only**: implemented in a crate/module but not wired into the
  standard CLI or HTTP startup path;
- **partial**: reachable, but an important lifecycle or production behavior is
  absent;
- **placeholder**: a public shape exists without the promised implementation;
- **planned**: no current implementation should be inferred.

Do not turn the existence of a type, route, or module into a claim that the
whole feature works. State the entry point, host wiring, persistence boundary,
and known limitation.

## Change checklist

When a change affects behavior:

1. Update every row in the source-of-truth table that applies.
2. Add or update an executable example and the closest lifecycle description.
3. Update compatibility and security notes for breaking or trust-boundary
   changes.
4. Avoid fixed test counts, line counts, and other values that become stale
   without changing behavior.
5. Check English and translated files. English is canonical; if a translation
   cannot be updated in the same change, add a visible synchronization note.
6. Validate local links, JSON, YAML, and relevant tests before review.

Useful validation commands from the repository root:

```bash
git diff --check
jq empty schemas/agent-spec.schema.json
ruby -e 'require "yaml"; YAML.load_file("docs/backend/openapi.yaml")'
cargo test
```

The JSON Schema intentionally permits unknown fields because the Rust
deserializer currently does. Tightening that rule is a behavior change and must
land in code and schema together.

## Review questions

- Can a reader distinguish declared configuration from configuration applied
  by the default host?
- Does each external request show where authentication, validation, timeouts,
  cancellation, and persistence occur?
- Are startup, steady-state execution, failure, restart, and shutdown covered?
- Does the documentation say which data leaves the process and which storage
  must survive a restart?
- Can the examples be copied without relying on an undocumented default?
