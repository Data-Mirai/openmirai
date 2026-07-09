# BITACORA — coordinación entre agentes paralelos

> Registro de sesiones de trabajo en este repo. Protocolo: leer antes de trabajar,
> registrar entrada EN PROGRESO, actualizar al tocar archivos, cerrar con COMPLETADO + commit.
> (La bitácora de Blueprint vive en `blueprint/BITACORA.md`; esta es la de coordinación general.)

---

### [2026-07-09] Sesion: prd-013-orquestador-sesiones-m1-m3
**Estado**: EN PROGRESO
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

**Resultado**: (pendiente)
