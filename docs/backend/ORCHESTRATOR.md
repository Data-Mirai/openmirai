# Claude Code Session Orchestrator

The orchestrator is a second execution system inside `mirai serve`. It does
not run AgentSpec graphs. It creates, observes, and controls external Claude
Code processes in tmux and exposes them through `/api/v1/orchestrator/*`.

## Architecture

```text
HTTP / mirai sessions CLI
 -> SessionManager
 -> SessionBackend
 -> tmux session
 -> claude CLI
 -> project directory

SessionManager
 -> JSON registry on disk
 -> pane polling every 2 seconds
 -> status heuristics + activity ring
 -> orchestrator SSE
```

Requirements are Unix, `tmux`, an installed/authenticated `claude` executable,
a writable `~/.openmirai`, and access to every configured project directory.

## Session record

The public record contains `id`, `name`, `project_dir`, `objective`, `status`,
`model`, `effort`, `ultracode`, `tmux_session`, `created_at`, `last_activity`,
`parent_id`, and `permission_mode`. Status is one of `starting`, `working`,
`waiting`, `permission`, `stopped`, or `error`.

The manager infers status from recent terminal output. These are heuristics
over the Claude TUI and can lag or fail when upstream output changes.

## API

All routes use normal server API-key middleware. The SSE events route also
accepts `?api_key=` for browser EventSource compatibility.

| Method and path | Contract |
|---|---|
| `POST /api/v1/orchestrator/sessions` | Spawn from `{name?, project_dir, objective, model?, effort?, ultracode?, parent_id?, no_hooks?, create_dir?, mcp?, permission_mode?}`; returns `201 {id, tmux_session}`. |
| `GET /api/v1/orchestrator/sessions` | List session records. |
| `GET /api/v1/orchestrator/sessions/{id}` | Record plus a 40-line `output_tail`. |
| `POST /api/v1/orchestrator/sessions/{id}/send` | Send non-empty `{text}`; returns 202. |
| `GET /api/v1/orchestrator/sessions/{id}/output?lines=N` | Capture 1–5000 lines, default 100. |
| `POST /api/v1/orchestrator/sessions/{id}/stop` | Stop tmux-backed session. |
| `POST /api/v1/orchestrator/sessions/{id}/restart` | Re-spawn same ID with optional `permission_mode`, `model`, and `effort`. |
| `POST /api/v1/orchestrator/sessions/register` | Register an externally managed visualizer node. |
| `POST /api/v1/orchestrator/sessions/{id}/status` | Update external node with `{status, activity?}`. |
| `POST /api/v1/orchestrator/sessions/{id}/unregister` | Mark external node stopped. |
| `DELETE /api/v1/orchestrator/sessions/{id}` | Alias for external unregister. |
| `POST /api/v1/orchestrator/sessions/{id}/activity` | Record `{tool, action, path?}`; returns 202. |
| `GET /api/v1/orchestrator/sessions/{id}/activity?limit=N` | Chronological in-memory activity, limit 1–500. |
| `GET /api/v1/orchestrator/events` | SSE event stream. |
| `GET /api/v1/orchestrator/projects` | Known session paths plus scanned configured roots. |
| `POST /api/v1/orchestrator/pick-folder` | Open a native host dialog; unsuitable for headless containers. |

## Spawn behavior

`project_dir` and `objective` are required. With `create_dir: false`, a missing
directory is rejected; with true, it is created. `ultracode` prepends that word
to the first prompt. `mcp: true` writes a per-session strict MCP configuration.
Hooks can report Claude Code resource activity back to the server unless
`no_hooks` is set.

`permission_mode: bypass` adds Claude Code's
`--dangerously-skip-permissions`. This removes interactive safety prompts and
must only be allowed in a strongly isolated, trusted worker. Other values use
normal prompting.

## SSE events

The stream starts with an SSE comment and then emits:

- `session_created`: full session record;
- `session_status_changed`: ID, status, last activity;
- `session_output`: newly observed pane lines;
- `session_stopped`: ID;
- `session_activity`: ID, tool, action, path, timestamp.

Slow consumers can lose broadcast events; lagged messages are skipped and
there is no replay cursor.

## Persistence and restart

Registry metadata is written as pretty JSON under `~/.openmirai`. At server
startup the manager loads it and reconciles tmux-backed records against actual
tmux sessions. External nodes are not reconciled as tmux processes.

The registry does not contain terminal history, in-memory activity rings, or a
portable process snapshot. Tmux sessions live on one host; moving the JSON file
to another replica does not migrate them.

## Security and deployment

The API can create directories, start coding agents, send them instructions,
read their output, and optionally bypass permissions. Treat it as privileged
remote process control.

- never expose it without API authentication and gateway policy;
- restrict project roots and filesystem mounts;
- run on a dedicated host/worker identity;
- prevent access to unrelated credentials and repositories;
- do not mount the Docker socket or cloud administrator credentials;
- protect query-string SSE keys from logging;
- disable/avoid the endpoints in the restricted graph-execution image;
- audit every use of bypass mode.

The native folder picker requires a desktop host and may wait around 120
seconds. It should be disabled or treated as unsupported in headless cloud
deployments.

## Operational checks

Verify `tmux -V`, `claude --version`, authentication, project permissions,
writable registry state, server port/API-key injection into hooks, polling,
status transitions, stop/restart, and registry reconciliation after restart.

## Related documentation

- [System lifecycle](../SYSTEM_LIFECYCLE.md)
- [Operations](../infra/OPERATIONS.md)
- [Security](../../SECURITY.md)
- [MCP](MCP.md)
