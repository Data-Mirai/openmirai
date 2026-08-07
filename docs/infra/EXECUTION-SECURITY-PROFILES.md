# Execution Security Profiles

OpenMirai tools are executable capabilities. This document defines deployable
profiles and prevents a trusted internal deployment from being confused with a
multi-tenant sandbox.

## Profile matrix

| Profile | Intended users | Enabled capabilities | Isolation |
|---|---|---|---|
| `restricted` | applications and lower-trust internal users | LLM, state, deterministic transforms, narrowly approved HTTP/MCP | non-root container, read-only root, no general shell/filesystem |
| `full-trusted` | named trusted operators | shell, filesystem, Git, MCP, browser, Claude/tmux orchestrator | dedicated VM initially; isolated worker pool later |
| `untrusted` | external or mutually untrusted tenants | none until a real sandbox exists | not supported |

The first GCP deployment uses `full-trusted`. Every user is therefore able to
cause high-impact side effects indirectly through an agent. Access must be
rare, attributable, revocable, and reviewed.

## Capability policy

| Family | Main risks | Required controls |
|---|---|---|
| shell/process | arbitrary code, fork bombs, credential theft | command policy, PID/CPU/time limits, no root/capabilities, isolated identity |
| filesystem | overwrite/delete, data exfiltration | narrow mounts, quotas, backups, path policy, no secret mounts visible to tools |
| Git | credential theft, malicious push, source exfiltration | per-repository deploy credentials, protected branches, known hosts, no global personal token |
| MCP | local process execution or remote third-party trust | allowlisted server definitions, pinned packages/binaries, egress policy, schema review |
| browser | prompt injection, arbitrary downloads, session theft | clean profiles, headless isolation, domain policy, download quarantine, no personal sessions |
| Claude/tmux | long-lived autonomous host control | dedicated home/workspace, session limits, audit, explicit kill/reap, dedicated workers later |
| LLM providers | sensitive prompt disclosure, spend abuse | approved models, DLP/redaction, budgets, rate limits, provider-specific keys |

## Identity and authorization

IAP supplies human identity at the edge. OpenMirai currently supplies only a
shared API key, so the application cannot enforce per-user roles or attribute
actions by itself. The ingress must:

- allow only an approved Google group;
- forward verified identity headers only from IAP;
- remove spoofed identity headers from direct clients;
- record principal, method, route, status, request ID, and timestamp;
- require a second approval path for production Git write credentials or other
  especially sensitive capabilities.

The API key remains required, rotates at least every 90 days and after any
suspected exposure, and is never the sole public perimeter.

## Secret handling

The VM service account may access only specifically named Secret Manager secret
versions. Prefer one secret per provider/environment. Tools must not see the
secret mount or environment of unrelated services. Never expose broad cloud
service-account credentials to agent workspaces.

For Git, prefer repository-scoped deploy keys or short-lived GitHub App tokens.
For Claude CLI, follow the vendor-supported non-interactive authentication
method and keep its state in a dedicated encrypted volume; do not copy a
developer's home directory into the image.

## VM and container controls

- private VM, Shielded VM, OS Login, no project-wide SSH keys;
- dedicated service account with minimum permissions;
- automatic security updates with controlled reboot window;
- non-root container, read-only root, dropped capabilities, seccomp, AppArmor;
- no privileged mode, host networking, host PID/IPC, or Docker socket;
- separate persistent mounts for engine data, workspace, and orchestrator home;
- CPU, memory, PID, file-descriptor, execution-time, and disk quotas;
- Cloud NAT/egress firewall and DNS logging;
- vulnerability scanning, SBOM, signed/digest-pinned releases;
- centralized logs and alerts for authorization failures and dangerous actions.

## Prompt-injection assumptions

Content retrieved from Git, web pages, files, MCP servers, or tool output is
untrusted data even when the user is trusted. It can instruct an LLM to use
other tools. Safety therefore cannot depend on prompts alone. Enforce policy at
the tool host, filesystem, network, identity, and operating-system layers.

## Required audit events

Retain for 90 days:

- IAP principal and request metadata;
- agent/graph creation, update, execution, and deletion;
- tool family and target category, with sensitive arguments redacted;
- Git remote and operation, never embedded credentials;
- MCP server identity and transport;
- browser destination domain and download metadata;
- orchestrator create/send/stop lifecycle;
- secret version rotation and access denial;
- deployment, rollback, backup, and restore actions.

## Incident response

On suspected compromise: remove the user from the IAP group, stop new runs,
isolate the VM from egress, snapshot disks for investigation, rotate OpenMirai,
LLM, Git, MCP, and Claude credentials, inspect audit logs, rebuild from a known
image, and restore only validated data. Do not merely restart the same
container.

## Conditions for supporting untrusted users

Do not add untrusted users until work executes in disposable sandboxes with no
shared writable filesystem or long-lived credentials, network deny-by-default,
strong syscall/kernel isolation, per-tenant identity and quotas, artifact
scanning, bounded execution, audited policy enforcement, and destructive
security tests. A container alone is not that sandbox.
