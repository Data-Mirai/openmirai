# Data Mirai Engine — Escenarios GWT

## FEAT-001: MVP Features

### Journey: Streaming de ejecucion

TEST-001: Streaming happy path — ejecucion completa
  Given: Agente desplegado con 3 bloques (trigger/manual → ai/llm_call → data/db_write)
  When: Usuario ejecuta el agente y se conecta al stream SSE
  Then: Recibe eventos en orden: session.started → block.started → block.completed → ... → session.completed

TEST-002: Streaming — tokens LLM en tiempo real
  Given: Agente con bloque ai/llm_call en ejecucion
  When: El LLM genera respuesta
  Then: Se reciben eventos llm.token con cada token antes de block.completed
  And: llm.completed incluye tokens_used y cost

TEST-003: Streaming — error durante ejecucion
  Given: Agente en ejecucion, bloque falla despues de agotar retries
  When: block.error se emite
  Then: Se recibe block.error con node_id y error_message
  And: session.failed se emite como ultimo evento

TEST-004: Streaming — reconexion
  Given: Usuario conectado al stream SSE, pierde conexion
  When: Se reconecta con Last-Event-ID
  Then: Recibe eventos desde el ultimo evento recibido

### Journey: Checkpointing

TEST-005: Checkpoint se crea en cada paso
  Given: Agente con 4 bloques en ejecucion
  When: Cada bloque completa
  Then: Se crea checkpoint con step_number incremental, state_snapshot completo y cursor_position

TEST-006: Rewind a paso anterior
  Given: Session completada con 5 checkpoints
  When: Usuario solicita rewind a step_number=3
  Then: Se crea nueva session con SharedState restaurado del checkpoint 3

TEST-007: Fork desde checkpoint
  Given: Session fallida con checkpoint en step 4
  When: Usuario solicita fork desde step 4 con state modificado
  Then: Se crea nueva session con state del checkpoint + modificaciones

TEST-008: Checkpoints persisten ante crash
  Given: Session en ejecucion con 3 checkpoints en DB
  When: El proceso se reinicia
  Then: Los checkpoints siguen disponibles y el usuario puede fork desde el ultimo

### Journey: Human-in-the-loop — Nodo declarativo

TEST-009: Nodo human_input pausa ejecucion
  Given: Grafo con bloque logic/human_input
  When: El cursor llega al nodo
  Then: Session pasa a interrupted, se crea checkpoint + interrupt con prompt

TEST-010: Usuario responde al interrupt
  Given: Session interrupted por nodo human_input
  When: Usuario envia respuesta via API
  Then: Interrupt pasa a RESOLVED, respuesta se inyecta en SharedState, session reanuda

TEST-011: Human_input con opciones predefinidas
  Given: Nodo human_input con options: ["Aprobar", "Rechazar", "Escalar"]
  When: El cursor llega al nodo
  Then: El interrupt incluye las opciones, UI las muestra como botones

TEST-012: Human_input con timeout
  Given: Nodo human_input con timeout_minutes: 30
  When: Pasan 30 minutos sin respuesta
  Then: Interrupt pasa a EXPIRED, session pasa a timeout

TEST-013: Human_input sin timeout
  Given: Nodo human_input con timeout_minutes: null
  When: Pasan 24 horas sin respuesta
  Then: Session sigue en interrupted, sin consumo de memoria/CPU

### Journey: Session Control

TEST-014: Pausar session en vivo
  Given: Session en estado running
  When: Usuario envia POST /api/sessions/{id}/pause
  Then: Bloque actual termina, checkpoint se crea, session pasa a interrupted (session_control)

TEST-015: Inspeccionar y modificar state durante pausa
  Given: Session pausada
  When: Usuario consulta GET /state, luego PATCH /state con modificaciones
  Then: SharedState se actualiza, nuevo checkpoint se crea con state modificado

TEST-016: Resumir session despues de pausa/modificacion
  Given: Session pausada, usuario ya modifico state
  When: Usuario envia POST /api/sessions/{id}/resume
  Then: Session reanuda desde checkpoint con state modificado

### Journey: Hooks/Callbacks

TEST-017: Hook pre_block_exec se ejecuta antes del bloque
  Given: Agente con hook pre_block_exec configurado
  When: Cursor llega a un bloque
  Then: Hook se ejecuta ANTES del bloque, evento hook.fired en SSE

TEST-018: Hook pre_block_exec puede bloquear ejecucion
  Given: Hook que retorna "abort" cuando input invalido
  When: Cursor llega al bloque con input invalido
  Then: Bloque NO ejecuta, session falla con razon del hook

TEST-019: Hook pre_block_exec puede modificar inputs
  Given: Hook que sanitiza campo "prompt"
  When: Cursor llega a bloque ai/llm_call con prompt con HTML
  Then: Bloque recibe prompt sanitizado

TEST-020: Hook post_block_exec puede modificar outputs
  Given: Hook post_block_exec que agrega timestamp
  When: Bloque completa
  Then: Output en SharedState incluye timestamp del hook

TEST-021: Hook pre_llm_call puede servir cache
  Given: Hook pre_llm_call con cache local
  When: Prompt ya tiene respuesta cacheada
  Then: Hook retorna "skip" con respuesta, no se llama al LLM

TEST-022: Hook con filtro por block_type
  Given: Hook con filter: "ai/*"
  When: Cursor llega a logic/condition
  Then: Hook NO se ejecuta
  When: Cursor llega a ai/llm_call
  Then: Hook SI se ejecuta

TEST-023: Hook on_error
  Given: Hook on_error configurado
  When: Bloque falla despues de retries
  Then: Hook ejecuta. Si retorna "retry", reintenta. Si "abort", session falla

### Journey: Versionamiento de agentes

TEST-024: Publicar crea snapshot inmutable
  Given: Agente con grafo en estado draft
  When: Usuario publica
  Then: Se crea agent_snapshot con version auto-incremental, status ACTIVE

TEST-025: Sessions usan snapshot activo
  Given: Agente con snapshot v3 ACTIVE
  When: Se ejecuta agente
  Then: Session usa graph_def del snapshot v3, ediciones al draft no afectan

TEST-026: Rollback a version anterior
  Given: Snapshots v1(DEPRECATED), v2(DEPRECATED), v3(ACTIVE)
  When: Rollback a v1
  Then: v3 pasa a ROLLED_BACK, v1 pasa a ACTIVE

TEST-027: Listar historial de versiones
  Given: Agente con 5 snapshots
  When: GET /api/agents/{id}/versions
  Then: Lista ordenada por version desc con status y created_at

### Journey: Vault de credenciales

TEST-028: Crear vault y agregar credencial
  Given: No existe vault
  When: Crea vault + credencial tipo api_key
  Then: Credencial almacenada cifrada, valor nunca en GET

TEST-029: Agente accede a credencial via vault
  Given: Resource con credential_ref configurado
  When: Agente ejecuta bloque que usa ese resource
  Then: ExecutionContext tiene credencial descifrada disponible

TEST-030: Credencial OAuth con refresh automatico
  Given: Credencial oauth con token que expira
  When: Token expira durante ejecucion
  Then: Sistema usa refresh_token, actualiza credencial en DB

TEST-031: Credencial expirada sin refresh
  Given: Credencial OAuth con refresh_token expirado
  When: Agente intenta usar credencial
  Then: Pasa a EXPIRED, bloque falla con error descriptivo

TEST-032: Eliminar credencial en uso
  Given: Credencial referenciada por 2 agentes
  When: Usuario intenta eliminar
  Then: Advertencia "usada por 2 agentes". Si confirma, pasa a REVOKED

### Journey: Telemetria

TEST-033: Span por cada bloque
  Given: Agente con 4 bloques
  When: Cada bloque ejecuta
  Then: Se crea execution_span con duracion, status, node_id

TEST-034: Span LLM con tokens y costo
  Given: Bloque ai/llm_call ejecuta
  When: LLM responde
  Then: Span incluye tokens_input, tokens_output, cost_estimate

TEST-035: Spans exportables OpenTelemetry
  Given: Telemetria configurada con endpoint OTLP
  When: Session completa
  Then: Spans se exportan en formato OTLP, trace_id = session_id

TEST-036: Dashboard metricas por agente
  Given: Agente con 50 sessions
  When: GET /api/agents/{id}/metrics
  Then: total_sessions, avg_duration, total_tokens, total_cost, error_rate

TEST-037: Telemetria no afecta performance
  Given: Agente con telemetria habilitada
  When: Se compara con ejecucion sin telemetria
  Then: Overhead < 5%, spans escritos async

### Journey: AI Assistant

TEST-038: Generar grafo desde descripcion (agente Data Mirai)
  Given: Agente graph-builder desplegado con acceso a ToolRegistry
  When: Usuario describe "Agente que reciba webhook con PDF, transcriba, extraiga action items, guarde en DB"
  Then: Genera GraphDef valido con tools existentes, edges conectados, config pre-llenada

TEST-039: Grafo generado es editable
  Given: Grafo generado por AI assistant
  When: Se carga en editor visual
  Then: Todos los nodos editables, movibles, conectables. Pasa validacion

TEST-040: Templates predefinidos
  Given: Templates disponibles
  When: Usuario selecciona "support agent"
  Then: Se genera grafo base con flujo de soporte, editable

TEST-041: AI assistant usa MCP y recursos disponibles
  Given: graph-builder con acceso a ToolRegistry y MCP servers
  When: Usuario describe flujo que requiere GitHub
  Then: Detecta MCP "github", incluye nodos MCP, configura credential_refs

### Journey: MCP Client

TEST-042: Registrar MCP server
  Given: MCP server "playwright" disponible
  When: Agrega a mcp_servers
  Then: Conexion establecida, tools descubiertos automaticamente

TEST-043: Tools MCP en catalogo del editor
  Given: MCP server "github" conectado
  When: Usuario abre catalogo
  Then: Categoria "MCP: github" con tools disponibles y schemas

TEST-044: Ejecutar tool MCP desde bloque
  Given: Grafo con nodo que usa tool MCP
  When: Cursor llega al nodo
  Then: Invoca via JSON-RPC, respuesta en SharedState, span registrado

TEST-045: MCP server HTTP con autenticacion
  Given: MCP server remoto con credential_ref del vault
  When: Agente ejecuta tool
  Then: Credencial descifrada del vault, usada para autenticar

TEST-046: MCP server no disponible
  Given: MCP server configurado pero no corriendo
  When: Agente intenta usar tool
  Then: Error "MCP server 'playwright' no disponible", retry aplica

TEST-047: Desconexion MCP durante ejecucion
  Given: MCP server conectado, agente en ejecucion
  When: MCP server se desconecta mid-call
  Then: Bloque falla, retry policy aplica con reconexion

### Edge cases cross-feature

TEST-048: Checkpoint + Interrupt — rewind a punto de interrupt
  Given: Session interrumpida en step 5, respondida, ejecuto hasta step 8
  When: Rewind a step 5
  Then: Nueva session en step 5, nuevo interrupt (debe responder de nuevo)

TEST-049: Hook + Streaming — eventos de hook en stream
  Given: Agente con hooks, usuario en SSE
  When: Hook se dispara
  Then: Evento hook.fired aparece en stream

TEST-050: Vault + MCP — credencial para MCP server
  Given: MCP server con credential_ref al vault
  When: Agente conecta al MCP server
  Then: Credencial descifrada para autenticar. Si se renueva (OAuth), reconecta

## FEAT-018: Real-time Experience — WebSocket + Live UI

### Journey: Conexión WebSocket

TEST-051: Conexión inicial al montar la app
  Given: Usuario abre la aplicación local
  When: Layout se monta
  Then: WebSocketManager establece conexión a ws://localhost:8000/ws
  And: Indicador de conexión muestra estado "connected" (dot verde con glow)

TEST-052: Reconexión automática tras desconexión
  Given: WebSocket conectado, usuario navegando
  When: Conexión se pierde (server reinicia, red inestable)
  Then: Indicador cambia a amarillo "Reconectando..."
  And: Retry automático con backoff exponencial (1s → 2s → 4s → 8s → 16s)
  And: Al reconectar, re-suscribe todos los canales previos automáticamente

TEST-053: Max retries agotados
  Given: WebSocket desconectado, 5 reintentos fallidos
  When: Último retry falla
  Then: Indicador rojo "Desconectado"
  And: Toast con botón "Reconectar" para retry manual
  And: No más intentos automáticos hasta click del usuario

TEST-054: Heartbeat mantiene conexión viva
  Given: WebSocket conectado, sin actividad
  When: Server envía ping cada 30s
  Then: Cliente responde pong
  And: Sin respuesta en 60s, server cierra conexión

TEST-055: Reconexión preserva suscripciones
  Given: Usuario en página de environment con canales suscritos (resources, agents, sessions)
  When: WebSocket se desconecta y reconecta
  Then: Todos los canales se re-suscriben automáticamente
  And: Datos se refrescan una vez al reconectar (re-fetch)

### Journey: Suscripción a canales y eventos CRUD

TEST-056: Suscripción al entrar a una página
  Given: WebSocket conectado
  When: Usuario navega a página de environment
  Then: Se suscribe a canales: resources:{env_id}, agents:{env_id}
  And: Al salir de la página, se desuscribe de esos canales

TEST-057: Evento de creación refleja nuevo recurso en vivo
  Given: Usuario en página de environment viendo lista de recursos
  When: Se crea un recurso (via otra tab, API directa, o acción en la misma UI)
  Then: Evento resource.created llega por WebSocket
  And: Lista se actualiza automáticamente (re-fetch) sin recargar página
  And: Nuevo recurso aparece con animación slide-in

TEST-058: Evento de eliminación remueve entidad en vivo
  Given: Usuario viendo lista de agentes en environment
  When: Un agente es eliminado
  Then: Evento agent.deleted llega por WebSocket
  And: Card del agente desaparece con animación fade-out
  And: Contadores se actualizan

TEST-059: Evento de actualización refleja cambios en vivo
  Given: Usuario viendo detalle de un agente
  When: Nombre o configuración del agente cambia
  Then: Evento agent.updated llega por WebSocket
  And: Datos en pantalla se actualizan sin recarga

TEST-060: Múltiples canales simultáneos
  Given: Usuario en página de environment (canales: resources, agents, sessions)
  When: Llegan eventos en canales distintos casi simultáneamente
  Then: Cada evento actualiza su sección correspondiente independientemente
  And: Sin conflictos ni race conditions

### Journey: Ejecución en vivo via WebSocket

TEST-061: Ejecución de agente streameada por WebSocket
  Given: Usuario ejecuta agente desde UI
  When: Session se crea y empieza a correr
  Then: Eventos block.started, block.completed llegan por canal session:{id}:events
  And: Nodos en el canvas cambian de estado con animaciones (queued → running → completed)

TEST-062: Partículas en edges durante ejecución
  Given: Nodo A completa, edge A→B activo
  When: Datos fluyen al nodo B
  Then: Partículas brillantes viajan por el edge de A hacia B
  And: Edge cambia de gris a color activo

TEST-063: Contadores de tokens y costo en vivo
  Given: Bloque ai/llm_call ejecutándose
  When: Eventos block.completed llegan con tokens_used y cost
  Then: Contadores de tokens y costo se actualizan con animación count-up (spring physics)
  And: Flash verde breve en el contador al actualizarse

TEST-064: Timer de duración ticking en vivo
  Given: Session en estado running
  When: Cada 100ms
  Then: Timer de duración incrementa visualmente con colon parpadeante
  And: Cambia de color verde (< 10s) → amarillo (10-60s) → rojo (> 60s)

TEST-065: Feed de eventos con entrada escalonada
  Given: Session ejecutándose, tab de eventos abierta
  When: Eventos llegan del WebSocket
  Then: Cada evento aparece con animación slide-in desde izquierda (x: -20→0)
  And: Stagger de 100ms entre eventos consecutivos
  And: Auto-scroll al bottom (pausa si usuario scrolleó arriba manualmente)

### Journey: Estados de nodo extendidos

TEST-066: Nodo en estado queued
  Given: Grafo con 5 nodos, nodo 1 ejecutándose
  When: Canvas muestra nodos 2-5
  Then: Nodos pendientes muestran border dashed con pulse sutil (opacity 0.4↔1.0)

TEST-067: Nodo en estado running con glow
  Given: Cursor llega al nodo
  When: Evento block.started recibido
  Then: Nodo muestra border gradient violeta→cyan con glow expandiéndose (2s cycle)
  And: Badge de estado cambia a "running" con animación

TEST-068: Nodo completado con flash
  Given: Nodo en estado running
  When: Evento block.completed recibido
  Then: Flash verde en border (0.6s, una vez)
  And: Badge checkmark aparece con pop animation (scale 0→1.3→1)
  And: Glow se desvanece

TEST-069: Nodo fallido con shake
  Given: Nodo en estado running
  When: Evento block.error recibido
  Then: Shake horizontal (±3px, 0.4s)
  And: Glow rojo pulsante
  And: Badge X rojo con pop animation

TEST-070: Nodo waiting-for-input
  Given: Nodo logic/human_input alcanzado
  When: Evento session.interrupted recibido
  Then: Nodo muestra border cyan, icono pausa
  And: Pulse lento (3s cycle) indicando espera
  And: Panel de interrupt aparece con slide-in

TEST-071: Nodo retrying
  Given: Bloque falla, retry policy permite reintento
  When: Bloque se re-ejecuta
  Then: Nodo muestra border amarillo con spinner rotando (0.8s)
  And: Badge muestra retry count

### Journey: Feedback visual global

TEST-072: Skeleton loading en carga inicial
  Given: Usuario navega a una página nueva
  When: Datos se están cargando (antes del primer fetch)
  Then: Se muestra skeleton con shimmer (gradiente moviéndose)
  And: Skeleton replica la forma del contenido final (cards, listas, badges)
  And: Al cargar datos, skeleton fade-out → contenido fade-in (0.4s)

TEST-073: Toast para eventos de background
  Given: Usuario en página de universes
  When: Evento de creación/eliminación/error llega por WebSocket
  Then: Toast aparece en esquina con slide-in (0.3s)
  And: Auto-dismiss después de 4s
  And: Múltiples toasts se stackean verticalmente con 8px gap

TEST-074: Panel slide con animación
  Given: Usuario clickea un nodo en el editor
  When: Panel de configuración debe aparecer
  Then: Panel entra con slide desde derecha + fade (300-400ms)
  And: Al cerrar, sale con slide + fade inverso

TEST-075: Transiciones de estado en badges
  Given: Badge mostrando estado "running"
  When: Estado cambia a "completed"
  Then: Color transiciona suavemente (azul→verde, 300ms)
  And: Texto cambia con cross-fade

### Journey: EventBus — emisión desde backend

TEST-076: Toda mutación CRUD emite evento
  Given: Backend corriendo con EventBus activo
  When: Se ejecuta cualquier operación de escritura (POST, PUT, PATCH, DELETE) en cualquier endpoint
  Then: EventBus emite evento en el canal correspondiente
  And: Payload incluye entity type, action, id, y datos relevantes

TEST-077: Crear universe emite en canal correcto
  Given: WebSocket conectado, suscrito a canal "universes"
  When: POST /api/universes crea universe + environment auto
  Then: Se reciben 2 eventos: universe.created y environment.created
  And: Ambos con payload conteniendo id y metadata

TEST-078: Eliminar universe emite cascade
  Given: Universe con 2 environments, 3 agentes
  When: DELETE /api/universes/{id}
  Then: Evento universe.deleted llega por canal "universes"
  And: Frontend remueve todo lo relacionado

TEST-079: Provisionar environment emite recursos creados
  Given: Suscrito a canal resources:{env_id}
  When: POST /api/environments/{env_id}/provision
  Then: 3 eventos resource.created (db, vector, storage) llegan secuencialmente

TEST-080: Operaciones LLM provider emiten eventos
  Given: Suscrito a canal "providers"
  When: CRUD de LLM provider (create, update, delete)
  Then: Evento correspondiente llega por WebSocket
  And: Settings page se actualiza sin recargar

### Edge cases

TEST-081: Desconexión durante ejecución de agente
  Given: Agente ejecutándose, WebSocket se desconecta
  When: WebSocket reconecta
  Then: Re-suscribe canal session:{id}:events
  And: Hace re-fetch del estado de la sesión para sincronizar
  And: Canvas muestra estado correcto de cada nodo

TEST-082: Múltiples tabs abiertas
  Given: Dos tabs del browser con la app abierta
  When: Acción en tab A crea un recurso
  Then: Tab B recibe evento y actualiza su vista automáticamente

TEST-083: Evento llega antes de que página termine de cargar
  Given: Usuario navega a página, fetch inicial en progreso
  When: Evento WebSocket llega antes de que fetch complete
  Then: Evento se bufferea
  And: Después del fetch inicial, se aplica sin duplicar datos

TEST-084: Alto volumen de eventos durante ejecución
  Given: Agente con 20 nodos ejecutándose rápido
  When: Eventos llegan en ráfaga (< 100ms entre ellos)
  Then: Animaciones se ejecutan sin jank (60fps)
  And: Feed de eventos mantiene stagger sin acumulación

TEST-085: Server reinicia — cliente recupera
  Given: App abierta, server se reinicia
  When: WebSocket detecta cierre
  Then: Indicador amarillo, reintentos automáticos
  And: Al reconectar, re-fetch de todos los datos visibles
  And: Estado visual correcto sin intervención del usuario

## FEAT-027: Energy Metering + Execution Lifecycle

### Journey: Energy Metering — Managed Session

TEST-086: Energy event por LLM call
  Given: Agente managed con nodo ai/llm_call configurado (Claude Sonnet)
  When: Session ejecuta y nodo LLM completa con 500 tokens in + 200 tokens out
  Then: Se crea energy_event con operation_type=LLM_CALL, provider=anthropic, cost_category=EXTERNAL_SERVICE
  And: energy_charged calculado según energy_rate activo para claude-sonnet-*

TEST-087: Energy event por MCP call
  Given: Agente con nodo MCP (WhatsApp send)
  When: Nodo MCP ejecuta exitosamente
  Then: Se crea energy_event con operation_type=MCP_CALL, cost_category=EXTERNAL_SERVICE, quantity=1, quantity_unit=INVOCATIONS

TEST-088: Energy event por compute_time
  Given: Session en status running
  When: Session completa (duración 45 segundos)
  Then: Se crea energy_event con operation_type=COMPUTE_TIME, quantity=45, quantity_unit=SECONDS, cost_category=INTERNAL_INFRA

TEST-089: Energía total de session = Σ energy_events
  Given: Session completada con 3 nodos (LLM + MCP + db_write)
  When: Usuario consulta energía de la session
  Then: Total = suma de todos los energy_events de esa session
  And: Desglose visible por operation_type y cost_category

TEST-090: Múltiples energy_events por nodo con loop
  Given: Agente con nodo LLM dentro de loop (max_iterations=3)
  When: Loop ejecuta 3 iteraciones
  Then: Se crean 3 energy_events independientes (uno por iteración)
  And: Total del nodo = suma de las 3

### Journey: Energy Metering — Live Agent Run/Cycles

TEST-091: Energy tracking por cycle
  Given: Live agent en ejecución, run activo
  When: Cycle #1 completa con 2 nodos (LLM + tool)
  Then: Se crean energy_events con cycle_id=cycle_1 y run_id=run_1

TEST-092: Energy total de run = Σ cycles
  Given: Run con 5 cycles completados
  When: Usuario consulta energía del run
  Then: Total = suma de energy_events de todos los cycles del run

TEST-093: Stop de run registra compute_time final
  Given: Live agent running, 3 cycles completados
  When: Usuario detiene el run
  Then: Se crea energy_event(COMPUTE_TIME) con duración desde start hasta stop
  And: Run status=completed, stop_reason="user_stopped"

### Journey: Execution Control — Session

TEST-094: Controles visibles en session detail cuando running
  Given: Session en status=running
  When: Usuario navega a session detail page
  Then: Botones Stop y Pause visibles en header
  And: Contador de energía consumida visible y actualizándose

TEST-095: Pause session → controles cambian
  Given: Session running, usuario en session detail
  When: Usuario hace clic en Pause
  Then: Session status → interrupted. Botón cambia a Resume + Stop
  And: Contador de energía se congela

TEST-096: Resume session → ejecución continúa
  Given: Session interrupted
  When: Usuario hace clic en Resume
  Then: Session status → running. Ejecución continúa desde checkpoint
  And: Contador de energía reanuda

TEST-097: Stop session → ejecución cancela
  Given: Session running
  When: Usuario hace clic en Stop
  Then: Session espera nodo actual → status=failed, error="cancelled_by_user"
  And: Energy events finales registrados

TEST-098: Session completada → controles desaparecen
  Given: Session en status=completed
  When: Usuario navega a session detail
  Then: No hay botones Stop/Pause/Resume
  And: Total de energía consumida como dato final

### Journey: Execution Control — Run

TEST-099: Controles visibles en run detail cuando running
  Given: Live agent con run activo (status=running)
  When: Usuario navega a run detail page
  Then: Botón Stop visible. Energía acumulada visible por cycle

TEST-100: Stop run → run finaliza
  Given: Run activo ejecutando cycle #4
  When: Usuario hace clic en Stop
  Then: Cycle actual termina → run status=completed, stop_reason="user_stopped"

### Journey: Energy Visibility en Detail Pages

TEST-101: Session detail muestra energy breakdown
  Given: Session completada con energy_events
  When: Usuario abre session detail
  Then: Sección "Energía" con total, breakdown por nodo, breakdown por cost_category

TEST-102: Energía en tiempo real via WebSocket
  Given: Session running, usuario en session detail
  When: Nodo LLM completa y genera energy_event
  Then: Contador se actualiza vía WebSocket sin refresh

TEST-103: Run detail muestra energía por cycle
  Given: Run con 3 cycles completados
  When: Usuario abre run detail
  Then: Tabla de cycles con columna "Energía" por cycle. Total run = Σ cycles

TEST-104: Agent detail muestra resumen de energía
  Given: Agente con sessions/runs históricos
  When: Usuario abre agent detail
  Then: Resumen: energía total, promedio por session/run, gráfico últimos 30 días

### Journey: Energy Rates Configuration

TEST-105: Ver rates configurados
  Given: Energy rates seed data cargado
  When: Usuario navega a Settings → Energy Rates
  Then: Tabla con rates por operation_type, provider, model_pattern, energy_per_unit

TEST-106: Actualizar rate de LLM
  Given: Rate activo para claude-sonnet-* = 0.003 energy/token
  When: Usuario cambia a 0.005 energy/token y guarda
  Then: Rate actualizado. Futuras LLM calls usan nuevo rate. Events anteriores inmutables

TEST-107: Rate sin configurar → energy = 0 + warning
  Given: Nodo usa modelo sin rate configurado
  When: Nodo ejecuta exitosamente
  Then: Energy_event con energy_charged=0. Warning en logs

### Journey: Edge Cases

TEST-108: Ollama (local) → actual_cost_usd = 0
  Given: Agente con LLM en Ollama (local)
  When: Nodo LLM ejecuta
  Then: Energy_event con actual_cost_usd=0. energy_charged según rate configurado

TEST-109: Session falla mid-execution → energía registrada hasta falla
  Given: Agente con 5 nodos, nodo #3 falla
  When: Session falla
  Then: Energy_events de nodos 1, 2, 3 registrados. 4, 5 sin events. Compute_time hasta falla

TEST-110: Session sin nodos que consumen energía
  Given: Agente solo con nodos logic (conditions, transforms)
  When: Session completa
  Then: Solo energy_event de COMPUTE_TIME. Energía total = solo compute_time

### Journey: Session State Machine — Edge Cases

TEST-111: Pause en session pending → no disponible
  Given: Session en status=pending
  When: Usuario navega a session detail
  Then: Botón Pause no visible. Solo indicador de "pending"

TEST-112: Resume en session running → no disponible
  Given: Session en status=running
  When: Usuario navega a session detail
  Then: Botón Resume no visible. Solo Pause + Stop visibles

TEST-113: Stop en session completed → no acción
  Given: Session en status=completed
  When: Usuario navega a session detail
  Then: No hay botones de control. Solo datos finales de energía

TEST-114: Stop desde interrupted (pause → cancel)
  Given: Session en status=interrupted
  When: Usuario hace clic en Stop
  Then: Session → failed, error="cancelled_by_user". Energy registrada hasta pausa

TEST-115: Session timeout → energy registrada hasta timeout
  Given: Agente con timeout configurado (30s)
  When: Session excede timeout
  Then: Session status=timeout. Energy_events hasta timeout. Compute_time parcial

TEST-116: Pause espera nodo complete (REGLA-48)
  Given: Session running, nodo LLM ejecutándose
  When: Usuario hace clic en Pause
  Then: "Pausando..." transitorio. Nodo termina → session interrupted
  And: Energy_event del nodo registrado ANTES de interrupt

TEST-117: Stop espera nodo complete (REGLA-48)
  Given: Session running, nodo MCP ejecutándose
  When: Usuario hace clic en Stop
  Then: "Deteniendo...". Nodo termina → session failed
  And: Energy_events completos hasta nodo terminado

TEST-118: Doble pause → solo 1 interrupt (REGLA-16)
  Given: Session running
  When: Usuario hace clic en Pause dos veces rápido
  Then: Solo 1 interrupt creado. Segundo clic ignorado

TEST-119: Ciclo pause/resume múltiple → compute_time correcto
  Given: Session: running 10s → pause 5s → resume 10s → pause 5s → resume → completa en 10s
  When: Session completa
  Then: Compute_time = 30s (solo activo). Energy compute = 30s × rate

### Journey: Run State Machine — Edge Cases

TEST-120: Stop en run completado → no disponible
  Given: Run en status=completed
  When: Usuario navega a run detail
  Then: Botón Stop no visible

TEST-121: Run falla durante cycle → energy hasta falla
  Given: Live agent running, cycle #3 falla
  When: Run status=failed
  Then: Energy_events de cycles 1, 2 y parcial de 3 registrados

TEST-122: Stop durante cycle → cycle completa primero (REGLA-51)
  Given: Live agent running, cycle #4 en progreso
  When: Usuario hace clic en Stop
  Then: Cycle #4 completa → run completed, stop_reason="user_stopped"

### Journey: Energy Rate Changes + Isolation

TEST-123: Rate cambia mid-session → events con rate correcto (REGLA-41)
  Given: Rate claude-sonnet = 0.003. Session running
  When: Rate cambia a 0.005. Siguiente nodo LLM ejecuta
  Then: Pre-cambio: rate 0.003. Post-cambio: rate 0.005. Events previos inmutables

TEST-124: Rate energy_per_unit=0 → event con 0 energy
  Given: Rate tool_exec con energy_per_unit=0
  When: Nodo tool ejecuta
  Then: Energy_event con energy_charged=0. Aparece en breakdown

TEST-125: Múltiples sessions simultáneas → energy aislada
  Given: 2 agentes ejecutando sessions simultáneamente
  When: Ambas completan
  Then: Energy_events aislados por session_id. Totales independientes

### Journey: Energy Visibility — Completeness

TEST-126: Energy breakdown 3 capas de costo
  Given: Session con nodos LLM (external) + tool (internal) + platform_fee
  When: Usuario abre session detail → Energy
  Then: Breakdown: INTERNAL_INFRA, EXTERNAL_SERVICE, PLATFORM_FEE

TEST-127: Session histórica → energy final sin controles
  Given: Session completada hace 1 hora
  When: Usuario navega a session detail
  Then: Energy total final. Sin botones control. Breakdown completo

TEST-128: WebSocket reconecta → energy counter sincroniza
  Given: Session running, energy counter ticking
  When: WebSocket desconecta y reconecta
  Then: Counter re-sincroniza via re-fetch. Sin saltos

### Journey: Energy Depletion + Resume desde Checkpoint

TEST-129: Energy depletes mid-session → auto-pause
  Given: Balance=2. Agente con 5 nodos (~1 energía c/u)
  When: Nodo 1 (balance→1), nodo 2 (balance→0)
  Then: Auto-interrupt antes de nodo 3. Reason=energy_depleted. Checkpoint en nodo 2

TEST-130: Recharge → resume → session completa
  Given: Session interrupted (energy_depleted) en nodo 2 de 5
  When: Recarga 3 energía. Click Resume
  Then: Reanuda desde nodo 3. Nodos 3,4,5 completan. Total consumido = 5

TEST-131: UI feedback energy depleted
  Given: Session auto-pausada por energy_depleted
  When: Usuario abre session detail
  Then: Banner "Energía agotada". Resume deshabilitado. Counter = 0

TEST-132: Resume sin recarga → bloqueado
  Given: Session interrupted (energy_depleted), balance=0
  When: Usuario intenta Resume
  Then: Botón deshabilitado. "Sin energía disponible"

TEST-133: Energía alcanza exacto → session completa
  Given: Balance=1. Agente con 1 nodo restante
  When: Nodo completa, balance→0
  Then: Session completa. No hay nodo siguiente que bloquear

TEST-134: Live run → energy depletes → run pausa
  Given: Live agent running, balance=1. Cycle necesita 2
  When: Primer nodo completa (balance→0)
  Then: Run auto-pausa. Reason=energy_depleted

TEST-135: Recarga parcial → resume → depletes de nuevo
  Given: Session nodo 2/5 (depleted). Balance=0
  When: Recarga 1. Resume. Nodo 3 completa (balance→0)
  Then: Auto-pausa otra vez antes de nodo 4

TEST-136: Energy depletion muestra nodos completados vs pendientes
  Given: Session interrupted (depleted) en nodo 3 de 5
  When: Usuario abre session detail
  Then: Nodos 1,2 check. Nodo 3 pause/lock. Nodos 4,5 grayed. "2/5 — sin energía"
