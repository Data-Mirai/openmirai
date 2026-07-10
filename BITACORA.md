# BITACORA — coordinación entre agentes paralelos

> Registro de sesiones de trabajo en este repo. Protocolo: leer antes de trabajar,
> registrar entrada EN PROGRESO, actualizar al tocar archivos, cerrar con COMPLETADO + commit.
> (La bitácora de Blueprint vive en `blueprint/BITACORA.md`; esta es la de coordinación general.)

---

### [2026-07-09] Sesion: prd-013-orquestador-sesiones-m1-m3
**Estado**: COMPLETADO
**Proyecto**: OpenMirai Engine (PRD-013 — orquestador de sesiones Claude sobre tmux)
**Objetivo**: Implementar M1 (módulo `engine/src/sessions/`: SessionBackend trait + TmuxBackend + SessionManager + registry persistente + detección de estado), M2 (API HTTP `/api/v1/orchestrator/*` + SSE), M3 (CLI `mirai sessions …`). Contrato de API pineado en Claude-Orchestrator/blueprint/prd/backlog/PRD-013/idea.md.

**Archivos tocados**:
- CREADO `engine/src/sessions/mod.rs` — módulo sessions (tipos, trait, re-exports)
- CREADO `engine/src/sessions/backend.rs` — trait SessionBackend + TmuxBackend (shell-out a tmux)
- CREADO `engine/src/sessions/manager.rs` — SessionManager + registry JSON persistente + poll de estado
- CREADO `engine/src/sessions/status.rs` — SessionStatus + heurísticas de detección por capture-pane
- MODIFICADO `engine/src/lib.rs` — registrar módulo `sessions`
- MODIFICADO `engine/src/core/events.rs` — EventType: session_created/session_status_changed/session_output/session_stopped
- CREADO `engine/src/server/orchestrator.rs` — handlers de los 7 endpoints + SSE
- MODIFICADO `engine/src/server/mod.rs` — rutas /api/v1/orchestrator/* + init del manager en serve()
- MODIFICADO `engine/src/server/state.rs` — AppState.orchestrator (Arc<SessionManager>)
- CREADO `cli/src/sessions_cmd.rs` — subcomando `mirai sessions list|spawn|send|output|stop`
- MODIFICADO `cli/src/main.rs` — wiring del subcomando + help

**Decisiones tomadas**:
- Registry persistente en `~/.openmirai/orchestrator_sessions.json` (convención existente: `~/.openmirai/` ya la usa config.toml del engine).
- Eventos del orquestador viajan por el EventEmitter existente (core/events.rs) con variantes nuevas; el SSE del orquestador filtra solo los 4 tipos del contrato.
- El poll (~2s) solo trabaja cuando hay sesiones activas; se arranca en `serve()`, no en tests.
- `claude --effort <level>` SÍ existe en el CLI de esta máquina → se pasa como flag real.
- Contrato afinado con la web UI (M4, ya pusheada en Claude-Orchestrator): SSE con payload plano
  (sin envelope), `session_output` = solo líneas NUEVAS (diff), `session_stopped` = `{id}`,
  y `?api_key=` como auth alternativa del SSE (EventSource no admite headers).
- tmux 3.6b: los comandos de pane (send-keys/capture-pane) exigen target `=name:` (con dos puntos);
  `=name` a secas solo sirve para has-session/kill-session. Hallado en smoke E2E real.

**Resultado**: M1+M2+M3 completos y validados E2E real en esta Mac (spawn → permission → waiting →
send → working → stop; reconciliación tras reinicio del server; SSE con shapes exactos del contrato).
Commits: 4f5dcb1 (M1 módulo sessions), 99c3475 (M2 API HTTP+SSE), 596efec (alineación contrato UI +
fix tmux), + M3 CLI `mirai sessions`. Suite completa en verde: 781 tests engine + 1 CLI, release limpio.
NO se hizo push (regla del repo: el PM pushea). Pendiente: M5 (E2E con sesión orquestadora + skill).

**Extensión (mismo día, COMPLETADO)**: requerimiento nuevo de Gabriel — paridad terminal↔UI:
`mirai sessions watch`, dashboard de terminal en vivo (SSE + fallback polling 2s con retry del
stream, tabla con colores por estado y permission destacada arriba, panel de actividad con los
últimos 10 eventos, alternate screen sin parpadeo con redraw in-place, resize tolerante, salir
con q/Esc/Ctrl-C con guard de restauración incluso en panic).
- CREADO `cli/src/sessions_watch.rs` — el dashboard (ANSI + crossterm, ya era dep del CLI); 9 tests
- MODIFICADO `cli/src/sessions_cmd.rs` — subcomando `watch` + help
- MODIFICADO `cli/src/main.rs` — módulo nuevo
- MODIFICADO `cli/Cargo.toml` — dep `chrono` (ya compilaba en el workspace vía engine)
Validado en vivo bajo pty (`script`): transición polling→live, tabla con sesiones reales (incluidas
las del E2E M5 de otro agente corriendo en paralelo), eventos SSE en el panel, restauración del
terminal al salir. NOTA para agentes paralelos: hay un `mirai serve` en :3777 con sesiones del E2E
M5 — no matarlo ni borrar `~/.openmirai/orchestrator_sessions.json`.

**M6 lado Engine (10-jul, COMPLETADO)** — canvas espacial vivo, datos nuevos:
1. `parent_id` nullable en sesión (registry + GET + POST spawn); TmuxBackend inyecta
   `MIRAI_SESSION_ID`/`MIRAI_PORT` con `tmux new-session -e`; el CLI spawn auto-manda parent_id
   desde MIRAI_SESSION_ID (override `--parent`).
2. Activity: POST/GET `/sessions/{id}/activity` (ring en memoria, 200/sesión, no persiste) +
   SSE `session_activity` → `{id, tool, action, path, ts}`.
3. Hooks reales de Claude Code: el engine escribe `~/.openmirai/hooks/report-activity.py`
   (python3 stdlib, exit 0 SIEMPRE) + `<id>-settings.json` por sesión (PostToolUse, formato
   verificado contra ~/.claude/settings.json real) y lanza `claude --settings <archivo>`;
   `--no-hooks` para optar fuera (API + CLI).
4. `/ui` estáticos desde `--ui-dir`/`MIRAI_UI_DIR` (sin auth, 404 claro si no está configurado,
   anti-traversal, content-types básicos).
5. `watch`: `session_activity` en el panel ACTIVITY (hora + id corto + action→path abreviado).
Archivos: CREADO `engine/src/sessions/hooks.rs`; MODIFICADOS backend.rs/manager.rs/mod.rs (sessions),
events.rs, server/{mod,state,orchestrator}.rs, cli/{main,sessions_cmd,sessions_watch}.rs.
E2E real validado en server aislado (:3799, HOME en /tmp): hook manual + hook REAL disparado por
claude escribiendo hola.txt (permission → aprobar → session_activity por SSE en <2s + GET + watch);
auto-parent verificado; /ui sirviendo. Server :4321 de Gabriel intacto (PID verificado antes de
matar solo el :3799). Suite completa: 796 engine + 13 cli, release limpio.
