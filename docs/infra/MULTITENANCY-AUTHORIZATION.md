# Future Multitenancy and Authorization Model

The initial GCP deployment is private and trusted-user only. It is not
multi-tenant: IAP identifies a person, but OpenMirai currently uses one shared
API key and shared process/filesystem/state boundaries. This document defines
what must exist before additional or untrusted user groups are onboarded.

## Isolation unit

Every durable and runtime object must carry a non-null `tenant_id`: users,
agents, graph revisions, runs, events, memories, artifacts, secrets, MCP
connections, workspaces, quotas, and audit records. Access checks occur on the
server using verified identity, never from a client-supplied tenant ID alone.

## Roles

| Role | Capabilities |
|---|---|
| tenant viewer | view approved definitions and redacted results |
| runner | execute approved agent revisions within quota |
| author | create/update definitions without granting new capabilities |
| operator | manage runs, schedules, and tenant operations |
| capability admin | approve tools, providers, MCP, Git, browser, orchestrator |
| platform admin | infrastructure and cross-tenant operations |

Separate authoring an agent from approving dangerous capabilities. A tenant
author must not self-grant shell or production Git write access.

## Enforcement points

- gateway: authenticate, rate limit, reject spoofed identity;
- control plane: authorize every object/action and bind tenant from identity;
- database: tenant-scoped keys/indexes and preferably row-level policies;
- queue: signed/scoped job envelopes produced only by control plane;
- worker: verify tenant/profile/capability claims and use scoped credentials;
- storage: tenant prefixes/buckets and short-lived object references;
- logs/metrics: redact sensitive content and protect cross-tenant queries.

## Credential model

Never place one shared provider, Git, or MCP credential in a worker serving
untrusted tenants. Resolve a tenant-owned credential reference at execution
time and issue the narrowest short-lived access possible. Audit access to the
reference and secret version without logging the value.

## Quotas and budgets

Enforce per tenant and principal:

- requests and concurrent runs;
- queued jobs and execution duration;
- LLM input/output tokens and monetary budget;
- browser/Claude/MCP sessions;
- workspace/artifact/storage bytes;
- outbound destinations and Git repositories;
- retries and failure-rate circuit breakers.

Quota denial must be explicit and must not degrade other tenants.

## Data lifecycle

Retention, export, deletion, legal hold, backup, and restore must operate by
tenant. Default run retention is 30 days, logs 14 days, and audits 90 days, but
tenant contracts may require different values. Deletion from live stores and
eventual expiration from backups must be documented separately.

## Audit

Audit events include tenant, verified principal, role/policy revision, action,
object, capability, outcome, request/run ID, source identity, timestamp, and
deployment version. Audit records are append-only to application users and
queryable only by authorized tenant/platform operators.

## Minimum go-live gates

Before claiming multitenancy, pass cross-tenant object-ID tampering, search,
SSE reconnect, artifact URL, cache, log, queue, workspace, secret, backup
restore, and deletion tests. Before untrusted tenancy, also satisfy the sandbox
conditions in [Execution Security Profiles](EXECUTION-SECURITY-PROFILES.md).
