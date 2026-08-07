# GCP Cloud Implementation Plan

This plan turns the architecture decision into ordered, verifiable work. It
does not claim the repository already contains the Docker or Terraform assets.

## Definition of done

The first production milestone is complete when a maintainer can create a new
GCP environment from versioned Terraform, deploy a pinned OpenMirai image
without SSH, authenticate through IAP, execute a smoke-test graph, survive a
container restart without losing recorded runs, restore from backup inside four
hours, and roll back to the preceding image digest.

## Phase 0 — Application prerequisites

1. Add a multi-stage `Dockerfile` for the full trusted execution profile.
2. Add `compose.yaml` with immutable image input, health check, resource limits,
   read-only root filesystem, explicit writable mounts, and structured logging.
3. Add a boot manifest for all graphs/agents that must exist after restart.
4. Add a startup wrapper that validates mounts, required programs, secrets, and
   boot definitions before starting `mirai serve`.
5. Add graceful termination and an application-consistent backup command.
6. Decide how browser automation runs headlessly and pin its browser/runtime
   versions.
7. Add an explicit tool allowlist or denylist in the host. Missing binaries are
   not a security policy.

Exit gate: all tests in [Cloud Acceptance Tests](CLOUD-ACCEPTANCE-TESTS.md)
that do not require GCP pass locally.

## Phase 1 — GCP foundation

Create separate projects for production and non-production when feasible.
Terraform state must live in a versioned, access-controlled Cloud Storage
bucket distinct from application data.

Provision:

- APIs for Compute Engine, Artifact Registry, Secret Manager, Cloud Logging,
  Cloud Monitoring, IAP, Certificate Manager, and required networking;
- custom VPC and subnet in `us-east1`;
- Cloud Router/NAT for private-VM egress;
- firewall rules that permit only load-balancer health checks and IAP/managed
  access paths;
- a dedicated VM service account with no project Editor role;
- Artifact Registry repository with cleanup policy;
- Secret Manager secrets and per-secret IAM bindings;
- DNS, managed certificate, external HTTPS load balancer, and IAP;
- zonal VM and separately managed Persistent Disk;
- snapshot schedule and alerting notification channel;
- log-based metrics, uptime checks, and dashboard.

Use `us-east1` as `var.region`. Select a zone during deployment according to
machine availability and keep `var.zone` independently configurable.

Exit gate: Terraform plan is reviewed, policy checks pass, and a private test
VM can pull the image and read only the named secrets.

## Phase 2 — Identity and delivery

1. Create a Google group for trusted OpenMirai users and grant only the IAP
   secured-resource role needed to access the service.
2. Configure GitHub Actions to authenticate to GCP using Workload Identity
   Federation. Do not store a service-account JSON key in GitHub.
3. Separate CI identities from runtime identities.
4. Build once, scan, generate SBOM/provenance, push, and deploy by digest.
5. Require protected-environment approval for production.
6. Record deployment actor, commit SHA, image digest, Terraform revision, and
   migration/backup outcome.

Exit gate: the pipeline can deploy non-production, cannot modify unrelated GCP
resources, and exposes no long-lived cloud credential.

## Phase 3 — Non-production rehearsal

Run the full acceptance suite, including:

- identity denied/allowed paths;
- each LLM provider with a low-cost model;
- shell, filesystem, Git, MCP, headless browser, and Claude/tmux probes;
- restart, host reboot, disk-full, provider-timeout, and secret-rotation tests;
- backup restore to a fresh VM;
- rollback to the previous image digest;
- 24-hour soak test with representative concurrency.

Measure HTTPS latency from Bogotá and at least one other real Colombian access
network. Compare `us-east1` with `southamerica-east1` only if the observed p95
does not meet the user-experience target.

Exit gate: signed acceptance record and a tested operations runbook.

## Phase 4 — Production launch

1. Freeze infrastructure and application revisions for the launch window.
2. Take a pre-deploy backup.
3. Deploy the exact non-production-tested image digest.
4. Validate health, readiness, identity, provider connectivity, tool profile,
   persistence, logs, and alerts.
5. Limit the initial IAP group and concurrency.
6. Observe error rate, saturation, LLM spend, and execution duration during a
   controlled ramp.

Exit gate: 24 hours stable, backup job successful, and the rollback path still
valid.

## Phase 5 — Scaling preparation

Implement these application capabilities before GKE or multiple workers:

- durable AgentSpec/GraphSpec repository and versioned boot behavior;
- shared PostgreSQL metadata/run store;
- object storage for large artifacts and transcripts;
- durable queue with job IDs, leases, visibility timeout, retries, and DLQ;
- idempotent result commits and duplicate-delivery handling;
- per-run cancellation, deadlines, heartbeats, and worker loss detection;
- capability-aware job routing;
- per-user/tenant authorization, quotas, budgets, and audit attribution;
- worker images split into restricted and full-execution profiles.

Then deploy a stateless control plane and autoscaled worker pools on GKE
Standard. Keep the full-execution pool isolated with its own nodes, identity,
network policy, and admission rules.

## Required repository deliverables

| Path | Purpose |
|---|---|
| `Dockerfile` | Reproducible full-profile image |
| `.dockerignore` | Small, secret-safe build context |
| `compose.yaml` | Single-VM runtime definition |
| `deploy/entrypoint.sh` | Preflight and server lifecycle |
| `deploy/backup.sh` | Consistent backup procedure |
| `infra/terraform/` | Versioned GCP foundation and environment modules |
| `.github/workflows/image.yml` | Build, test, scan, attest, and push |
| `.github/workflows/deploy.yml` | Approved digest-based deployment |
| `docs/infra/` | Runbooks, decisions, acceptance evidence |

These are implementation targets, not existing files.

## Ownership checklist

Before implementation, name an owner and reviewer for application image,
Terraform, security/IAM, operations/on-call, backup/restore, and budget. A single
person may hold multiple roles initially, but every responsibility must be
explicit.
