# Data Mirai Engine — PRD y Arquitectura Tecnica

**Repositorio**: `datamirai-engine` (publico / open source)
**Fecha**: 2026-04-30 (inicio) | 2026-05-01 (arquitectura cerrada)
**Estado**: Arquitectura completa. Pendiente: implementacion
**Autor**: Gabriel

---

## Que es Data Mirai Engine

Data Mirai Engine es un **motor propio de ejecucion de grafos agentivos**, escrito en Python. Es una alternativa a LangGraph y Google ADK, sin dependencias de frameworks externos ni vendor lock-in.

El engine permite definir agentes como grafos visuales compuestos por bloques reutilizables (tipo Lego), ejecutarlos de forma autonoma con memoria persistente, y conectarlos a cualquier recurso (bases de datos, storage, LLMs, APIs externas).

Es el core del producto Data Mirai Universes, pero funciona como **libreria Python standalone** — no necesita Kubernetes, Temporal, ni ningun componente de la version Cloud para operar.

**Modelo probado**: mismo patron que Supabase, GitLab, N8N — core abierto, cloud privado.

### Que contiene este repositorio

- Motor de ejecucion de grafos y agentes (Python)
- Sistema de Bloques con interfaz BlockSpec
- Editor Visual (React Flow + logica de grafos)
- Runtime de agentes
- Sistema de triggers (webhook, schedule, event, manual)
- Sistema de memoria (corto plazo, largo plazo, bitacora)
- Definicion de interfaces y abstracciones
- Helm Chart basico para despliegue self-hosted
- Documentacion para desarrolladores

### Que puede hacer un usuario self-hosted

- Instalar y ejecutar sus propios universos de forma independiente
- Conectar sus propios recursos (DB, storage, LLMs)
- Crear bloques custom y grafos
- Control total sobre la infraestructura

---

## Glosario

| Concepto | Definicion |
|---|---|
| **Bloque** | Pieza reutilizable tipo Lego. Una accion ejecutable que se conecta visualmente con otras para formar un grafo |
| **Grafo** | Flujo visual creado al conectar bloques. Representa la logica completa de un agente |
| **Agente** | Un grafo desplegado que tiene memoria y trabaja de forma continua |
| **Trigger** | Punto de entrada que activa la ejecucion de un agente (webhook, cron, evento de recurso, manual) |
| **Memoria corto plazo** | Contexto actual del agente — la traza de ejecucion en curso |
| **Memoria largo plazo** | Historial de actividades y aprendizajes del agente — persiste entre ejecuciones |
| **BlockSpec** | Interfaz declarativa que define identidad, inputs, outputs, config y retry policy de un bloque |
| **ExecutionContext** | Objeto que da acceso a los recursos disponibles (DB, storage, LLM, memoria, auth, vector) |
| **BlockRegistry** | Catalogo de bloques disponibles con autodiscovery por convencion de directorio |
| **SharedState** | Diccionario global donde cada bloque guarda su output, indexado por ID de nodo |
| **Version (Bloque)** | Version semantica de un bloque. Patch/minor/major indican compatibilidad |
| **Version (Agente)** | Snapshot versionado de un grafo desplegado |

---

## Principio de Diseno

**Atomicidad por capas con composicion**: bloques atomicos en la base, composicion hacia arriba.
Cada capa trabaja con los recursos disponibles en su nivel. El usuario compone piezas pequenas
para construir comportamientos complejos — no necesita entender la complejidad interna.

---

## Arquitectura Tecnica

Motor propio de ejecucion de grafos agentivos. Alternativa a LangGraph y Google ADK.
Sin dependencias de frameworks externos.

### Modelo de ejecucion

La ejecucion es un **cursor secuencial con branching condicional**:

1. El cursor empieza en el nodo trigger
2. Ejecuta el bloque actual
3. Mira las edges de salida
4. Evalua condiciones contra el output del bloque
5. Sigue la edge cuya condicion sea true (solo UNA)
6. Ejecuta el siguiente bloque
7. Repite hasta que no haya mas edges

**No hay ejecucion paralela.** Solo un camino se ejecuta a la vez.
Las bifurcaciones son condicionales: si A produce X va por un camino; si produce Y va por otro.
Los caminos pueden converger (Merge) o terminar independientemente.

### Estructura de un grafo

```json
{
  "id": "grafo-001",
  "name": "Procesar Reunion",
  "version": "1.0",
  "nodes": [
    {"id": "n1", "block_type": "trigger/file_upload", "config": {}},
    {"id": "n2", "block_type": "logic/condition", "config": {"field": "n1.file_type", "operator": "==", "value": "audio"}},
    {"id": "n3", "block_type": "ai/transcribe", "config": {}},
    {"id": "n4", "block_type": "ai/extract_audio", "config": {}},
    {"id": "n5", "block_type": "logic/merge", "config": {}},
    {"id": "n6", "block_type": "ai/llm_call", "config": {"prompt_template": "..."}}
  ],
  "edges": [
    {"from": "n1", "to": "n2"},
    {"from": "n2", "to": "n3", "condition": "n2.output.result == true"},
    {"from": "n2", "to": "n4", "condition": "n2.output.result == false"},
    {"from": "n3", "to": "n5"},
    {"from": "n4", "to": "n5"},
    {"from": "n5", "to": "n6", "data_map": {"n5.output.texto": "n6.input.content"}}
  ]
}
```

### Data passing: estado compartido con named outputs

Cada bloque produce un output que se guarda en un diccionario global bajo su ID:

```
State durante ejecucion:
{
  "n1": { "output": { "file": "reunion.mp3", "file_type": "audio", "size": 15000 } },
  "n3": { "output": { "texto": "Hola, en esta reunion vamos a..." } },
  "n6": { "output": { "resumen": "...", "action_items": [...] } }
}
```

Las edges definen mapeos de datos entre bloques:
- `n1.output.file` → `n3.input.audio_file`
- `n3.output.texto` → `n6.input.content`

En el editor visual, esto se ve como flechitas entre bloques con los datos que pasan.

### Bloques logicos (puertas logicas)

Son bloques del catalogo, no features especiales del engine:

| Bloque | Funcion | Edges de salida |
|---|---|---|
| **Condition (IF-ELSE)** | Evalua condicion, rutea a una de dos salidas | 2: true, false |
| **Switch (Router)** | Evalua valor, rutea a una de N salidas | N: una por caso |
| **Loop** | Repite seccion hasta condicion | 2: continuar, salir |
| **Merge** | Punto de convergencia de multiples caminos | 1 |
| **Wait/Delay** | Pausa ejecucion por tiempo definido | 1 |

Con estas 5 puertas + bloques de accion = todas las permutaciones logicas posibles.

### Manejo de errores y reintentos

Cada bloque puede tener su propia retry policy:

```json
{
  "retry_policy": {
    "max_retries": 3,
    "backoff": "exponential",
    "initial_delay_seconds": 1,
    "on_failure": "stop | skip | route_to_error"
  }
}
```

Comportamiento cuando un bloque falla despues de agotar retries:
- **stop**: detiene el grafo entero (fail-fast)
- **skip**: salta el bloque, continua con valor por defecto
- **route_to_error**: sigue una edge especial hacia un bloque de manejo de error

### Ciclos agentivos (loops)

Un agente que piensa en loop usa edges condicionales que regresan a nodos anteriores:

```
[Recibir tarea] → [Pensar (LLM)] → [Actuar] → [Verificar] ──┐
                       ▲                                       │
                       │         done == false                 │
                       └───────────────────────────────────────┘
                                 done == true → [Reportar]
```

Safeguard: `max_iterations` configurable para evitar loops infinitos.

---

## BlockSpec — Interfaz de un Bloque

Cada bloque debe declarar su identidad, contrato de datos, configuracion y politica de errores.

### Declaracion YAML

```yaml
# Identidad
block_type: "ai/llm_call"           # categoria/nombre
version: "1.0.0"                     # semantico
display_name: "Llamar IA"           # nombre visual en el catalogo
description: "Envia un prompt a un modelo de IA"
category: "ai"                       # para agrupar en el catalogo
icon: "brain"                        # icono en el editor

# Contrato de datos
inputs:
  - name: "prompt"
    type: "string"
    required: true
    description: "El texto que se envia al modelo"
  - name: "context"
    type: "string"
    required: false
    description: "Contexto adicional para el modelo"

outputs:
  - name: "response"
    type: "string"
    description: "La respuesta del modelo"
  - name: "tokens_used"
    type: "number"
    description: "Tokens consumidos"

# Configuracion (panel lateral del editor)
config:
  - name: "model"
    type: "select"
    options: ["Claude Sonnet", "GPT-4o", "Gemini Flash"]
    default: "Claude Sonnet"
  - name: "temperature"
    type: "slider"
    min: 0.0
    max: 1.0
    default: 0.7
  - name: "max_tokens"
    type: "number"
    default: 1000

# Politica de errores
retry_policy:
  max_retries: 3
  backoff: "exponential"
  initial_delay_seconds: 1
  on_failure: "stop"
```

### Implementacion en Python

```python
class LLMCallBlock(BaseBlock):
    spec = BlockSpec("ai/llm_call", "1.0.0", inputs=[...], outputs=[...], config=[...])

    async def execute(self, inputs: dict, config: dict, context: ExecutionContext) -> dict:
        response = await context.llm.call(
            model=config["model"],
            prompt=inputs["prompt"],
            context=inputs.get("context"),
            temperature=config["temperature"],
            max_tokens=config["max_tokens"]
        )
        return {
            "response": response.text,
            "tokens_used": response.tokens
        }
```

---

## ExecutionContext — Acceso a Recursos

El `context` (ExecutionContext) da acceso a los recursos disponibles para el agente. Es agnostico al origen de los recursos — funciona igual si son provistos por Data Mirai Cloud, externos del cliente, o self-hosted.

| Propiedad | Funcion |
|---|---|
| `context.db` | Conexion a PostgreSQL |
| `context.vector` | pgvector para embeddings |
| `context.storage` | Cliente S3 (R2, MinIO, o cualquier S3-compatible) |
| `context.llm` | Cliente LLM (model-agnostic, cualquier provider) |
| `context.memory` | Memoria del agente (corto y largo plazo) |
| `context.auth` | Informacion del caller (user_id, role, org) |

### Ejemplo de uso

```python
# Acceso a DB
rows = await context.db.query("SELECT * FROM orders WHERE status = 'pending'")

# Acceso a storage
file = await context.storage.get("reuniones/2026-05-01.mp3")

# Acceso a LLM
response = await context.llm.call(model="claude-sonnet", prompt="Resumir este texto...")

# Acceso a memoria
context.memory.short_term.log("Decidio usar modelo Claude porque el prompt era largo")
learnings = context.memory.long_term.search("como manejar archivos grandes")

# Acceso a auth
caller = context.auth.get_caller()
# { user_id: "usr-123", role: "user", org: "acme" }

# Acceso a vector
results = await context.vector.search(embedding, limit=10)
```

---

## Tres Modos de Conexion a Recursos

Como los bloques acceden a recursos depende de la configuracion:

| Modo | Recursos DB/Storage | Como se conectan los bloques |
|---|---|---|
| **Recursos internos** | Provistos por la plataforma (Postgres, R2) | El ExecutionContext tiene las credenciales. Bloques acceden directo |
| **Conectores externos** | Externos (DB del cliente, APIs) | Bloques "connector" con credenciales configuradas por el usuario |
| **Self-hosted (puente)** | Lo que el cliente instale | Bloques de puente para conectarse a cualquier servicio |

Los bloques de datos (DB read/write, storage) son **agnosticos al origen**. Funcionan igual si la DB
es provista internamente o es externa. La diferencia es de donde vienen las credenciales:

- **Recursos internos**: el context las inyecta automaticamente
- **Recursos externos**: el usuario las configura en el panel del bloque (connection string, API key, etc.)

Dentro del bloque, el usuario puede especificar:
- A que recurso acceder (cual DB, cual bucket)
- A que ruta/tabla/coleccion
- Filtros por columnas/filas (que datos leer/escribir)
- Con que credenciales (si es externo)

---

## BlockRegistry — Carga Dinamica de Bloques

El engine tiene un registro de bloques disponibles con autodiscovery por convencion de directorio:

```python
registry = BlockRegistry()
registry.discover("datamirai_engine.blocks.builtin")   # bloques incluidos
registry.discover("my_custom_blocks")                 # bloques del usuario (plugins)
```

Los bloques se descubren automaticamente por convencion de directorio.
En el editor visual, el catalogo muestra todos los bloques registrados con su metadata (display_name, description, category, icon, inputs, outputs).

El sistema es extensible: cualquier desarrollador puede crear bloques custom empaquetados como modulos Python y registrarlos via `registry.discover()`.

---

## Triggers — Sistema de Activacion de Agentes

Un agente puede tener **multiples triggers** (puntos de entrada). No esta limitado a uno solo.
Todos son bloques del catalogo con la misma interfaz BlockSpec.

### Triggers v1

| Trigger | Descripcion | Output principal |
|---|---|---|
| **trigger/webhook** | Endpoint HTTP del agente. Recibe requests externos | body, headers, query_params |
| **trigger/schedule** | Cron/intervalo. Se ejecuta periodicamente | triggered_at, run_count |
| **trigger/event** | Evento de recurso (archivo subido, fila insertada) | metadata del evento (file_name, row, etc.) |
| **trigger/manual** | Click en "Ejecutar" desde el dashboard. Formulario opcional | user_input, triggered_by |

Webhook y API call al agente son lo mismo — ambos son HTTP request al endpoint del agente.

### Multiples triggers en un grafo

Cada trigger es un nodo de entrada independiente. Todos convergen al primer bloque de logica:

```
[Webhook POST]──────┐
                     │
[Cron cada 1h]───────┤
                     ├──→ [Bloque A] → [Bloque B] → ...
[Archivo subido]─────┤
                     │
[Manual]─────────────┘
```

El runner sabe cual trigger se activo y ejecuta desde ese nodo.

### Configuracion de cada trigger

**Webhook**:
- method: POST/GET/PUT
- path: string (genera endpoint accesible)
- auth: none / api_key / bearer_token

**Schedule**:
- mode: interval ("cada 5 min") o cron expression
- timezone (default UTC)
- UI amigable: selector visual "cada __ minutos/horas/dias"

**Event**:
- source: storage / database
- event_type: file_uploaded, file_deleted, row_inserted, row_updated, row_deleted
- filter: prefix, extension (storage) o table, column, value (database)

**Manual**:
- input_form: campos opcionales que la UI muestra al hacer click en "Ejecutar"

### Regla de atomicidad: eventos los emite el recurso, no el proceso

**El storage emite el evento, no el proceso que sube el archivo.**

Razon: si el trigger dependiera de "quien subio", cada camino de upload (UI, otro agente, API directa)
tendria que saber que debe disparar algo. Eso acopla y rompe la atomicidad.

En cambio, el storage (MinIO/R2) emite un bucket notification cuando un objeto se crea.
No importa COMO llego el archivo — el evento se dispara siempre.

```
[Cualquier fuente] → archivo llega al storage → Bucket Notification → trigger/event → Grafo
```

**Cada pieza hace una sola cosa**: el storage almacena, el trigger detecta.

Esto aplica tambien a la base de datos: PostgreSQL LISTEN/NOTIFY emite eventos cuando hay cambios
en tablas. El trigger/event los detecta sin importar quien hizo el cambio.

Implementacion: MinIO bucket notifications via webhook al pod del agente.

### Implementacion tecnica por trigger

| Trigger | Como vive | Infra necesaria |
|---|---|---|
| **Webhook** | Pod del agente con FastAPI mini escuchando HTTP | Ingress/Service |
| **Schedule** | CronJob (standalone) o Temporal Schedule (Cloud) | Scheduler |
| **Event** | MinIO bucket notification (storage) o PG LISTEN/NOTIFY (DB) → webhook al pod | Configuracion de notifications |
| **Manual** | API call al pod del agente | Endpoint interno |

---

## Memoria de Agentes

Dos niveles de memoria + una bitacora compartida.

### Memoria a corto plazo (por sesion/ejecucion)

Es el **detalle paso a paso** de la ejecucion actual:

- Estado del grafo en cada momento (outputs de cada bloque)
- Decisiones tomadas en cada bifurcacion (por que se fue por un camino y no otro)
- Errores encontrados y como se manejaron (retry, skip, etc.)
- Metricas por bloque (tiempo, tokens usados, etc.)

**Vive en memoria (RAM) durante la ejecucion.** Al terminar la sesion, se persiste como registro de detalle en Postgres.

Acceso via `context.memory.short_term`:
```python
context.memory.short_term.log("Decidio usar modelo Claude porque el prompt era largo")
context.memory.short_term.get_trace()  # traza completa de la sesion
```

### Memoria a largo plazo (persiste entre sesiones)

Son las **conclusiones y aprendizajes** de cada sesion:

- Resumen de que hizo el agente en esa sesion
- Decisiones clave que tomo y por que
- Aprendizajes: que funciono, que no, que ajustar
- Patrones detectados sobre el tiempo

Al **iniciar una nueva sesion**, el agente lee su memoria a largo plazo para informar la toma de decisiones.
Es como un humano que revisa sus notas antes de empezar a trabajar.

**Se persiste en Postgres** (datos estructurados) y **pgvector** (busqueda semantica por relevancia):

- Postgres: historial cronologico de sesiones, decisiones, metricas
- pgvector: embeddings de aprendizajes para que el agente busque por relevancia, no solo cronologicamente

Acceso via `context.memory.long_term`:
```python
# Al terminar una sesion
context.memory.long_term.save_learning(
    session_id="sess-001",
    summary="Proceso 15 reuniones. Los archivos MP3 >50MB fallan en transcripcion directa, mejor dividirlos.",
    decisions=["Dividir archivos >50MB antes de transcribir"],
    tags=["transcripcion", "limites", "optimizacion"]
)

# Al iniciar una nueva sesion
learnings = context.memory.long_term.search("como manejar archivos grandes")
recent = context.memory.long_term.get_recent(limit=10)
```

### Bitacora (memoria compartida con autoria)

Todos los agentes que comparten recursos escriben en una **bitacora unica compartida**. Cada entrada tiene un **autor** (que agente la escribio).

No es memoria aislada por agente ni memoria compartida sin dueno.
Es un **journal con autoria**, similar a un log de git donde cada commit tiene autor.

```
Bitacora:

[2026-05-01 14:30] Agente: transcriptor-v2
  Sesion: sess-045
  General: Proceso 8 reuniones. 7 exitosas, 1 fallo por formato no soportado (.ogg)
  Aprendizaje: Agregar validacion de formato antes de transcribir

[2026-05-01 15:00] Agente: analizador-v1
  Sesion: sess-022
  General: Analizo 7 transcripciones. Detecto 3 reuniones con conflictos no resueltos.
  Aprendizaje: Cuando hay conflictos, generar alerta al usuario, no solo registrar.

[2026-05-01 16:00] Agente: transcriptor-v2
  Sesion: sess-046
  General: Recibio archivo .ogg. Leyo aprendizaje de sess-045. Rechazo con mensaje claro al usuario.
  Aprendizaje: La validacion de formato funciona. Reducir mensajes de error a algo mas amigable.
```

**Cualquier agente puede leer toda la bitacora** — pero cada entrada es de un autor especifico.
Esto permite:
- Coordinacion entre agentes sin acoplamiento directo
- Trazabilidad completa de todo lo que ha pasado
- Un agente puede aprender de la experiencia de otro agente

### Dos granularidades de consulta

| Granularidad | Que contiene | Donde vive | Para que sirve |
|---|---|---|---|
| **General (por sesion)** | Resumen, decisiones clave, aprendizajes | Postgres + pgvector | Iniciar nueva sesion informada, buscar patrones |
| **Detalle (por sesion)** | Traza paso a paso de cada bloque | Postgres | Debugging, auditoria, entender que paso exactamente |

### Schema en Postgres

```sql
-- Bitacora (general por sesion)
CREATE TABLE universe_memory (
    id UUID PRIMARY KEY,
    agent_id TEXT NOT NULL,          -- autor
    agent_version TEXT NOT NULL,
    session_id TEXT NOT NULL,
    started_at TIMESTAMP,
    finished_at TIMESTAMP,
    summary TEXT,                     -- resumen general de la sesion
    decisions JSONB,                  -- decisiones clave
    learnings TEXT[],                 -- aprendizajes
    tags TEXT[],                      -- para busqueda
    embedding VECTOR(1536)            -- para busqueda semantica
);

-- Detalle por sesion (corto plazo persistido)
CREATE TABLE session_trace (
    id UUID PRIMARY KEY,
    session_id TEXT NOT NULL,
    agent_id TEXT NOT NULL,
    block_id TEXT NOT NULL,
    block_type TEXT NOT NULL,
    started_at TIMESTAMP,
    finished_at TIMESTAMP,
    inputs JSONB,
    outputs JSONB,
    status TEXT,                      -- success, error, skipped
    error_message TEXT,
    retry_count INT DEFAULT 0,
    decision_reason TEXT              -- por que tomo este camino
);
```

---

## Versionamiento de Bloques

Los bloques tienen version semantica (1.0, 1.1, 2.0):

- **Patch (1.0 → 1.0.1)**: fix interno, compatible. El usuario no nota nada
- **Minor (1.0 → 1.1)**: nuevo output o config opcional. Compatible con grafos existentes
- **Major (1.0 → 2.0)**: cambio en inputs/outputs. Requiere reconexion

Cuando un bloque tiene upgrade disponible, la UI del editor muestra:
- Badge en el bloque indicando update disponible
- Preview de los cambios (que es nuevo, que cambio, que se removio)
- Si es compatible → boton "Actualizar" directo
- Si requiere cambios → wizard guiado mostrando que edges reconectar

---

## Estructura del Repositorio

```
datamirai-engine/
  ├── core/
  │     ├── graph.py          ← GraphDef: estructura del grafo
  │     ├── runner.py         ← GraphRunner: cursor que ejecuta el grafo
  │     ├── state.py          ← SharedState: datos entre bloques
  │     └── context.py        ← ExecutionContext: recursos disponibles
  ├── blocks/
  │     ├── base.py           ← BlockSpec: interfaz de todo bloque
  │     ├── registry.py       ← BlockRegistry: catalogo de bloques
  │     └── builtin/          ← Bloques incluidos
  │           ├── logic/      ← Condition, Switch, Loop, Merge, Wait
  │           ├── ai/         ← LLM call, transcribe, embeddings
  │           ├── data/       ← DB read/write, storage read/write
  │           └── notify/     ← Email, webhook, notificacion
  ├── triggers/
  │     ├── webhook.py        ← Trigger por HTTP
  │     ├── schedule.py       ← Trigger por cron
  │     └── event.py          ← Trigger por evento
  ├── memory/
  │     ├── short_term.py     ← Memoria de contexto
  │     └── long_term.py      ← Memoria persistente
  └── editor/                 ← React Flow + logica visual
```

---

## Despliegue Self-Hosted

El repositorio incluye un **Helm Chart basico** para despliegue en Kubernetes. Permite instalar el engine con sus dependencias minimas (PostgreSQL + pgvector, storage S3-compatible) en cualquier cluster.

El Helm Chart es intencionalmente simple — cubre el caso de uso standalone. La orquestacion avanzada (multi-tenancy, provisioning automatizado, ambientes) es responsabilidad de la version Cloud.

---

## Relacion con Temporal

Temporal NO es parte del engine. Es una capa de infraestructura **opcional** para la version Cloud:

```
Self-hosted:   Trigger → Data MiraiEngine.run(grafo) → Done
Cloud:         Trigger → Temporal Workflow → Data MiraiEngine.run(grafo) → Temporal registra resultado
```

El engine es una libreria Python standalone. No sabe si Temporal existe.

Temporal agrega para Cloud:
- Scheduling avanzado (cron)
- Durabilidad (si el pod crashea, Temporal reanuda)
- Historial de ejecuciones
- Retry a nivel de workflow

---

## Relacion con datamirai-cloud

Data Mirai Engine es el core open source. El repositorio privado `datamirai-cloud` **envuelve** este engine y agrega las capas necesarias para operar como SaaS multi-tenant:

| Capa Cloud | Funcion |
|---|---|
| **Control Plane** (Next.js + FastAPI) | UI de la plataforma, gestion de usuarios, billing, catalogo de bloques, dashboard |
| **K8s Operator** (Python + Kopf) | Provisioning automatizado de universos y ambientes. Detecta Custom Resources y crea/destruye namespaces con todos sus recursos |
| **Universe Gateway** | Pod FastAPI dentro de cada namespace. Unico punto de entrada del Control Plane a los recursos del universo. El Control Plane nunca accede directo a DB/storage |
| **Ambientes** (Dev/Staging/Prod) | Sub-namespaces con separacion completa de recursos. Promocion de agentes entre ambientes copiando definicion (no datos) |
| **Temporal** | Durabilidad, scheduling avanzado, retry a nivel de workflow, historial de ejecuciones |

El engine no sabe si esta corriendo en Cloud o self-hosted. La diferencia es quien inyecta las credenciales en el ExecutionContext y que infraestructura rodea la ejecucion.

```
datamirai-engine (publico)          datamirai-cloud (privado)
┌────────────────────────┐        ┌──────────────────────────────┐
│  Motor de grafos       │        │  Control Plane (UI + billing)│
│  Sistema de bloques    │◄───────│  K8s Operator (provisioning) │
│  Editor visual         │  usa   │  Universe Gateway            │
│  Runtime de agentes    │        │  Ambientes + promocion       │
│  Triggers              │        │  Temporal (durabilidad)      │
│  Memoria               │        │  Multi-tenancy               │
│  Helm Chart basico     │        │  Integraciones premium       │
└────────────────────────┘        └──────────────────────────────┘
```
