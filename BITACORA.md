# BITACORA — coordinación entre agentes paralelos

> Registro de sesiones de trabajo en este repo. Protocolo: leer antes de trabajar,
> registrar entrada EN PROGRESO, actualizar al tocar archivos, cerrar con COMPLETADO + commit.
> (La bitácora de Blueprint vive en `blueprint/BITACORA.md`; esta es la de coordinación general.)

---

### [2026-07-31] Sesion: relicense-apache (agente mecanico · fan-out) — cherry-pick a main para release 0.7.0
**Estado**: COMPLETADO
**Proyecto**: OpenMirai Engine — relicenciar de MIT a Apache 2.0
**Objetivo**: Al constituir Data Mirai Inc., el motor open source pasa a Apache 2.0 (patent grant + cláusula defensiva que MIT no da). Autoriza Gabriel (31-jul-2026). Engine v0.7.0.

**Archivos tocados**:
- MODIFICADO `LICENSE` — texto MIT reemplazado por Apache License 2.0 íntegro (copyright appendix: "Copyright 2026 Data Mirai Inc.").
- CREADO `NOTICE` — "OpenMirai" / "Copyright 2026 Data Mirai Inc.".
- MODIFICADO `engine/Cargo.toml`, `cli/Cargo.toml` — `license = "MIT"` → `"Apache-2.0"`. (En main/0.7.0 no existe `agentmirai/Cargo.toml`; ese cambio queda en la branch 0.8.0.)
- MODIFICADO `README.md` — badge, línea de features, tabla comparativa y sección License → Apache-2.0.

**Resultado**: OpenMirai relicenciado MIT→Apache-2.0. `cargo check --workspace` verde (metadata no rompe build). Commit local; Gabriel pushea (safety valve — nada público hasta push).

---

### [2026-07-16] Sesion: security-gate-cors-auth-0.7.0
**Estado**: COMPLETADO
**Proyecto**: OpenMirai Engine (gate de seguridad 0.7.0 — área CORS + AUTH del server)
**Objetivo**: Cerrar el BLOQUEANTE #1 (drive-by RCE): `CorsLayer::permissive()` + server sin auth permitían que una web maliciosa hiciera POST cross-origin a `/api/v1/agents/{id}/execute` o al spawn/send del orchestrator en el server local del usuario.

**Archivos tocados**:
- MODIFICADO `engine/src/server/mod.rs` — (1) CORS pasa de `permissive()` a allowlist vía `AllowOrigin::predicate`: orígenes loopback (localhost/127.x/[::1], cualquier puerto) en toda la API; `Origin: null` (file://) SOLO en `/api/v1/orchestrator/*` (lo necesita la web UI del Claude-Orchestrator abierta por file://). (2) Nuevo middleware `cross_origin_guard`: 403 a requests mutantes (POST/PUT/DELETE/PATCH) con `Origin` presente y no confiable — cierra los "simple requests" sin preflight (p. ej. `stop` sin body); sin Origin (curl/SDK/webhooks) y same-host (UI por LAN) pasan. (3) Comparación de api-key timing-safe (digest SHA-256 vs SHA-256). (4) Warning reforzado a `error!` cuando bindea no-loopback (0.0.0.0 default) sin api-key.
- MODIFICADO `engine/src/server/editor.rs` — el router del editor (`mirai edit`) envuelve el router mergeado con el mismo `cross_origin_guard` (los `/api/edit/undo|redo` son POST sin body → ejecutaban cross-origin sin preflight y escriben el YAML).
- MODIFICADO `engine/src/server/tests.rs` — 7 tests nuevos: preflight rechaza evil.com, permite loopback, `null` acotado a orchestrator, guard 403 a mutación cross-site, guard deja pasar curl/loopback/same-host, api_key_matches exacto, clasificación loopback del host.

**Decisiones tomadas**:
- Opción (a) del gate (CORS allowlist) + guard de Origin como defensa en profundidad; NO se exigió api-key en endpoints (opción b) para no romper la web UI del orchestrator por file:// cuando no hay key.
- Residual documentado: `Origin: null` también lo mandan iframes sandbox (`allow-scripts`) → los endpoints del orchestrator siguen alcanzables por esa vía SI el server corre sin `--api-key`. Con key configurada el auth middleware lo cierra del todo.
- Los UIs legítimos quedan intactos: orchestrator UI (file:// y localhost) → permitida por predicate; mirai edit/serve UI → same-origin, no depende de CORS.

**Resultado**: `cargo check -p openmirai-engine` y `--tests` verdes. Sin commit (regla del gate: NO git desde el agente).

### [2026-07-16] Sesion: fixtures-verdes-release-0.7.0
**Estado**: COMPLETADO
**Proyecto**: OpenMirai Engine (gate de calidad de ejemplos/fixtures pre-release 0.7.0)
**Objetivo**: Que todo `examples/`, `test/` y `agents/` corra verde con `--provider mock` (bare o con `-i` de muestra). Raíz de los fallos: (a) `logic/condition` declaraba `value: Object` (rechazaba escalares) y los fixtures usaban el esquema viejo `config: {field, op, value}`; (b) el provider mock no soportaba media de audio/video (bloqueaba los ejemplos multimodales offline).

**Archivos tocados**:
- MODIFICADO `engine/src/tools/base.rs` — nuevo `FieldType::Any` (serde "any", `matches()` → true) + tests
- MODIFICADO `engine/src/tools/builtin/logic.rs` — condition/switch/merge/loop/human_input usan `Any` donde el valor es de forma libre; `evaluate_condition` acepta el vocabulario de operadores de las edge conditions (`equals`, `greater_than`, …) además de los cortos; tests nuevos (aliases, escalar pasa validación de spec)
- MODIFICADO `engine/src/adapters/mock_llm.rs` — `provider_name()` = "mock" (antes caía al default "unknown") + test
- MODIFICADO `engine/src/llm/media.rs` — provider "mock" soporta los mismos MIME que Gemini (test double universal → ejemplos multimodales corren offline) + test
- MODIFICADO `cli/src/terminal.rs` — `type_map` devuelve `Option` (`Any` ⇒ omitir `type` en el JSON Schema de tools)
- MODIFICADO `examples/conditional-routing.yaml`, `test/test_02/03/08/10`, `agents/support-router.yaml` — esquema nuevo de condition: `field` (valor a evaluar) llega por `data_map`; `operator`+`value` en config
- MODIFICADO `test/test_claude_code.yaml` — data_map usa `trigger.payload.question` (alias `user_input` deprecado)
- MODIFICADO `examples/image-editing.yaml` — header Run con sample y nota de OPENAI_API_KEY
- MODIFICADO `USAGE.md` — fila de `logic/condition` refleja el esquema real

**Decisiones tomadas**:
- Fix de raíz en el TOOL (`FieldType::Any`), no aflojar la validación global: las comparaciones son contra escalares legítimamente.
- El tool de condición acepta los MISMOS nombres de operador que las condiciones de edges (una sola gramática para el usuario); operador desconocido sigue siendo `false` (fail-closed).
- `mock` = proveedor de capacidad universal (media igual a Gemini) para que los ejemplos multimodales sean verificables offline.
- Verificación: `cargo test -p openmirai-engine --lib` 832/0, `cargo test -p openmirai-cli` 13/0, y E2E de los 25 YAML con `target/debug/mirai` (todos validan; todos corren verde bare o con `-i`; voice-synthesis/image-editing llegan hasta su API externa — requieren key real por diseño). OJO: `target/release/mirai` quedó desactualizado; recompilar release antes del gate final.

**Resultado**: 25/25 fixtures validan y corren verde (los 2 de API externa fallan solo por credenciales, con wiring correcto). Sin commit (instrucción explícita: NO git en esta sesión).

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

**M7 lado Engine (10-jul, COMPLETADO)** — picker de proyectos + create_dir:
- `GET /api/v1/orchestrator/projects` → `{projects: [{path, name, source: "session"|"scan", exists}]}`:
  unión de project_dirs del registry + subdirs de primer nivel de `--projects-dirs a:b:c` /
  `MIRAI_PROJECTS_DIRS` (`~` expandido; excluye ocultos, node_modules y no-directorios);
  dedup por path (session gana); orden: sessions primero, luego scan, alfabético por grupo.
- `POST /sessions` acepta `create_dir` (default false): true → mkdir -p antes del spawn;
  false + dir inexistente → 400 claro. HALLAZGO documentado: antes tmux aceptaba `-c` inexistente
  en silencio (exit 0) y claude arrancaba en el dir equivocado — el 400 corrige un bug silencioso.
- CLI: `mirai sessions spawn --create-dir`; `mirai serve --projects-dirs`.
Encima del commit 0e9fc8e del coordinador (acción net) sin tocarlo. Smoke aislado en :3798 (HOME
de prueba, server :4321 intacto). Suite: 803 engine + 13 cli, release limpio.

**M9 lado Engine (10-jul, COMPLETADO)** — selector de carpetas nativo:
- `POST /api/v1/orchestrator/pick-folder` body `{start?}` → abre el diálogo NATIVO del host y
  devuelve la ruta. macOS: `osascript` con `tell System Events to activate` + `choose folder`
  (`default location` si `start` es un dir existente). Linux: `zenity --file-selection --directory`
  (+ `--filename` si `start`); zenity ausente → 501. Otro OS → 501.
- Respuestas: 200 `{path}` (POSIX path sin trailing slash) · 200 `{cancelled: true}` (osascript -128 /
  zenity exit 1 / timeout 120s que mata el proceso) · 409 si ya hay diálogo abierto (AtomicBool +
  RAII guard, un solo diálogo a la vez) · 501 sin soporte · 500 fallo inesperado.
- Runner inyectable (`DialogRunner` trait; real = `TokioRunner` con `tokio::process` + timeout + kill,
  NO bloquea el runtime). Archivos: CREADO `engine/src/sessions/picker.rs`; MODIFICADOS mod.rs
  (sessions), server/{state,orchestrator,mod}.rs.
- Tests: 11 unit (build_command mac/linux/otro, parseo picked/cancel/timeout/501/failed, busy flag
  concurrente con release) + 4 HTTP (200 path, cancel+timeout 200, 501, 409 concurrente real).
- PRUEBA MANUAL REAL en server aislado :3797: el diálogo Finder REAL se abrió (osascript al frente),
  un 2º request devolvió 409 `{error: already open}` en vivo, cerrar/matar el diálogo → 200
  `{cancelled: true}` (verificado dos veces), y osascript `POSIX path of` devuelve `/Users/gabo/`
  con exit 0 (formato exacto que el parser recorta a `/Users/gabo`). El click-through interactivo
  para el 200 `{path}` no se pudo automatizar (accesibilidad de computer-use no concedida en la
  sesión + osascript sin permiso de keystrokes), pero está cubierto por test HTTP con el stdout real.
  Server :4321 de Gabriel intacto (PID verificado antes de matar solo :3797). Suite: 818 engine + 13 cli.
