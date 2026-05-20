# Data Mirai Engine — Flujos y Reglas

## Flujo principal

```
Diseñar grafo (editor) → Desplegar como agente → Habilitar → Trigger fires → Session ejecuta → Memoria guarda → Resultado
```

## Máquinas de estado

### Agent Lifecycle

```
disabled ←→ enabled
                │
                └→ triggers registrados, esperando activación
                │
                ├── trigger fires → crea session
                │
                └→ disabled (apagar sin destruir)
```

- `disabled`: existe pero no responde a triggers
- `enabled`: activo, escuchando triggers configurados
- Destruir agente = operación separada (elimina definición + memoria)

### Session (ejecución individual)

```
pending → running → completed
                  → failed
                  → timeout
         running ↔ interrupted
     interrupted → cancelled
```

- `pending`: creada por trigger, esperando slot de ejecución
- `running`: cursor recorriendo el grafo
- `completed`: grafo terminó sin errores
- `failed`: error no recuperable (retries agotados, on_failure=stop, hook abort, cancelled_by_user)
- `timeout`: excedió tiempo máximo configurado
- `interrupted`: pausada por nodo logic/human_input o por session control manual. Checkpoint creado. Espera acción del usuario

| From | To | Guard | Side-effect |
|---|---|---|---|
| pending | running | Slot disponible | Crear checkpoint inicial (step 0) |
| running | completed | Grafo terminó sin errores | Persistir short_term memory. Crear spans telemetría |
| running | failed | Error no recuperable | Persistir error en span. Persistir último checkpoint |
| running | timeout | Excedió tiempo máximo | Persistir último checkpoint |
| running | interrupted | Nodo logic/human_input alcanzado O usuario pausa manualmente | Crear checkpoint + interrupt. Emitir evento SSE |
| interrupted | running | Interrupt resuelto (usuario respondió o resumió) | Cargar checkpoint. Inyectar respuesta si aplica. Emitir session.resumed |
| interrupted | failed | Usuario cancela | Marcar interrupt CANCELLED |
| interrupted | timeout | Timeout del interrupt | Marcar interrupt EXPIRED |

### Graph Version

```
draft → published → promoted (a environment superior)
                  → deprecated (reemplazado por nueva versión)
```

- Solo 1 versión publicada por environment
- `draft`: editándose en el editor
- `published`: activa en su environment
- Promover = copiar snapshot a environment superior

## Runtime — Modelo de ejecución

Environment = proceso long-running (FastAPI) que gestiona todos los agentes:

```
Environment process (siempre activo)
  ├── Webhook listener — recibe HTTP, rutea al agente correcto
  ├── Scheduler — cron interno, dispara agentes programados
  ├── Event listener — suscrito a PG NOTIFY / storage notifications
  └── Registry de agentes enabled — mapea trigger → agente
```

No es 1 proceso por agente. UN proceso gestiona todos.

## Triggers

| Trigger | Activación | Tipo |
|---|---|---|
| **webhook** | HTTP request externo | Pasivo — listener siempre escuchando |
| **schedule** | Cron/intervalo configurado | Activo — scheduler interno dispara |
| **event** | Notificación de recurso (file uploaded, row inserted, etc.) | Pasivo — suscrito a notificaciones |
| **manual** | Usuario ejecuta desde UI/API | API call al environment |
| **agent_call** | Otro agente invoca a este | Interno — sub-ejecución |

`agent_call` permite composición: agente orquestador llama sub-agentes especializados. Reutilización de recursos a nivel de agentes.

## Environments — Stack de promoción

Environments organizados en stack ordenado (no por nombre, por posición):

```
Ejemplo:
  [0] dev        ← más bajo
  [1] staging
  [2] prod       ← más alto
```

- Posición define jerarquía, no el nombre
- Promover = copiar snapshot de graph version al environment siguiente en el stack
- Solo se promueve hacia arriba (posición mayor)
- UI permite crear environments en cualquier posición del stack (arriba, abajo, entre medio)
- Cada environment tiene sus propios recursos independientes

### Promoción

```
Graph v3 en dev [0] → promote → Graph v3 en staging [1] → promote → Graph v3 en prod [2]
```

- Promueve definición (grafo + config), NO datos ni memoria
- Rollback = re-promover versión anterior
- Historial de versiones se mantiene por environment

## Reglas globales

| ID | Regla |
|---|---|
| REGLA-01 | Ejecución = cursor secuencial. Solo un camino activo por session. Sin paralelismo |
| REGLA-02 | Output de bloque → SharedState[node_id]. Inmutable durante ejecución normal. Modificable únicamente durante session_control interrupt: flujo se pausa → usuario notificado → hace cambios → guarda → reanuda. Se crea nuevo checkpoint |
| REGLA-03 | Edge condicional: solo UNA edge true por nodo. Ninguna true = error |
| REGLA-04 | Loop: `max_iterations` obligatorio. Exceder = error |
| REGLA-05 | Retry policy por bloque. Retries agotados → on_failure (stop/skip/route_to_error) |
| REGLA-06 | Auth: engine recibe identidad verificada. Sin identidad + modo restrictivo = reject |
| REGLA-07 | Agente solo accede a recursos de su environment |
| REGLA-08 | Memory short_term (transcript) en RAM durante session → persiste a Postgres al terminar. Checkpoints son sistema aparte: snapshots del SharedState en cada paso. Resultados del agente != snapshot de lo que hizo para llegar al resultado |
| REGLA-09 | Solo 1 versión publicada de un grafo por environment |
| REGLA-10 | Promoción solo hacia arriba en el stack de environments |
| REGLA-11 | agent_call: sub-agente hereda resources del environment pero crea session propia |
| REGLA-12 | Cada bloque completado genera exactamente un checkpoint con state_snapshot completo |
| REGLA-13 | Checkpoints inmutables. Fork/rewind crean nueva session, no modifican checkpoints |
| REGLA-14 | Pause solo en running. Espera que bloque actual termine antes de pausar |
| REGLA-15 | State editable solo en interrupted. Usuario notificado → modifica → guarda → reanuda |
| REGLA-16 | Max 1 interrupt PENDING por session a la vez |
| REGLA-17 | Respuesta de interrupt se inyecta como output del nodo en SharedState |
| REGLA-18 | Exactamente 1 snapshot ACTIVE por agente. Publicar depreca anterior atómicamente |
| REGLA-19 | Sessions usan graph_def del snapshot, no del draft. Ediciones no afectan sessions en curso |
| REGLA-20 | Valores de credenciales nunca en responses API. Solo metadata |
| REGLA-21 | Credenciales cifradas en reposo. Sin clave configurada, vault no opera — UI muestra wizard |
| REGLA-22 | Hooks timeout 30s. Si excede, se trata como continue |
| REGLA-23 | Hook abort = session falla inmediatamente |
| REGLA-24 | Un nodo usa UN tool (builtin o MCP). No híbridos |
| REGLA-25 | Telemetría async, nunca bloquea ejecución. Si falla, ejecución continúa |
| REGLA-26 | Toda mutación DB debe emitir evento al EventBus. Sin emisión = bug |
| REGLA-27 | Frontend usa `useReactive` para datos del backend. Fetch directo solo para datos estáticos |
| REGLA-28 | Toda transición de estado visible debe usar animación. Sin cambios abruptos |
| REGLA-29 | WebSocket reconexión: exponential backoff 1s→2s→4s→8s→16s, max 5 intentos |
| REGLA-30 | Skeleton con shimmer obligatorio durante carga inicial. Nunca pantalla en blanco |
| REGLA-31 | Un solo WebSocket por sesión de browser. Nunca múltiples conexiones simultáneas desde la misma tab |
| REGLA-32 | Canales WebSocket siguen formato estricto: `{entity}` o `{entity}:{parent_id}`. Canal inválido = error, no silencio |
| REGLA-33 | Evento WebSocket NUNCA incluye datos sensibles (credenciales, API keys). Solo metadata e IDs |
| REGLA-34 | Re-suscripción automática tras reconexión. El frontend NUNCA debe quedar suscrito a cero canales después de reconectar |
| REGLA-35 | Emisión de evento al EventBus es fire-and-forget. Si falla la emisión, la mutación DB NO se revierte — el dato es la fuente de verdad, no el evento |
| REGLA-36 | Animaciones usan solo propiedades GPU-accelerated (transform, opacity, box-shadow). Nunca animar width, height, top, left |
| REGLA-37 | Buffer de eventos por sesión = 100 eventos max. Eventos más viejos se descartan. Suficiente para reconexión, no para replay completo |
| REGLA-38 | useReactive invalida via re-fetch, nunca aplica delta directo del evento. El REST endpoint es la fuente de verdad |

### WebSocket Connection (frontend)

```
disconnected → connecting → connected
                           → disconnected (timeout/error)
     connected → disconnected (ws.onclose inesperado)
     connected → reconnecting (ws.onclose con auto-retry)
  reconnecting → connected (ws.onopen)
  reconnecting → disconnected (max retries agotados)
```

- `disconnected`: sin conexión activa. Indicador rojo. Botón reconectar manual si max retries agotados
- `connecting`: intento inicial al montar la app. Indicador "Conectando..."
- `connected`: socket activo, canales suscritos. Indicador verde con glow ring
- `reconnecting`: reconexión automática tras desconexión. Indicador amarillo, toast "Reconectando..."

| From | To | Guard | Side-effect |
|---|---|---|---|
| disconnected | connecting | App mount o reconexión manual | Crear WebSocket, mostrar indicador |
| connecting | connected | `ws.onopen` | Re-suscribir canales previos, indicador verde, toast "Conectado" |
| connecting | disconnected | Timeout 5s o error | Indicador rojo, programar retry |
| connected | disconnected | `ws.onclose` inesperado | Indicador rojo, programar retry con backoff |
| connected | reconnecting | `ws.onclose` con auto-retry habilitado | Indicador amarillo, toast "Reconectando..." |
| reconnecting | connected | `ws.onopen` | Re-suscribir canales, indicador verde |
| reconnecting | disconnected | Max retries (5) agotados | Indicador rojo, toast "Desconectado", botón reconectar manual |

**Exponential backoff**: 1s → 2s → 4s → 8s → 16s. Reset a 1s tras conexión exitosa.
