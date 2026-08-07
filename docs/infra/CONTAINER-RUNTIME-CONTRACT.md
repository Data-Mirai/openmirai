# Container Runtime Contract

This contract defines what a production OpenMirai container may assume and what
the platform must provide. It is the acceptance boundary for the future
`Dockerfile`, Compose file, startup wrapper, and VM configuration.

## Process

The main process must ultimately execute:

```bash
mirai serve --host 0.0.0.0 --port 3000 --db-path /data/openmirai/engine.db
```

PID 1 must forward `SIGTERM`, stop accepting new work, allow a bounded drain,
flush logs, and exit non-zero on failed startup. The platform termination grace
period should initially be 120 seconds. Runs exceeding that window may be
interrupted and must be visible as such.

## Image profiles

The selected initial image is `full-trusted`. A later deployment should also
build `restricted`.

| Capability | `restricted` | `full-trusted` |
|---|---:|---:|
| OpenMirai binary | yes | yes |
| CA certificates | yes | yes |
| shell and `ps` | no | yes |
| Git and SSH client | no | yes |
| Python/Node | only if required | yes, pinned |
| headless browser | no | yes, pinned |
| MCP stdio clients | selected only | yes, allowlisted |
| `tmux` and Claude CLI | no | yes |

Both profiles run as a non-root UID/GID. Neither receives the Docker socket,
host PID namespace, privileged mode, additional Linux capabilities, or cloud
credentials beyond its dedicated workload identity.

## Filesystem

| Container path | Mode | Lifecycle | Contents |
|---|---|---|---|
| `/app` | read-only | image | `mirai` and immutable support files |
| `/data/openmirai` | read-write | durable | SQLite database and owned runtime metadata |
| `/workspace` | read-write | durable or per-run | trusted working trees and generated artifacts |
| `/home/mirai` | read-write | durable for full profile | approved CLI state, Claude auth/config if policy allows |
| `/tmp` | read-write, size-limited | ephemeral | scratch files and browser temp state |
| `/run/secrets` | read-only | runtime | optional file-mounted secret values |

Mount individual directories, never `/`, host `/home`, `/var/run`, or a broad
shared workspace. Filesystem tools can reach everything visible to the process.

The root filesystem must be read-only. Set `HOME=/home/mirai`, a deterministic
working directory, `umask 077`, and explicit limits for `/tmp` and logs.

## Configuration

Non-secret configuration is supplied through a versioned environment file or
Compose variables. Secrets come from Secret Manager at deploy/start time and
must not be placed in the image, repository, Terraform state, command line, or
logs.

Required server settings:

```text
MIRAI_API_KEY
MIRAI_LLM_PROVIDER
MIRAI_LLM_MODEL
```

Provider secrets used by the approved provider set:

```text
OPENAI_API_KEY
ANTHROPIC_API_KEY
GOOGLE_API_KEY
OPENROUTER_API_KEY
```

Only inject secrets for enabled providers. A consumer ChatGPT subscription does
not replace `OPENAI_API_KEY`.

## Network

- listen on container port 3000 over HTTP;
- publish it only to the VM-local reverse-proxy/private interface required by
  the load balancer;
- terminate TLS at the HTTPS load balancer;
- deny inbound access that bypasses IAP;
- permit outbound DNS, provider endpoints, approved MCP endpoints, Git remotes,
  browser destinations, package endpoints required at runtime, and Google APIs;
- block metadata-server access from execution processes unless a narrowly
  scoped cloud call is required.

Because browser, shell, Git, and MCP are enabled, practical egress must start
with an explicit audited allowlist and a documented exception process.

## Health and readiness

`GET /health` proves the process responds; it does not prove SQLite is writable,
provider credentials work, disk space is sufficient, required binaries exist,
or boot definitions were loaded. The startup wrapper must fail before launch
when hard dependencies are absent. Monitoring must separately probe:

- process liveness;
- authenticated API readiness;
- SQLite read/write;
- free disk/inodes;
- provider connectivity through synthetic low-cost calls;
- full-profile executable versions;
- tmux/Claude session capability where enabled.

## Resources

Start with a general-purpose VM/container allocation of 4 vCPU and 16 GiB RAM,
then right-size from observations. Enforce container limits for memory, PIDs,
open files, and scratch space. Reserve headroom for headless browser and Claude
sessions; they can dominate the Rust server's resource use.

Set application concurrency limits below the point where memory or provider
rate limits become unstable. Reject or queue excess work rather than allowing
unbounded process creation.

## Logging

Write structured logs to stdout/stderr. Include request/run ID, agent ID, node
ID, provider, model, duration, status, image version, and error class. Never log
API keys, authorization headers, full prompts by default, secret files, or raw
Claude credentials. Retain operational logs for 14 days and audit events for
90 days.

## Startup contract

Before starting the server, the entrypoint must:

1. verify UID is non-root and root filesystem is read-only;
2. verify ownership, free space, and write access on durable mounts;
3. verify SQLite integrity and apply only explicitly approved migrations;
4. verify enabled-provider secrets exist without printing them;
5. verify required executable versions for the selected profile;
6. validate boot AgentSpecs/GraphSpecs and detect duplicate IDs;
7. emit the image digest/commit/build metadata;
8. start OpenMirai only after all hard checks pass.

## Shutdown contract

On termination, the runtime must stop admitting work, wait up to the configured
drain deadline, mark remaining runs interrupted, checkpoint what the current
engine can persist, stop orchestrated child processes, close SQLite, and exit.
Until the application exposes this complete behavior, upgrades require a
maintenance window and an operator check that no runs are active.
