# Data Mirai Engine — Arquitectura

## 1. Stack

| Capa | Tecnologia | Version |
|---|---|---|
| Core/Runtime | Python | 3.12+ |
| Web framework | FastAPI | latest |
| Editor frontend | Next.js + TypeScript + React Flow | Next.js 15 |
| Base de datos | PostgreSQL + pgvector | PG 16+ |
| Storage | S3-compatible (MinIO self-hosted, R2 cloud, cualquier S3) | — |
| LLM | Model-agnostic (cualquier provider via interfaces) | — |

## 2. Distribución

Un solo artefacto PyPI: `datamirai-engine`

| Modo | Comando / Uso | Incluye |
|---|---|---|
| **Standalone** | `pip install datamirai-engine && datamirai serve` o Docker | Editor (UI pre-built) + Runtime + API |
| **Embebido** | `from datamirai_engine import GraphRunner` + opcionalmente `app.mount("/datamirai", editor_app)` | SDK Python + editor montable |
| **Cloud** (datamirai-cloud) | Dep interna del producto cloud | Engine como librería, cloud agrega orquestación |

Frontend Next.js se compila a static assets en build time → FastAPI los sirve. Un solo proceso.

## 3. Recursos — Interfaces agnósticas

ExecutionContext define contratos abstractos. Quién inyecta credenciales depende del modo:

| Recurso | Interfaz | Self-hosted | Cloud |
|---|---|---|---|
| DB relacional | `context.db` | User configura connection string | Auto-provisioned por ambiente |
| DB vectorial | `context.vector` | User configura pgvector | Auto-provisioned |
| Storage | `context.storage` | User configura S3 endpoint | R2/GCS auto-provisioned |
| LLM | `context.llm` | User configura API keys | Keys del tenant |
| Auth | `context.auth` | User implementa | Gestionado por cloud |

Modelo híbrido (cloud hostea agentes, user tiene recursos externos) → API bridge con credenciales configuradas manualmente desde UI cloud.

## 4. Convenciones

### Nombrado

| Contexto | Convención |
|---|---|
| Archivos Python | snake_case |
| Archivos TS/React componentes | PascalCase |
| Archivos TS/React utils/hooks | camelCase |
| Funciones Python | snake_case |
| Funciones TS | camelCase |
| Clases (ambos) | PascalCase |
| Herramientas (tool_type) | `categoria/nombre_snake` |

### Estructura del repo

```
datamirai-engine/
  framework/
    src/datamirai_engine/        # Paquete Python principal (PyPI)
      core/                    # GraphDef, GraphRunner, SharedState, ExecutionContext
      tools/                   # ToolSpec, ToolRegistry, builtin/
      triggers/                # webhook, schedule, event, manual
      memory/                  # short_term, long_term
      runtime/                 # AgentRuntime, Scheduler
      resources/               # Interfaces agnósticas
    tests/                     # pytest (framework)
    pyproject.toml
  app/                         # App local (usa framework como dep)
    server/                    # FastAPI + SQLite persistence
    web/                       # Next.js 15 + React Flow
    e2e/                       # Playwright E2E tests
  docs/
  docker-compose.yml
```

## 5. Testing

| Tipo | Herramienta | Scope |
|---|---|---|
| Unit + Integration (Python) | pytest | core, blocks, triggers, memory |
| Unit (Frontend) | vitest | editor components, hooks |
| E2E (UI) | Playwright | editor visual, flujos completos |

## 6. Linting y Formato

| Lenguaje | Herramienta |
|---|---|
| Python | ruff (lint + format) |
| TypeScript | eslint + prettier |

## 7. Gestión de dependencias

| Lenguaje | Herramienta |
|---|---|
| Python | pyproject.toml (pip / uv compatible) |
| Frontend | npm + package.json |

## 8. Relación con datamirai-cloud

Engine = core open source. Cloud = producto privado que lo consume como dependencia.

```
datamirai-engine (público/PyPI)       datamirai-cloud (privado)
┌─────────────────────────┐         ┌────────────────────────────────┐
│ Motor de grafos          │         │ Control Plane (UI + billing)   │
│ Sistema de bloques       │◄────────│ K8s Operator (provisioning)    │
│ Editor visual            │  usa    │ Ambientes (dev/staging/prod)   │
│ Runtime + triggers       │         │ Auto-conexión a providers      │
│ Memoria                  │         │ Multi-tenancy                  │
│ Interfaces de recursos   │         │ Temporal (durabilidad)         │
│ Server (FastAPI)         │         │ Marketplace de bloques         │
└─────────────────────────┘         └────────────────────────────────┘
```

Cloud no forkea engine → lo instala como dep. Diferencia = quién orquesta y quién inyecta recursos.

## 9. Real-time — WebSocket + EventBus

La app local opera en modo **full real-time**: toda mutación de datos se refleja instantáneamente en el frontend via WebSocket. No hay polling ni fetch-on-demand como patrón primario.

### Transporte

Un solo WebSocket por sesión de browser:

```
ws://localhost:8000/ws
```

Protocolo de mensajes (JSON):

| Dirección | Tipo | Ejemplo |
|---|---|---|
| Cliente → Server | `subscribe` | `{"action": "subscribe", "channel": "resources:env-123"}` |
| Cliente → Server | `unsubscribe` | `{"action": "unsubscribe", "channel": "resources:env-123"}` |
| Server → Cliente | `event` | `{"channel": "resources:env-123", "event": "created", "payload": {...}}` |
| Server → Cliente | `ping` | `{"type": "ping"}` |
| Cliente → Server | `pong` | `{"type": "pong"}` |

Heartbeat: server envía `ping` cada 30s. Cliente responde `pong`. Sin respuesta en 60s → server cierra conexión.

### EventBus

Extiende `EventEmitter` (framework) con canales por entidad:

| Canal | Eventos | Scope |
|---|---|---|
| `universes` | created, deleted | Global |
| `environments:{universe_id}` | created, deleted, provisioned | Por universe |
| `resources:{env_id}` | created, updated, deleted, tested, status_changed | Por environment |
| `graphs:{env_id}` | created, updated | Por environment |
| `agents:{env_id}` | created, updated, deleted, published, rolled_back | Por environment |
| `sessions:{agent_id}` | created, status_changed | Por agent |
| `session:{id}:events` | block.started, block.completed, block.error, session.completed, session.failed, session.interrupted, checkpoint.created, interrupt.created, interrupt.resolved | Por session |
| `providers` | created, updated, deleted | Global |
| `vaults` | created, deleted | Global |
| `credentials:{vault_id}` | created, updated, deleted | Por vault |
| `config` | updated | Global |
| `memory:{agent_id}` | created, deleted | Por agent |

### Convención obligatoria

**Toda operación de escritura en un repositorio de database.py DEBE emitir un evento al EventBus.** Patrón:

```python
# En el endpoint o servicio, después de la mutación DB:
await event_bus.emit(channel="resources:env-123", event="created", payload={...})
```

Nunca mutar la DB sin emitir. Si se agrega un nuevo endpoint con escritura, agregar emisión correspondiente.

### Migración de SSE

El endpoint SSE (`GET /api/sessions/{id}/stream`) queda **deprecado**. Las sesiones de ejecución se consumen via el canal `session:{id}:events` del WebSocket. Los mismos event types, mismo payload, distinto transporte.

## 10. Frontend — Patrón reactivo

### WebSocketManager

Singleton que vive en `layout.tsx` como Context provider. Responsabilidades:

- Mantener conexión WebSocket
- Reconexión automática con exponential backoff (1s → 2s → 4s → 8s → 16s, max 5 intentos)
- Registro de suscripciones activas por canal
- Re-suscripción automática tras reconexión
- Estado de conexión observable: `connected | connecting | reconnecting | disconnected`

### Hook useReactive

Patrón estándar para consumir datos reactivos:

```typescript
const { data, loading, error } = useReactive<Resource[]>(
  "/api/environments/{envId}/resources",  // endpoint para fetch inicial
  `resources:${envId}`,                    // canal WebSocket
);
```

Comportamiento:
1. **Mount**: fetch inicial al endpoint REST
2. **Suscribe**: se registra al canal WebSocket
3. **Evento llega**: re-fetch automático (estrategia de invalidación)
4. **Unmount**: unsuscribe del canal

Estrategia de invalidación (no delta): cuando llega un evento del canal, el hook hace re-fetch completo al endpoint REST. Simple, robusto, suficiente para latencia local (< 1ms).

### Convención obligatoria

**Toda página que muestra datos del backend DEBE usar `useReactive` en vez de `fetch` + `useState` directo.** Esto garantiza que los datos se actualicen automáticamente sin polling ni recarga manual.

Excepciones permitidas:
- Datos estáticos que no cambian (tool catalog, resource schemas)
- Acciones one-shot (test connection, generate graph)

### Loading & feedback

- **Carga inicial**: skeleton con shimmer (obligatorio, nunca pantalla en blanco)
- **Eventos de background**: toast automático para creaciones, eliminaciones y errores
- **Indicador de conexión**: dot en el header que refleja estado del WebSocketManager

## 11. Motion & Live UI

### Stack de animación

| Necesidad | Tecnología |
|---|---|
| Layout animations, stagger, enter/exit | Motion (Framer Motion) |
| Glows, pulses, shimmer | CSS keyframes (globals.css) |
| Edge particles | SVG `animateMotion` |
| Counters numéricos | Motion AnimateNumber |
| Toasts | Sonner o custom con Motion |

### Convención: sin cambios abruptos

**Toda transición de estado visible DEBE usar animación.** Nunca un elemento aparece/desaparece/cambia sin transición. Duración estándar:

| Tipo | Duración | Easing |
|---|---|---|
| Micro-interacción (hover, press) | 100-150ms | ease-out |
| Transición de estado (badge, dot) | 200-300ms | ease-in-out |
| Panel slide (entrada/salida) | 300-400ms | ease-in-out |
| Stagger entre items | 50-100ms delay | ease-out |
| Glow/pulse loop | 2-3s | ease-in-out, infinite |

### Estados de nodo en ejecución

| Estado | Visual | Animación |
|---|---|---|
| idle | Border `var(--line-2)`, sin animación | — |
| queued | Border dashed, opacity pulse | `animate-pulse` 2s |
| running | Border gradient violeta→cyan, glow expandiéndose | `glow-pulse` 2s infinite |
| completed | Border verde, badge checkmark | Flash verde 0.6s + badge pop 0.35s |
| failed | Border rojo, badge X, shake | Shake 0.4s + glow rojo 0.6s |
| retrying | Border amarillo, spinner | Rotate 0.8s infinite |
| waiting-for-input | Border cyan, icono pausa | Pulse lento 3s infinite |

### Edges durante ejecución

- **Activo**: partículas (dots 4-6px con glow) viajando por el path SVG via `animateMotion`, 2s por traversal
- **Completado**: stroke verde sólido, sin animación
- **Fallido**: stroke rojo sólido
- **Inactivo**: stroke gris, opacity 0.08

### Contadores en vivo

- Tokens y costo: `AnimateNumber` con spring physics (stiffness: 100, damping: 15)
- Duración: timer ticking cada 100ms, colon parpadeante (1s interval)
- Flash de actualización: background `rgba(34, 197, 94, 0.3)` que fade a transparent en 500ms

### Feed de eventos

- Cada evento entra con slide-in desde izquierda: `x: -20 → 0, opacity: 0 → 1`
- Stagger: 100ms entre items consecutivos
- Auto-scroll suave al bottom (pausa si usuario scrollea arriba)
- Eventos recientes en color completo, viejos fade a `var(--fg-3)`
