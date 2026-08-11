# Security Policy

## Supported versions

| Version | Supported |
|---|---|
| 0.7.x | :white_check_mark: |
| < 0.7 | :x: |

## Reporting a vulnerability

Please report security issues using **GitHub Private Vulnerability Reporting**:
[github.com/Data-Mirai/openmirai/security/advisories/new](https://github.com/Data-Mirai/openmirai/security/advisories/new).
This opens a private advisory visible only to maintainers — do not open a public issue for a
suspected vulnerability.

We aim to acknowledge new reports within 5 business days and to provide an initial assessment
(severity, affected versions, expected timeline) within 10 business days.

## Scope

OpenMirai is a Rust engine that executes LLM agent workflows, exposes a sandboxed tool
(`system/sandbox_exec`), handles API credentials for multiple LLM providers, and serves an HTTP
API (default `:3000`). In scope:

- Sandbox escape or unintended host access via `system/sandbox_exec` or any other built-in tool.
- Credential or API-key leakage across provider adapters, logs, or the HTTP API.
- Prompt-injection paths that lead to remote code execution, privilege escalation, or data
  exfiltration beyond the invoking agent's declared scope.
- Authentication/authorization bypass on the HTTP API.

Out of scope: vulnerabilities requiring an already-malicious agent definition run with full
local trust (the engine executes YAML workflows you provide it, by design), denial-of-service
via resource exhaustion on a self-hosted instance, and issues in third-party LLM provider
services themselves.
