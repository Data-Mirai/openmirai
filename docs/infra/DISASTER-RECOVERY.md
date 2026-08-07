# Disaster Recovery Plan

This plan implements the initial service targets: RPO 24 hours and RTO 4 hours.
The first deployment is single-zone and does not promise automatic failover.

## Protected assets

| Asset | Protection | Target |
|---|---|---|
| SQLite run database | daily verified logical backup plus disk snapshots | RPO 24h |
| boot definitions/config | Git revision | loss limited to unpushed changes |
| container image | immutable Artifact Registry digest | retain known-good releases |
| Terraform/state | versioned Git plus protected versioned GCS state | reconstruct environment |
| provider/runtime secrets | Secret Manager versions and recovery policy | rotate when compromised |
| workspaces | Git/object backup based on declared class | per-workspace policy |
| orchestrator home/registry | encrypted backup if operationally required | documented separately |
| logs/audit | Cloud Logging sinks/retention | 14d/90d defaults |

## Failure scenarios

### Container or application failure

Restart the pinned image, validate boot and SQLite, and run smoke tests. If the
same version repeatedly fails, roll back to the prior compatible digest.

### VM or zonal failure

Provision a new VM in another zone of `us-east1`, create/restore a disk from the
latest verified backup or snapshot, load the pinned image/config, validate
privately, then update the backend. This is operator-driven within four hours.

### Regional failure

Provision in the pre-approved secondary region, initially
`southamerica-east1` or another tested US region, restore exported backups,
recreate secrets/config as allowed, validate provider and identity paths, and
change traffic/DNS. Meeting a four-hour RTO for a regional event requires a
quarterly tested Terraform path and backup copies accessible outside the failed
region.

### Data corruption

Stop writers, preserve the corrupt disk, determine the last known-good verified
backup, restore to a new disk, run integrity/application checks, and switch only
after approval. Do not overwrite evidence.

### Credential compromise

Block access/egress, revoke and rotate affected credentials, inspect Secret
Manager/IAP/audit logs, rebuild from trusted artifacts, and restore only
validated state. A backup containing compromised credentials/config is not
automatically safe.

### Provider outage

Disable or reroute only if the agent/model policy permits equivalent behavior.
OpenAI, Anthropic, Gemini, and OpenRouter are not automatically interchangeable:
model semantics, context limits, structured output, cost, and data handling can
differ. Preserve explicit provider choice where correctness matters.

## Disaster declaration and roles

The incident commander declares DR, chooses the recovery point, assigns an
infrastructure operator and application validator, controls communication, and
records timestamps. Only the incident commander authorizes traffic cutover and
return to primary.

## Recovery sequence

```text
detect -> declare -> stop writes -> preserve evidence -> choose recovery point
-> provision clean environment -> restore -> validate privately -> cut over
-> monitor -> reconcile external effects -> retrospective
```

Validation includes IAP/API-key access, image identity, boot definitions,
SQLite integrity, representative historical run query, deterministic graph,
one enabled-provider call, full-profile tool probe, logging/alerts, and a fresh
backup.

## Failback

Do not fail back during initial stabilization. Rebuild the primary from
Terraform, reconcile data and external side effects, take a new verified
backup, rehearse the switch, then change traffic in a controlled window. Never
attach one writable zonal disk to competing active writers.

## Exercises

- monthly: verify backup age, checksums, and alert path;
- quarterly: restore SQLite/config to a clean VM and measure RPO/RTO;
- twice yearly: simulate zone loss and rebuild in another zone;
- yearly: regional recovery tabletop plus technical proof of backup access;
- after every material storage/IAM/deployment change: repeat affected tests.

Every exercise records timings, missing permissions, manual steps, data gaps,
failed tests, owners, and due dates. A backup that has never been restored is
not accepted as recoverable.

## Limits of the initial plan

The single-VM design can lose in-flight runs and process-local registrations or
memory. It cannot provide zero-downtime failover. Achieving lower RPO/RTO and
higher availability requires shared durable state, a control plane, queue,
idempotent workers, and multi-zone infrastructure described in
[Control-Plane and Worker Protocol](CONTROL-PLANE-WORKER-PROTOCOL.md).
