# FEAT-005 — Tier 3: Inteligencia (Context Modeling + Tool Scoring + Nudges + Curator)

**Estado**: Draft
**Fecha**: 2026-05-09
**Epic**: EPIC-050

---

## Problem Statement

**Tipo**: Feature nueva (4 capacidades de inteligencia)
**Actor**: Usuario local — persona que instala y opera Data Mirai Engine en su computadora.

Data Mirai Engine ejecuta grafos y persiste memoria (FEAT-001, FEAT-002), pero los agentes no aprenden de su operacion. Tras cientos de ejecuciones, el agente no sabe que "los viernes hay mas volumen" ni que "el campo 3 siempre trae el monto". El usuario debe descubrir estos patrones manualmente.

Ademas, el Designer Assistant (FEAT-001) no prioriza tools por relevancia — el catalogo se muestra plano. Y no hay mecanismo para notificar al usuario sobre patrones importantes ni para limpiar reglas/trazas obsoletas.

Cuatro carencias concretas:

1. **Sin aprendizaje contextual**: El agente no extrae conclusiones de sus ejecuciones. La memoria de largo plazo guarda learnings genericos pero no construye un modelo acumulativo del contexto de operacion.

2. **Catalogo de tools sin prioridad**: Al disenar un agente con el Designer Assistant, todas las tools tienen igual peso. No hay scoring por relevancia al contexto del agente que se esta disenando.

3. **Sin notificaciones de patrones**: No hay mecanismo para alertar al usuario sobre hallazgos relevantes (N ejecuciones exitosas, nodo fallando repetidamente, reflexiones sin revisar).

4. **Sin mantenimiento automatizado**: Playbook rules se acumulan sin poda. Trazas de ejecucion crecen indefinidamente. No hay proceso de limpieza.

---

## Objetivo

Cuando esto este implementado, el usuario puede:
1. Ver conclusiones que el agente extrae automaticamente de sus ejecuciones, organizadas por tipo (explicitas, deductivas, inductivas, contradicciones)
2. Recibir un catalogo de tools ordenado por relevancia cuando diseña un agente con el Designer Assistant
3. Recibir notificaciones inteligentes sobre patrones de uso de sus agentes
4. Configurar un proceso de mantenimiento que poda reglas obsoletas, fusiona duplicadas y limpia trazas antiguas

---

## Features

### 4.1 — User/Context Modeling

> **Nota**: El **almacenamiento acumulativo de datos** (scrapes, analisis, reportes entre sesiones) se resuelve con FEAT-020.1 Knowledge Vault. Esta feature (4.1) se enfoca exclusivamente en **conclusiones deductivas** extraidas por el LLM del transcript — no en almacenar datos crudos. El vault guarda los datos; Context Modeling extrae conclusiones de esos datos.

**Problema**: El agente ejecuta grafos repetidamente pero no construye un modelo del contexto en que opera. Los learnings de `LongTermMemory` (FEAT-002) son resumenes genericos por sesion. No hay extraccion estructurada de conclusiones tipadas ni acumulacion incremental de conocimiento. El usuario no puede ver que ha aprendido el agente sobre su dominio.

**Solucion**: Despues de cada ejecucion completada, un paso post-session analiza el transcript y extrae conclusiones de 4 niveles cognitivos. Las conclusiones se persisten en tabla dedicada y se inyectan en llm_call via Context Compiler. Conclusiones repetidas se refuerzan (incremento de confidence y times_reinforced). Contradicciones detectadas invalidan conclusiones previas.

**Arquitectura**:
- `ContextModeler` — componente del framework que analiza el transcript de una session completada y extrae conclusiones
  - Metodo principal: `extract_conclusions(agent_id, session_id, transcript, existing_conclusions) -> list[Conclusion]`
  - Usa llm_call interno con prompt especializado que recibe el transcript + conclusiones existentes del agente
  - Retorna lista de conclusiones nuevas, reforzadas o contradichas
  - Se ejecuta post-session (despues de que el runner completa, antes de cerrar la session)
- `Conclusion` dataclass:
  - `id: str` — UUID
  - `agent_id: str` — agente dueno
  - `conclusion_type: str` — explicit / deductive / inductive / contradiction
  - `content: str` — texto de la conclusion
  - `confidence: float` — 0.0 a 1.0, se incrementa con refuerzos
  - `source_session_ids: list[str]` — sessions que contribuyeron a esta conclusion
  - `times_reinforced: int` — cuantas veces se ha visto confirmada
  - `contradicts_id: str | None` — si es contradiction, ID de la conclusion que contradice
  - `status: str` — active / superseded / dismissed
  - `created_at: str`
  - `updated_at: str`
- Tipos de conclusion:
  - `explicit` — statements directos extraidos del data ("el cliente envia PDFs", "el campo email es obligatorio")
  - `deductive` — conclusiones logicas derivadas ("los montos siempre estan en columna 3", "el header tiene 2 filas")
  - `inductive` — generalizaciones de patrones observados ("los viernes hay mas volumen", "los archivos de enero son mas grandes")
  - `contradiction` — conflictos detectados con conclusiones previas ("antes era CSV, ahora es PDF"). Al crear una contradiction, la conclusion contradecida pasa a status `superseded`
- Logica de refuerzo:
  - Al extraer conclusiones, el ContextModeler compara contra conclusiones existentes (active) del agente
  - Si una conclusion nueva es semanticamente equivalente a una existente → no se crea nueva, se incrementa `times_reinforced` y se agrega el session_id a `source_session_ids`. Confidence sube: `min(1.0, confidence + 0.1)`
  - Si una conclusion nueva contradice una existente → se crea conclusion tipo `contradiction`, la existente pasa a `superseded`
  - Equivalencia semantica: cosine similarity > 0.85 entre embeddings de content (usa HybridSearch de FEAT-002)
- Context Compiler:
  - Cuando un nodo `ai/llm_call` se ejecuta, el runner inyecta conclusiones activas del agente como contexto adicional
  - Solo conclusiones con `confidence >= 0.5` y `status == active`
  - Se inyectan como bloque de texto al inicio del prompt: "Contexto aprendido del agente: ..."
  - Formato compacto: un bullet por conclusion, ordenadas por confidence descendente
  - Max 20 conclusiones inyectadas (configurable) para no consumir contexto excesivo

**Entidades nuevas**:

Tabla `context_conclusion` en SQLite:

| Columna | Tipo | Constraint |
|---|---|---|
| id | TEXT | PK |
| agent_id | TEXT | FK agents(id) ON DELETE CASCADE, NOT NULL |
| conclusion_type | TEXT | NOT NULL (explicit / deductive / inductive / contradiction) |
| content | TEXT | NOT NULL |
| confidence | REAL | NOT NULL DEFAULT 0.5 |
| source_session_ids | TEXT | JSON array, NOT NULL DEFAULT '[]' |
| times_reinforced | INTEGER | NOT NULL DEFAULT 0 |
| contradicts_id | TEXT | FK context_conclusion(id), NULL |
| status | TEXT | NOT NULL DEFAULT 'active' (active / superseded / dismissed) |
| embedding | BLOB | NULL (para busqueda de equivalencia semantica) |
| created_at | TEXT | NOT NULL |
| updated_at | TEXT | NOT NULL |

Indices:
- `CREATE INDEX idx_conclusion_agent ON context_conclusion(agent_id, status)`
- `CREATE INDEX idx_conclusion_type ON context_conclusion(agent_id, conclusion_type)`

**Contratos API**:
- `GET /api/agents/{id}/conclusions` — listar conclusiones del agente. Query params: `type` (filtrar por conclusion_type), `status` (default active), `limit` (default 50). Response: `{ conclusions: Conclusion[], total: int }`
- `DELETE /api/conclusions/{id}` — eliminar conclusion. Pasa a status `dismissed`. Response: `204`
- `PATCH /api/conclusions/{id}` — actualizar conclusion (permite editar content, cambiar status manualmente). Body: `{ content?, status? }`. Response: `{ conclusion: Conclusion }`

**Pantallas**:
- **AgentDetail** (`/agents/[id]`) — nuevo tab "Contexto":
  - Conclusiones agrupadas por tipo (4 secciones: Explicitas, Deductivas, Inductivas, Contradicciones)
  - Cada conclusion muestra: content, confidence como barra visual, times_reinforced como badge, fecha
  - Contradicciones muestran enlace a la conclusion que contradicen
  - Accion por conclusion: Dismiss (eliminar), Edit (modificar texto)
  - Conclusiones superseded se muestran tachadas con label "Reemplazada"
  - Contador total por tipo en header de cada seccion
  - Empty state: "Este agente aun no ha aprendido nada. Ejecutalo para que empiece a extraer conclusiones."

**Reglas**:

| ID | Invariante | Enforcement |
|---|---|---|
| REGLA-71 | ContextModeler se ejecuta solo en sessions completadas (status=completed). Sessions fallidas o canceladas no generan conclusiones | Post-session hook, guard por status |
| REGLA-72 | Equivalencia semantica = cosine similarity > 0.85. Por debajo, se trata como conclusion distinta | ContextModeler._is_equivalent con threshold configurable |
| REGLA-73 | Al crear contradiction, la conclusion contradecida pasa a superseded atomicamente (misma transaccion) | Service layer con transaccion SQLite |
| REGLA-74 | Confidence nunca excede 1.0. Formula: min(1.0, current + 0.1) por refuerzo | ContextModeler._reinforce |
| REGLA-75 | Max 20 conclusiones inyectadas en llm_call. Si hay mas activas con confidence >= 0.5, se toman las de mayor confidence | Context Compiler con ORDER BY confidence DESC LIMIT |
| REGLA-76 | Context Compiler es opt-in por nodo. Campo `inject_context: bool` en config del nodo ai/llm_call (default true) | NodeConfigPanel + runner |
| REGLA-77 | Si no hay embedding provider configurado, ContextModeler no puede comparar equivalencia semantica. Cada conclusion se guarda como nueva (sin dedup). Warning en log | Fallback graceful, mismo patron que REGLA-38 |

**Archivos a crear/modificar**:
- CREAR `framework/src/datamirai_engine/intelligence/__init__.py` — exports del modulo
- CREAR `framework/src/datamirai_engine/intelligence/context_modeler.py` — ContextModeler + Conclusion dataclass
- CREAR `framework/src/datamirai_engine/intelligence/context_compiler.py` — ContextCompiler (inyecta conclusiones en prompt)
- MODIFICAR `framework/src/datamirai_engine/core/runner.py` — invocar ContextModeler post-session en sessions completadas
- MODIFICAR `framework/src/datamirai_engine/tools/builtin/ai/llm_call.py` — integrar ContextCompiler para inyectar conclusiones en prompt
- MODIFICAR `app/server/datamirai_app/database.py` — tabla context_conclusion + indices + ConclusionRepo (SCHEMA_VERSION bump)
- CREAR `app/server/datamirai_app/routes/conclusions.py` — endpoints GET/DELETE/PATCH conclusiones
- MODIFICAR `app/server/datamirai_app/app.py` — registrar blueprint de conclusions
- MODIFICAR `app/web/src/lib/api.ts` — funciones client para endpoints de conclusiones
- MODIFICAR `app/web/src/app/agents/[id]/page.tsx` — agregar tab "Contexto" con vista de conclusiones
- MODIFICAR `app/web/src/components/editor/NodeConfigPanel.tsx` — agregar toggle `inject_context` para nodos ai/llm_call

---

### 4.2 — Tool Scoring para Designer Assistant

**Problema**: Cuando el usuario diseña un agente con el Designer Assistant (FEAT-001), el catalogo de tools se muestra plano — todas las herramientas con igual peso. Si el usuario describe "un agente que analiza PDFs y extrae datos", el catalogo muestra trigger/webhook y logic/loop con la misma prominencia que ai/llm_call y data/storage_read. Esto obliga al usuario a buscar manualmente las tools relevantes.

**Solucion**: Un componente `ToolScorer` que recibe el contexto de diseno (descripcion del agente, conversacion con el Designer Assistant) y retorna tools rankeadas por relevancia. Usa embedding similarity entre la descripcion de la tarea y las descripciones de los tools. Fallback a keyword matching cuando no hay embedding provider.

**Arquitectura**:
- `ToolScorer` — componente del framework que puntua tools por relevancia a un contexto dado
  - Metodo principal: `score_tools(context: str, tool_specs: list[ToolSpec]) -> list[ScoredTool]`
  - `ScoredTool`: `{ tool_type: str, score: float, reason: str }` — score en [0, 1], reason explica por que es relevante
  - Estrategia primaria: embedding similarity
    1. Genera embedding del context (descripcion + conversacion)
    2. Genera embeddings de cada tool (tool_type + display_name + description + inputs/outputs). Estos se cachean en memoria — no cambian entre llamadas
    3. Cosine similarity entre context embedding y cada tool embedding
    4. Normaliza scores a [0, 1]
  - Estrategia fallback: keyword matching
    1. Tokeniza context en keywords (stopwords removidos)
    2. Busca matches en tool_type, display_name, description de cada tool
    3. Score = count(matches) / count(keywords), normalizado a [0, 1]
  - Ambas estrategias retornan `reason` — una frase corta que explica la relevancia (generada por el LLM en modo embedding, o por los keywords matcheados en modo fallback)
- Integracion con Designer Assistant:
  - El endpoint `POST /api/tools/score` recibe el contexto de diseno y retorna tools rankeadas
  - El frontend llama a este endpoint cuando el usuario abre el catalogo de tools durante una sesion de diseno
  - El catalogo se reordena por score descendente
  - Tools con score > 0.7 reciben badge "Relevante"
  - Tools con score < 0.3 se colapsan en seccion "Otras herramientas"
- Cache de tool embeddings:
  - Los embeddings de tools se generan una vez y se cachean en memoria del server (dict tool_type → embedding)
  - Se invalidan cuando el ToolRegistry cambia (nueva herramienta registrada)
  - No se persisten a DB — se regeneran al reiniciar el server (son pocos, ~20 tools)

**Entidades nuevas**: Ninguna. Tool scoring es stateless — no persiste resultados. Los embeddings de tools se cachean en memoria del server, no en DB.

**Contratos API**:
- `POST /api/tools/score` — puntuar tools por relevancia al contexto de diseno. Body: `{ context: str }`. Response: `{ tools: ScoredTool[] }` donde `ScoredTool = { tool_type: str, display_name: str, category: str, score: float, reason: str }`. Ordenados por score descendente.

**Pantallas**:
- **AgentDesigner** (componente existente en `/agents/[id]?design=1`):
  - Cuando hay contexto de diseno (descripcion del agente o conversacion activa), el catalogo de tools se reordena por score
  - Badge visual "Relevante" en tools con score > 0.7 (color accent, pill shape)
  - Seccion colapsable "Otras herramientas" para tools con score < 0.3
  - Tooltip en cada tool mostrando `reason` del score
  - Si no hay contexto de diseno (catalogo abierto sin Designer Assistant activo), se muestra orden por defecto (por categoria)

**Reglas**:

| ID | Invariante | Enforcement |
|---|---|---|
| REGLA-78 | Tool scoring es solo para Designer Assistant (design-time). No afecta runtime ni ejecucion de grafos | Endpoint separado, sin integracion con runner |
| REGLA-79 | Si no hay embedding provider configurado, cae a keyword matching. Nunca falla — siempre retorna scores | ToolScorer con fallback chain |
| REGLA-80 | Tool embeddings se cachean en memoria del server. Se regeneran al reiniciar o cuando ToolRegistry cambia | In-memory cache con invalidacion |
| REGLA-81 | Scores normalizados a [0, 1]. Nunca negativo, nunca > 1 | ToolScorer._normalize |
| REGLA-82 | El endpoint /api/tools/score no requiere agent_id. Funciona con contexto libre (texto) | API design — stateless |

**Archivos a crear/modificar**:
- CREAR `framework/src/datamirai_engine/intelligence/tool_scorer.py` — ToolScorer + ScoredTool dataclass
- CREAR `app/server/datamirai_app/routes/tool_scoring.py` — endpoint POST /api/tools/score
- MODIFICAR `app/server/datamirai_app/app.py` — registrar blueprint de tool_scoring
- MODIFICAR `app/web/src/lib/api.ts` — funcion client `scoreTools(context: string)`
- MODIFICAR `app/web/src/components/agent/AgentDesigner.tsx` — integrar scoring al catalogo de tools durante diseno
- MODIFICAR `app/web/src/components/editor/ToolCatalog.tsx` — soportar ordenamiento por score, badge "Relevante", seccion colapsable

---

### 4.3 — Nudge System

**Problema**: El usuario opera multiples agentes pero no tiene visibilidad sobre patrones que emergen del uso. Despues de 50 ejecuciones exitosas no sabe que podria publicar el agente como template. Si un nodo falla 5 veces consecutivas no recibe alerta. Si el Context Modeler (4.1) genera insights interesantes, nadie se los muestra proactivamente.

**Solucion**: Sistema de notificaciones inteligentes (nudges) que detecta patrones de uso y genera mensajes para el usuario. Los nudges son pasivos — se muestran en la UI, el usuario decide si actua. No ejecutan acciones automaticas.

**Arquitectura**:
- `NudgeEngine` — componente del framework que evalua condiciones y genera nudges
  - Metodo principal: `evaluate(agent_id) -> list[Nudge]`
  - Se ejecuta post-session (despues de ContextModeler) y en evaluacion periodica (cada hora, configurable)
  - Evalua reglas predefinidas contra datos del agente (sessions, conclusiones, spans)
  - Si una condicion se cumple y no hay nudge pendiente del mismo tipo para ese agente → crea nudge
- `Nudge` dataclass:
  - `id: str` — UUID
  - `agent_id: str` — agente relacionado (NULL para nudges globales)
  - `nudge_type: str` — tipo de nudge (ver tabla abajo)
  - `message: str` — texto que se muestra al usuario
  - `metadata: dict` — datos adicionales (e.g., nombre del nodo que falla, count de ejecuciones)
  - `status: str` — pending / seen / acted / dismissed
  - `created_at: str`
- Nudge types predefinidos:

| nudge_type | Condicion | Mensaje template |
|---|---|---|
| `template_candidate` | Agent tiene >= N ejecuciones exitosas consecutivas (N configurable, default 20) | "Tu agente '{name}' tiene {count} ejecuciones exitosas. Podrias publicarlo como template." |
| `failing_node` | Un nodo falla en las ultimas N ejecuciones consecutivas (N configurable, default 5) | "El nodo '{node_name}' ha fallado {count} veces seguidas. Quieres que el asistente lo revise?" |
| `unreviewed_insights` | Hay >= N conclusiones nuevas (4.1) sin que el usuario haya visitado el tab Contexto (N configurable, default 5) | "Hay {count} insights nuevos sobre tu agente '{name}'." |
| `high_cost_alert` | Costo estimado de las ultimas N sessions excede threshold (configurable) | "Las ultimas {count} ejecuciones de '{name}' costaron ~${total}. Revisa si hay optimizaciones." |
| `idle_agent` | Agent habilitado sin ejecuciones en N dias (configurable, default 30) | "Tu agente '{name}' lleva {days} dias sin actividad. Quieres deshabilitarlo?" |

- Dedup: antes de crear nudge, verifica que no exista uno con mismo `agent_id + nudge_type` en status `pending`. Si existe, no crea duplicado.
- El usuario interactua con nudges:
  - `seen`: lo vio pero no actuo (se marca automatico al expandir)
  - `acted`: hizo click en la accion sugerida
  - `dismissed`: lo descarto explicitamente

**Entidades nuevas**:

Tabla `nudge` en SQLite:

| Columna | Tipo | Constraint |
|---|---|---|
| id | TEXT | PK |
| agent_id | TEXT | FK agents(id) ON DELETE CASCADE, NULL (nudges globales) |
| nudge_type | TEXT | NOT NULL |
| message | TEXT | NOT NULL |
| metadata | TEXT | JSON, DEFAULT '{}' |
| status | TEXT | NOT NULL DEFAULT 'pending' (pending / seen / acted / dismissed) |
| created_at | TEXT | NOT NULL |

Indices:
- `CREATE INDEX idx_nudge_agent_status ON nudge(agent_id, status)`
- `CREATE INDEX idx_nudge_status ON nudge(status)`

**Contratos API**:
- `GET /api/nudges` — listar nudges. Query params: `status` (default pending), `agent_id` (filtro opcional), `limit` (default 20). Response: `{ nudges: Nudge[], total: int }`
- `PATCH /api/nudges/{id}` — actualizar status de un nudge. Body: `{ status: "seen" | "acted" | "dismissed" }`. Response: `{ nudge: Nudge }`
- `GET /api/nudges/count` — count de nudges pendientes (para badge en navbar). Response: `{ count: int }`

**Pantallas**:
- **Navbar** (componente global AppShell):
  - Badge con count de nudges pendientes (circulo rojo con numero, estilo notificacion). Solo visible si count > 0
  - Click abre panel lateral de notificaciones
- **Panel de Notificaciones** (sidebar o dropdown):
  - Lista de nudges pendientes y vistos recientes, ordenados por fecha
  - Cada nudge muestra: icono por tipo, mensaje, nombre del agente, fecha relativa
  - Acciones por nudge: "Ver" (navega al contexto relevante, marca acted), "Ignorar" (marca dismissed)
  - Boton "Marcar todas como vistas"
  - Empty state: "No hay notificaciones pendientes"

**Reglas**:

| ID | Invariante | Enforcement |
|---|---|---|
| REGLA-83 | Nudges son informativos. Nunca ejecutan acciones automaticas. El usuario siempre decide | NudgeEngine solo crea registros en tabla nudge |
| REGLA-84 | Max 1 nudge pendiente por agent_id + nudge_type. Dedup antes de crear | NudgeEngine._check_dedup con query EXISTS |
| REGLA-85 | NudgeEngine evalua solo agentes habilitados (status=enabled). Agentes deshabilitados no generan nudges excepto idle_agent | NudgeEngine.evaluate con guard |
| REGLA-86 | Thresholds de nudges son configurables via app_config. Si no hay config, usa defaults | NudgeEngine lee app_config, fallback a constantes |
| REGLA-87 | Nudges se purgan automaticamente despues de 90 dias en status dismissed/acted. No requiere Curator (4.4) | NudgeEngine._purge_old al evaluar |

**Archivos a crear/modificar**:
- CREAR `framework/src/datamirai_engine/intelligence/nudge_engine.py` — NudgeEngine + Nudge dataclass + nudge type definitions
- MODIFICAR `framework/src/datamirai_engine/core/runner.py` — invocar NudgeEngine.evaluate post-session (despues de ContextModeler)
- MODIFICAR `app/server/datamirai_app/database.py` — tabla nudge + indices + NudgeRepo (SCHEMA_VERSION bump)
- CREAR `app/server/datamirai_app/routes/nudges.py` — endpoints GET/PATCH nudges
- MODIFICAR `app/server/datamirai_app/app.py` — registrar blueprint de nudges + evaluacion periodica (background task)
- MODIFICAR `app/web/src/lib/api.ts` — funciones client para endpoints de nudges + count
- MODIFICAR `app/web/src/components/ui/AppShell.tsx` — badge de nudges en navbar
- CREAR `app/web/src/components/nudges/NudgePanel.tsx` — panel lateral de notificaciones

---

### 4.4 — Curator (Poda de Playbook + Trazas)

**Problema**: Con el uso prolongado, los agentes acumulan debt operativo: reglas de playbook que ya no aplican (harmful_count > helpful_count), reglas duplicadas que dicen lo mismo con palabras distintas, y trazas de ejecucion que crecen indefinidamente. No hay mecanismo automatizado de limpieza.

**Solucion**: Proceso background periodico (configurable, default semanal) que ejecuta tres acciones de mantenimiento: desactiva reglas daninas, fusiona reglas duplicadas, y poda trazas antiguas. Genera un reporte de lo que hizo. El usuario puede ejecutarlo manualmente o ver historial de runs.

**Arquitectura**:
- `CuratorProcess` — componente del framework que ejecuta el ciclo de mantenimiento
  - Metodo principal: `run(config: CuratorConfig) -> CuratorReport`
  - `CuratorConfig`:
    - `rule_prune_enabled: bool` — activar poda de reglas (default true)
    - `rule_merge_enabled: bool` — activar fusion de duplicadas (default true)
    - `trace_prune_enabled: bool` — activar poda de trazas (default true)
    - `trace_retention_days: int` — dias de retencion (default 90)
    - `rule_harmful_threshold: float` — ratio harmful/helpful para desactivar (default 1.0 — desactiva cuando harmful > helpful)
    - `rule_similarity_threshold: float` — cosine similarity para considerar duplicadas (default 0.9)
  - `CuratorReport`:
    - `rules_disabled: int` — reglas desactivadas
    - `rules_disabled_details: list[dict]` — detalle de cada regla desactivada (id, content, harmful_count, helpful_count)
    - `rules_merged: int` — pares de reglas fusionadas
    - `rules_merged_details: list[dict]` — detalle de cada fusion (ids originales, regla resultante)
    - `traces_pruned: int` — trazas eliminadas
    - `traces_pruned_by_agent: dict[str, int]` — breakdown por agente
    - `run_duration_ms: int` — duracion del proceso
- Tres acciones de mantenimiento:
  1. **Poda de reglas daninas**:
     - Query: reglas donde `harmful_count > helpful_count * rule_harmful_threshold`
     - Accion: cambiar status a `disabled`
     - No elimina — solo desactiva. El usuario puede reactivar manualmente
  2. **Fusion de reglas duplicadas**:
     - Compara reglas activas entre si por similitud semantica de content
     - Si cosine similarity > `rule_similarity_threshold` → fusiona
     - La regla mas antigua sobrevive. La mas nueva se marca `merged_into: {id_superviviente}`
     - `helpful_count` y `harmful_count` se suman en la superviviente
     - Usa embeddings de FEAT-002 para comparacion. Fallback: FTS similarity
  3. **Poda de trazas antiguas**:
     - Elimina registros de `sessions.trace` y `sessions.transcript` donde `finished_at < now() - retention_days`
     - Tambien elimina `execution_spans` de sessions antiguas
     - NO elimina la session misma — solo los campos pesados (trace, transcript). La session queda como registro ligero con status, duration, result
     - Libera espacio en SQLite via `VACUUM` post-poda (si se eliminaron > 1000 registros)
- Scheduling:
  - Default: semanal (configurable via app_config)
  - Implementacion: background task en el server FastAPI con `asyncio.create_task`
  - Timer verifica cada hora si toca ejecutar (basado en `last_run_at` + intervalo)
  - Tambien disponible via API para ejecucion manual

**Entidades nuevas**:

Tabla `curator_run` en SQLite:

| Columna | Tipo | Constraint |
|---|---|---|
| id | TEXT | PK |
| run_at | TEXT | NOT NULL |
| trigger | TEXT | NOT NULL DEFAULT 'scheduled' (scheduled / manual) |
| config_used | TEXT | JSON, NOT NULL |
| rules_disabled | INTEGER | NOT NULL DEFAULT 0 |
| rules_merged | INTEGER | NOT NULL DEFAULT 0 |
| traces_pruned | INTEGER | NOT NULL DEFAULT 0 |
| report | TEXT | JSON, NOT NULL DEFAULT '{}' |
| duration_ms | INTEGER | NOT NULL DEFAULT 0 |
| created_at | TEXT | NOT NULL |

Indice:
- `CREATE INDEX idx_curator_run_at ON curator_run(run_at DESC)`

**Contratos API**:
- `POST /api/curator/run` — ejecutar curator manualmente. Body opcional: `{ config?: CuratorConfig }` (usa defaults si no se pasa). Response: `{ run: CuratorRun }` con reporte completo.
- `GET /api/curator/history` — historial de runs. Query params: `limit` (default 10). Response: `{ runs: CuratorRun[] }`
- `GET /api/curator/config` — obtener configuracion actual del curator. Response: `{ config: CuratorConfig }`
- `PATCH /api/curator/config` — actualizar configuracion. Body: campos parciales de CuratorConfig. Response: `{ config: CuratorConfig }`

**Pantallas**:
- **Settings** (`/settings`) — nueva seccion "Curator":
  - Toggles para cada accion (poda reglas, fusion, poda trazas)
  - Slider/input para retention days (default 90)
  - Selector de frecuencia (diario / semanal / mensual / desactivado)
  - Boton "Ejecutar ahora" que dispara POST /api/curator/run con loading state
  - Historial de runs como tabla: fecha, trigger (scheduled/manual), reglas desactivadas, reglas fusionadas, trazas podadas, duracion
  - Click en run expande reporte completo con detalles
  - Empty state: "El Curator no se ha ejecutado aun."

**Reglas**:

| ID | Invariante | Enforcement |
|---|---|---|
| REGLA-88 | Curator NUNCA elimina reglas. Solo desactiva (status=disabled) o fusiona (merged_into). Las reglas son recuperables | CuratorProcess — UPDATE status, no DELETE |
| REGLA-89 | Curator NUNCA elimina sessions completas. Solo limpia campos pesados (trace, transcript) de sessions > retention_days. La session queda como registro ligero | CuratorProcess._prune_traces — UPDATE SET trace='[]', transcript='[]' |
| REGLA-90 | Curator NUNCA archiva ni elimina agentes. Solo mantiene reglas y trazas | CuratorProcess — scope limitado a playbook_rules + sessions + execution_spans |
| REGLA-91 | Fusion de reglas es atomica. Ambas reglas se actualizan en la misma transaccion (superviviente recibe counts, fusionada se marca merged_into) | Transaccion SQLite |
| REGLA-92 | VACUUM solo se ejecuta si se eliminaron > 1000 registros en la poda de trazas. VACUUM es costoso y no debe ejecutarse en cada run | CuratorProcess._maybe_vacuum con threshold |
| REGLA-93 | Curator run es idempotente. Si se ejecuta dos veces seguidas, la segunda no hace nada (ya no hay reglas daninas ni trazas viejas) | Queries con condiciones estrictas, no side effects en datos limpios |
| REGLA-94 | El scheduler del curator no bloquea el event loop. Se ejecuta como background task async | asyncio.create_task, no blocking |

**Archivos a crear/modificar**:
- CREAR `framework/src/datamirai_engine/intelligence/curator.py` — CuratorProcess + CuratorConfig + CuratorReport dataclasses
- MODIFICAR `app/server/datamirai_app/database.py` — tabla curator_run + indice + CuratorRunRepo (SCHEMA_VERSION bump)
- CREAR `app/server/datamirai_app/routes/curator.py` — endpoints POST run, GET history, GET/PATCH config
- MODIFICAR `app/server/datamirai_app/app.py` — registrar blueprint de curator + background task para scheduling
- MODIFICAR `app/web/src/lib/api.ts` — funciones client para endpoints del curator
- CREAR `app/web/src/app/settings/curator/page.tsx` — pagina UI de configuracion y historial del Curator (o seccion dentro de Settings existente)

---

## Dependencias entre features

```
4.1 (Context Modeling) ← depende de FEAT-002 (HybridSearch para equivalencia semantica, LLM adapters para extraccion)
4.2 (Tool Scoring)     ← depende de FEAT-002 (LLM adapters para embeddings). Independiente de 4.1
4.3 (Nudge System)     ← depende de 4.1 (nudge type unreviewed_insights usa conclusiones). Parcialmente independiente
4.4 (Curator)          ← depende de FEAT-002 (embeddings para fusion de reglas). Independiente de 4.1/4.2/4.3
```

Orden de implementacion sugerido: 4.1 → 4.2 → 4.3 → 4.4

Razon:
- 4.1 primero porque 4.3 lo necesita para unreviewed_insights
- 4.2 es independiente pero usa el mismo patron de embeddings que 4.1 — implementar despues reutiliza infrastructure
- 4.3 despues de 4.1 para tener el nudge de insights disponible
- 4.4 al final porque es el mas independiente y su scheduler puede integrarse limpiamente

---

## Entidades nuevas (resumen consolidado)

### context_conclusion
| Columna | Tipo | Constraint |
|---|---|---|
| id | TEXT | PK |
| agent_id | TEXT | FK agents(id) ON DELETE CASCADE, NOT NULL |
| conclusion_type | TEXT | NOT NULL (explicit / deductive / inductive / contradiction) |
| content | TEXT | NOT NULL |
| confidence | REAL | NOT NULL DEFAULT 0.5 |
| source_session_ids | TEXT | JSON array, NOT NULL DEFAULT '[]' |
| times_reinforced | INTEGER | NOT NULL DEFAULT 0 |
| contradicts_id | TEXT | FK context_conclusion(id), NULL |
| status | TEXT | NOT NULL DEFAULT 'active' |
| embedding | BLOB | NULL |
| created_at | TEXT | NOT NULL |
| updated_at | TEXT | NOT NULL |

### nudge
| Columna | Tipo | Constraint |
|---|---|---|
| id | TEXT | PK |
| agent_id | TEXT | FK agents(id) ON DELETE CASCADE, NULL |
| nudge_type | TEXT | NOT NULL |
| message | TEXT | NOT NULL |
| metadata | TEXT | JSON, DEFAULT '{}' |
| status | TEXT | NOT NULL DEFAULT 'pending' |
| created_at | TEXT | NOT NULL |

### curator_run
| Columna | Tipo | Constraint |
|---|---|---|
| id | TEXT | PK |
| run_at | TEXT | NOT NULL |
| trigger | TEXT | NOT NULL DEFAULT 'scheduled' |
| config_used | TEXT | JSON, NOT NULL |
| rules_disabled | INTEGER | NOT NULL DEFAULT 0 |
| rules_merged | INTEGER | NOT NULL DEFAULT 0 |
| traces_pruned | INTEGER | NOT NULL DEFAULT 0 |
| report | TEXT | JSON, NOT NULL DEFAULT '{}' |
| duration_ms | INTEGER | NOT NULL DEFAULT 0 |
| created_at | TEXT | NOT NULL |

---

## Maquinas de estado

### Conclusion
```
active → superseded (contradiccion detectada)
active → dismissed (usuario descarta)
dismissed → active (usuario reactiva — via PATCH)
```

### Nudge
```
pending → seen (usuario lo vio)
pending → dismissed (usuario lo ignora)
seen → acted (usuario hizo click en accion)
seen → dismissed (usuario lo ignora despues de ver)
```

No hay transiciones inversas. Nudges dismissed/acted son terminales (eventualmente se purgan a los 90 dias por REGLA-87).

---

## Reglas de negocio nuevas (consolidado)

| ID | Invariante | Enforcement |
|---|---|---|
| REGLA-71 | ContextModeler solo en sessions completadas | Post-session guard |
| REGLA-72 | Equivalencia semantica = cosine > 0.85 | Threshold configurable |
| REGLA-73 | Contradiction → superseded atomicamente | Transaccion SQLite |
| REGLA-74 | Confidence max 1.0. Formula: min(1.0, current + 0.1) | ContextModeler._reinforce |
| REGLA-75 | Max 20 conclusiones inyectadas en llm_call | Context Compiler LIMIT |
| REGLA-76 | Context injection opt-in por nodo (default true) | Config campo inject_context |
| REGLA-77 | Sin embeddings → conclusiones sin dedup. Warning en log | Fallback graceful |
| REGLA-78 | Tool scoring solo en design-time | Endpoint separado |
| REGLA-79 | Sin embeddings → keyword matching fallback. Nunca falla | ToolScorer fallback |
| REGLA-80 | Tool embeddings cacheados en memoria. Invalidacion por cambio en registry | In-memory cache |
| REGLA-81 | Scores en [0, 1] | Normalizacion |
| REGLA-82 | /api/tools/score stateless, no requiere agent_id | API design |
| REGLA-83 | Nudges son informativos, nunca ejecutan acciones | NudgeEngine read-only |
| REGLA-84 | Max 1 nudge pending por agent_id + nudge_type | Dedup query |
| REGLA-85 | Solo agentes enabled generan nudges (excepto idle_agent) | Guard |
| REGLA-86 | Thresholds configurables via app_config con defaults | Config + fallback |
| REGLA-87 | Nudges dismissed/acted se purgan a los 90 dias | NudgeEngine._purge_old |
| REGLA-88 | Curator NUNCA elimina reglas. Solo desactiva o fusiona | UPDATE, no DELETE |
| REGLA-89 | Curator NUNCA elimina sessions. Solo limpia trace/transcript | UPDATE campos pesados |
| REGLA-90 | Curator NUNCA archiva ni elimina agentes | Scope limitado |
| REGLA-91 | Fusion de reglas atomica | Transaccion SQLite |
| REGLA-92 | VACUUM solo si > 1000 registros eliminados | Threshold check |
| REGLA-93 | Curator run idempotente | Queries condicionales |
| REGLA-94 | Scheduler del curator no bloquea event loop | asyncio background task |

---

## Notas de implementacion

- **ContextModeler usa el LLM provider configurado** (FEAT-002). El prompt de extraccion va al adapter default. Si no hay LLM configurado, el modeler no se ejecuta (skip silencioso, warning en log).
- **Tool Scoring embedding cache** es un dict en memoria del server. Con ~20 tools y embeddings de 1536 dims, son ~120KB. Irrelevante en memoria.
- **NudgeEngine evaluacion periodica** se implementa como background task en el lifespan de FastAPI. Usa `asyncio.sleep` entre evaluaciones. Si el server se reinicia, el timer se resetea — no hay persistencia del timer.
- **Curator y playbook rules**: El Curator asume que existe una tabla `playbook_rules` con campos `harmful_count`, `helpful_count`, `status`, `content`. Si esta tabla no existe aun en el schema (depende del tier de Reflexion/Playbook), el Curator skipea la poda de reglas y solo ejecuta poda de trazas.
- **SCHEMA_VERSION**: Este PRD bumps de version 2 a 3 (o el que sea correcto al momento de implementar). Las 3 tablas nuevas (context_conclusion, nudge, curator_run) se crean en una sola migracion.
- **Framework vs App**: `ContextModeler`, `ToolScorer`, `NudgeEngine` y `CuratorProcess` viven en `framework/src/datamirai_engine/intelligence/`. Son componentes del engine reutilizables. Las rutas API y la UI viven en `app/`.

---

## doc_refs
- `docs/prd/draft/DATAMIRAI-ENGINE-PRD.md` — PRD base del engine
- `docs/ARCHITECTURE.md` — stack, convenciones
- `docs/prd/draft/FEAT-001.md` — MVP features (context: streaming, AI assistant, session control)
- `docs/prd/draft/FEAT-002.md` — Tier 1 (context: LLM adapters, memory persistence, hybrid search)
- `docs/producto/FLUJOS.md` — maquinas de estado, reglas existentes
