# OpenMirai Security Policy and Threat Model

OpenMirai executes declarative graphs whose tools can read and modify files,
run commands, access networks, call models and MCP servers, and start external
processes. Treat an agent specification as executable code and the HTTP API as
a remote-execution control plane.

This document describes the security posture of OpenMirai 0.7.0. It is not a
certification or a claim that the default process safely executes hostile
workloads.

## Reporting a vulnerability

Use GitHub's private security-advisory reporting flow for the
`Data-Mirai/openmirai` repository. Include the affected version/commit, impact,
reproduction steps, and any suggested mitigation. Do not publish working
exploits, credentials, private data, or a detailed issue in the public tracker
before maintainers have had an opportunity to respond.

For ordinary bugs without security impact, use the public issue tracker.

## Supported versions

The repository does not currently publish a multi-version security-support
window. Assume fixes target the latest release and main branch. A formal LTS
policy must be established before production consumers can rely on backports.

## Trust model

| Input or actor | Default trust assumption |
|---|---|
| Local agent YAML | Trusted as code by the user who runs it. |
| Authenticated HTTP caller | Authorized to invoke high-impact engine behavior; not sandboxed merely by authentication. |
| Trigger payload, web page, MCP response, retrieved document | Untrusted content that may contain malicious instructions or data. |
| LLM output | Untrusted; it can be incorrect, adversarially influenced, or request dangerous tool operations. |
| Tool implementation | Trusted engine code, but some tools intentionally expose host capabilities. |
| Container/VM boundary | Primary isolation boundary for agent side effects. |

OpenMirai has one shared HTTP API key. It does not currently provide user
identity, roles, tenant isolation, per-agent permissions, quotas, or an audit
principal.

## Security boundaries

```text
Internet/user
  -> TLS ingress / identity gateway / rate limit
  -> X-API-Key protected OpenMirai API
  -> AgentSpec and trigger validation
  -> GraphRunner
  -> ToolRegistry
  -> tool with process privileges
  -> mounted filesystem / network / child processes / provider secrets
```

The runner and registry validate structure and tool schemas. They do not make
unsafe tools safe. The operating-system identity, container mounts,
capabilities, seccomp profile, egress policy, and secret exposure determine the
maximum impact of an agent.

## HTTP authentication

- A non-loopback `mirai serve` bind is refused unless `MIRAI_API_KEY` or the
  server `--api-key` is configured.
- Clients send the secret in `X-API-Key`.
- `/health` and `/version` remain public.
- Orchestrator EventSource clients may use `?api_key=` because browser
  `EventSource` cannot set headers. Query credentials can appear in access
  logs and should be avoided at external gateways when possible.
- The listener is plain HTTP. Terminate TLS at a trusted ingress or reverse
  proxy.
- CORS is a browser control, not service authorization.

Use a high-entropy secret, inject it through a secret manager, prevent it from
entering command history and logs, and rotate it through a controlled restart.

## High-impact capabilities

| Capability | Risk |
|---|---|
| Filesystem tools | No application-level root jail; visible paths can be read or changed and delete is recursive. |
| `system/bash` | Runs `sh -c` with the engine user's permissions. Its blocklist is an accident guardrail. |
| `system/sandbox_exec` | Adds temp-directory, timeout, and language-runner controls but is not kernel isolation. |
| Git tools | Can stage and commit changes in accessible repositories. |
| Web scraping and provider calls | Outbound HTTP, data exfiltration, SSRF-like reachability, and untrusted content ingestion. |
| MCP stdio | Spawns configured programs and exchanges JSON-RPC over their standard streams. |
| MCP HTTP | Sends data to configured remote services. |
| Claude Code tool and orchestrator | Starts external coding processes with access to project directories. |
| Agent graphs | Authorized callers choose tool types, configs, mappings, and payloads. |

There is no built-in production tool allowlist in the default server. Removing
programs from an image makes some tools fail but does not remove them from the
catalog or constitute a permission model.

## Prompt injection and data handling

`ai/llm_call` scans text inputs for prompt injection by default. This is a
defense-in-depth signal, not a proof that content is safe. Data obtained from
web scraping, RAG, MCP, user uploads, or prior model output must stay untrusted.

Recommended controls:

1. separate instructions from retrieved/user content;
2. keep dangerous tools out of untrusted execution profiles;
3. require human approval outside OpenMirai for irreversible operations;
4. restrict network destinations and filesystem mounts;
5. validate structured output before using it as commands or paths;
6. redact trigger data, prompts, state, trace, and transcript before export;
7. never place provider secrets in AgentSpec, node config, or `data_map`.

## MCP security

Only configure MCP servers you trust. Pin executable paths and package
versions, avoid shell wrappers, use explicit arguments, restrict remote URLs,
and run servers with the same or a narrower sandbox than OpenMirai. The
`credential_ref` field is a declaration; the default host does not turn it
into a complete secret-manager integration. See [MCP](docs/backend/MCP.md).

## Deployment minimum

- dedicated non-root UID/GID;
- read-only root filesystem;
- narrow writable volumes;
- no Docker socket, host root/home, cloud metadata socket, or unnecessary
  Kubernetes service-account token;
- all Linux capabilities dropped and `no-new-privileges` enabled;
- seccomp/AppArmor/SELinux policy;
- CPU, memory, PID, file-descriptor, and storage limits;
- deny-by-default egress with explicit provider/MCP destinations;
- TLS and stronger identity/rate controls at the gateway;
- one replica until state and coordination are externalized;
- trusted agent specifications only unless execution is moved to isolated
  disposable workers.

## Known security limitations

- shared-secret authentication only;
- no native RBAC, tenancy, quotas, rate limiting, or per-tool authorization;
- no application-level filesystem confinement;
- no kernel sandbox for tool processes;
- graph/agent registries and agent memory are process-local;
- request success can outlive a failed SQLite persistence attempt;
- streaming execution lacks explicit disconnect cancellation;
- static UI and public health/version exposure may reveal metadata;
- audit identity is limited because calls are not tied to individual users.

## Security review checklist for changes

- Does the change add a new file, process, network, model, or secret boundary?
- Is input validated before side effects?
- Can a model or remote document control a command, path, URL, or credential?
- Are time, memory, output-size, recursion, and concurrency bounded?
- Are secrets excluded from errors, state, trace, transcript, and logs?
- Does the feature work under a non-root, read-only, deny-egress deployment?
- Are negative tests included for traversal, injection, SSRF, destructive
  commands, missing authorization, and cancellation?
- Has [Cloud Deployment Readiness](docs/infra/CLOUD-DEPLOYMENT.md) been updated?

## Related documentation

- [System lifecycle](docs/SYSTEM_LIFECYCLE.md)
- [Built-in tool security boundaries](docs/backend/BUILTIN_TOOLS.md)
- [Cloud deployment readiness](docs/infra/CLOUD-DEPLOYMENT.md)
- [Operations runbook](docs/infra/OPERATIONS.md)
- [MCP](docs/backend/MCP.md)
