# Cloud Acceptance Tests

These tests are the release gate for the first GCP deployment. Save commands,
timestamps, resource identifiers, image digest, and redacted results as release
evidence.

## Image and runtime

- image builds from a clean checkout with locked dependencies;
- image runs as non-root with read-only root filesystem;
- image contains the expected `mirai`, shell, `ps`, Git, Python/Node, browser,
  tmux, Claude CLI, and approved MCP binaries at recorded versions;
- no provider/cloud secret exists in image layers, SBOM, or build log;
- container starts with only documented mounts and environment;
- SIGTERM produces bounded graceful shutdown;
- CPU, memory, PID, file-descriptor, and scratch limits are enforced;
- container cannot access Docker socket, host root, or unauthorized metadata.

## Network and identity

- VM has no path that exposes port 3000 directly to the Internet;
- HTTPS certificate and redirect policy are correct;
- user outside the IAP group is denied;
- approved user reaches the service;
- missing/incorrect `X-API-Key` is denied on protected routes;
- spoofed IAP identity header is ignored/removed;
- health checker reaches only the intended health endpoint;
- egress to each approved provider/MCP/Git/browser target works;
- representative disallowed egress is blocked and logged.

## Functional lifecycle

- register and execute a deterministic graph;
- execute an LLM graph through OpenAI, Anthropic, Gemini, and OpenRouter;
- verify malformed AgentSpec and unknown tools fail clearly;
- verify sync and streaming endpoints with documented event semantics;
- verify run ID, graph/node state, transcript/trace, and final record;
- confirm scheduler limitations documented for the deployed version;
- confirm client disconnect/cancellation behavior matches documentation.

Provider tests use low-cost models and non-sensitive synthetic prompts. A
ChatGPT web subscription is not used as an OpenAI API credential.

## Full trusted execution profile

- shell command executes in the intended UID/workspace and hits time/PID limits;
- filesystem write succeeds only under approved mounts;
- destructive/path-escape attempts are denied by the deployed policy;
- Git clone/read and an approved write flow use repository-scoped credentials;
- MCP stdio and remote transports work only for approved server definitions;
- headless browser reaches an approved page, stores downloads in quarantine,
  and uses no persistent personal session;
- Claude/tmux session create, message, inspect, stop, and orphan reconciliation
  work across container/VM restart according to documented semantics.

## Persistence and restart

- completed run remains queryable after container restart and VM reboot;
- boot-manifest graphs/agents are reloaded idempotently;
- non-durable HTTP-only registrations are demonstrably lost and called out;
- agent execution memory loss matches the documented current behavior;
- workspace persistence/recreation follows its declared policy;
- SQLite write fails safely on full disk and alerts fire;
- no traffic is admitted when required boot definitions fail.

## Backup and disaster recovery

- daily backup passes integrity and checksum validation;
- failed backup alerts within the defined window;
- restore occurs to a clean VM/disk from Terraform;
- restored service passes authenticated smoke and data checks;
- actual RPO is no more than 24 hours and RTO no more than 4 hours;
- old and new resources are distinguishable and forensic source data preserved.

## Delivery and rollback

- GitHub authenticates with WIF and has no stored GCP key;
- image is scanned, has SBOM/provenance, and deploys by digest;
- production approval is enforced;
- deployment record includes all required revisions and identities;
- failed readiness automatically stops promotion;
- previous known-good digest rolls back successfully against compatible state;
- incompatible migration follows tested restore-to-new-disk procedure.

## Observability and limits

- dashboards show traffic, errors, latency, runs, providers, saturation, disk,
  backup age, processes, and deployed version;
- synthetic checks and every launch alert are exercised;
- logs correlate request/run/node without leaking secrets or full prompts;
- 14/90/30-day retention policies are configured or gaps explicitly tracked;
- load test establishes concurrency limit and demonstrates overload rejection;
- 24-hour soak has no growing process/file/socket leak.

## Region validation

From representative Colombian networks, measure 24–48 hours of median/p95 RTT
and HTTPS transaction latency to the selected `us-east1` deployment. If user
experience is unacceptable, run the same test against `southamerica-east1` and
record both latency and dated price estimates before changing the decision.

## Sign-off record

The release evidence must name the test environment, date, image digest,
Terraform revision, tester, approver, failed/waived cases, risk owner, expiry of
each waiver, and links to monitoring and backup/restore evidence.
