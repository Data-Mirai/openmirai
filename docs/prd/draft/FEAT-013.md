# FEAT-013 — YAML Declarative Agents

**Estado**: Draft
**Fecha**: 2026-05-11
**Epic**: EPIC-031

---

## Problem Statement

**Tipo**: Feature nueva (formato declarativo para agentes)
**Actor**: Usuario local — persona que desarrolla agentes en Data Mirai Engine y quiere versionarlos, compartirlos o migrarlos.

Hoy los agentes viven exclusivamente en la base de datos SQLite. Su definicion (grafo, config, triggers, data_maps) esta serializada como JSON en columnas de las tablas `agents` y `graphs`. Esto tiene tres problemas:

1. **Sin versionado real**: El usuario no puede hacer git diff de un agente. Si modifica un grafo y algo se rompe, no hay forma de volver a la version anterior excepto restaurando un backup de SQLite. Los `agent_snapshots` son internos y no son human-readable.

2. **Sin portabilidad**: No hay forma de exportar un agente de una instalacion de Data Mirai y importarlo en otra. Tampoco de compartir un agente como archivo (ej: en un repo, en un marketplace, via email). El agente esta atrapado en la DB.

3. **Sin infra-as-code**: Los equipos que trabajan con CI/CD no pueden tratar agentes como codigo. No hay forma de validar la definicion de un agente en un PR, ni de desplegar agentes automaticamente desde un repo git. El workflow es 100% click-and-deploy desde la UI.

Necesitamos un formato declarativo (YAML) que sea human-readable, git-friendly, y bidireccional (export ↔ import sin perdida de fidelidad).

---

## Objetivo

Cuando esto este implementado, el usuario puede:
1. Exportar cualquier agente como un archivo YAML legible y completo
2. Importar un archivo YAML para crear o actualizar un agente en cualquier instalacion
3. Hacer round-trip fidelity: exportar → importar produce un agente identico al original
4. Versionar agentes en git con diffs legibles
5. Validar YAML de agentes con errores descriptivos antes de importar
6. Usar CLI para import/export como parte de workflows de CI/CD

---

## Features

### 13.1 — Agent YAML Format

**Problema**: No existe un formato estandarizado para representar un agente de Data Mirai fuera de la base de datos. La definicion interna (JSON en SQLite) mezcla datos operacionales (id, created_at, status) con la definicion semantica (grafo, config, triggers). No es legible ni portable.

**Solucion**: Definir un formato YAML con schema estricto que capture la definicion completa de un agente: metadata, triggers, grafo (nodos + edges + data_maps), configuracion, context compiler y guardrails. El formato separa identidad (name, description) de definicion (graph) de operacion (status, created_at — que NO se incluyen en el YAML).

**Arquitectura**:
- `AgentYAMLSchema` define la estructura:
  ```
  datamirai: "1.0"              # version del formato
  kind: "Agent"               # tipo de recurso
  name: str                   # nombre unico del agente
  description: str            # descripcion
  tags: list[str]             # tags para busqueda/filtrado

  triggers:                   # lista de triggers
    - type: str               # manual, webhook, schedule, event
      config: dict            # config especifica del trigger type

  graph:
    nodes:                    # lista de nodos del grafo
      - id: str               # ID unico del nodo en el grafo
        tool_type: str         # tipo de herramienta (ai/llm_call, data/db_read, etc)
        label: str             # label visible en editor
        config: dict           # configuracion del nodo
        position:              # posicion visual (x, y) para el editor
          x: float
          y: float
        data_map: dict         # mapeo de datos (templates con referencias a otros nodos)
    edges:                    # conexiones entre nodos
      - source: str            # id del nodo origen
        target: str            # id del nodo destino
        condition: str | null  # condicion para edges condicionales
        label: str | null      # label visible en el edge

  config:                     # configuracion global del agente
    max_retries: int
    timeout_seconds: int
    provider_id: str | null   # LLM provider default
    model: str | null         # modelo default

  context_compiler:           # como se compila el contexto para LLM calls
    strategy: str             # full, summary, selective
    max_tokens: int | null

  guardrails:                 # reglas de proteccion
    - type: str               # input_validation, output_validation, cost_limit
      config: dict
  ```
- `AgentYAMLSerializer`:
  - `serialize(agent, graph) -> str` — convierte agent + graph de DB a YAML string
  - `deserialize(yaml_str) -> (AgentSpec, GraphSpec)` — parsea YAML a objetos intermedios
  - `validate(yaml_str) -> list[ValidationError]` — valida sin importar, retorna errores descriptivos
- `ValidationError`: `{ path: str, message: str, severity: str }` (error/warning)
- Validaciones:
  - Schema validation (campos requeridos, tipos correctos)
  - Graph integrity: todos los edges referencian nodos existentes
  - Tool types: todos los tool_type existen en el ToolRegistry
  - Data map references: templates referencian nodos que existen en el grafo
  - Circular dependency detection: no hay ciclos infinitos (excepto loops explícitos)
  - Config completeness: nodos que requieren config la tienen

**Entidades nuevas**:

No se crean tablas nuevas. El YAML es una representacion externa. Se agrega campo a tabla existente:

Columna adicional en `agents`:

| Columna | Tipo | Constraint |
|---|---|---|
| yaml_hash | TEXT | NULL (hash SHA256 del ultimo YAML exportado, para detectar drift) |

**Contratos API**:
- `GET /api/agents/{id}/export?format=yaml` — exportar agente como YAML. Response: `text/yaml` con Content-Disposition attachment. Header `X-YAML-Hash: {sha256}`.
- `POST /api/agents/import` — importar agente desde YAML. Body: `multipart/form-data` con archivo YAML + `environment_id`. Response: `{ agent: Agent, warnings: ValidationError[] }`. Si hay errors (no warnings): `400` con `{ errors: ValidationError[] }`.
- `POST /api/agents/validate-yaml` — validar YAML sin importar. Body: `multipart/form-data` con archivo YAML. Response: `{ valid: bool, errors: ValidationError[], warnings: ValidationError[] }`.
- `POST /api/agents/{id}/sync-yaml` — actualizar agente existente desde YAML (overwrite). Body: `multipart/form-data` con archivo YAML. Response: `{ agent: Agent, changes: string[] }`. Guard: solo si el agente no tiene sessions activas.

**Pantallas**:
- **AgentDetail → boton "Export YAML"** (en header, junto a otros actions):
  - Click descarga archivo `{agent_name}.yaml`
  - Tooltip muestra ultimo hash exportado vs actual (indica si hay cambios desde ultimo export)
- **AgentDetail → boton "Import YAML"** (en header):
  - Abre file picker para seleccionar archivo .yaml
  - Preview de cambios (diff visual) antes de confirmar
  - Muestra warnings/errors de validacion
  - Boton "Apply" para confirmar importacion
- **Environment page → boton "Import Agent from YAML"**:
  - Abre file picker
  - Preview del agente que se va a crear
  - Validacion inline
  - Boton "Import" que crea el agente en el environment

**Reglas**:

| ID | Invariante | Enforcement |
|---|---|---|
| REGLA-220 | El YAML NUNCA incluye datos operacionales: id, status, created_at, updated_at, environment_id, graph_id. Estos se generan al importar | AgentYAMLSerializer.serialize excluye estos campos |
| REGLA-221 | Round-trip fidelity: export(import(yaml)) == yaml (salvo whitespace y orden de keys). Si no se cumple, es un bug | Test de round-trip en CI |
| REGLA-222 | Validacion es obligatoria antes de import. Si hay errores (severity=error), import se rechaza. Warnings se muestran pero no bloquean | Endpoint validate-yaml llamado internamente por import |
| REGLA-223 | El campo `datamirai: "1.0"` es obligatorio y define la version del formato. Futuras versiones del formato seran backward compatible. Si la version es mayor a la soportada, error descriptivo | Version check en deserialize |
| REGLA-224 | API keys y credenciales NUNCA aparecen en el YAML. Los nodos que referencian providers lo hacen por nombre (provider_id), no por key | Serializer strip de api_key fields |

**Archivos a crear/modificar**:
- CREAR `framework/src/datamirai_engine/yaml/__init__.py` — exports del modulo
- CREAR `framework/src/datamirai_engine/yaml/schema.py` — AgentYAMLSchema definition + version constant
- CREAR `framework/src/datamirai_engine/yaml/serializer.py` — AgentYAMLSerializer (serialize, deserialize, validate)
- CREAR `framework/src/datamirai_engine/yaml/validators.py` — validaciones de integridad (graph, tools, data_maps, cycles)
- CREAR `app/server/datamirai_app/routes/yaml_agents.py` — endpoints export, import, validate, sync
- MODIFICAR `app/server/datamirai_app/database.py` — columna yaml_hash en agents
- MODIFICAR `app/server/datamirai_app/app.py` — registrar blueprint de yaml_agents
- MODIFICAR `app/web/src/lib/api.ts` — funciones client para export, import, validate
- MODIFICAR `app/web/src/app/agents/[id]/page.tsx` — botones Export/Import YAML
- MODIFICAR `app/web/src/app/universes/[id]/environments/[envId]/page.tsx` — boton Import Agent from YAML

---

### 13.2 — CLI Import/Export

**Problema**: Los endpoints REST permiten import/export programatico, pero no hay CLI. Para workflows de CI/CD, scripts de migracion, o uso desde terminal, el usuario necesita comandos de linea.

**Solucion**: Comandos CLI integrados en el CLI existente de Data Mirai (`datamirai` command) para exportar e importar agentes. Round-trip completo desde terminal.

**Arquitectura**:
- Extiende `framework/src/datamirai_engine/cli.py` (CLI existente)
- Comandos nuevos:
  - `datamirai agent export <agent_id> [--format yaml] [--output <path>]`
    - Si `--output` no se especifica, imprime a stdout
    - Default format: yaml (unico soportado inicialmente)
    - Resuelve agent_id contra la DB local o contra un server remoto si `--server` se pasa
  - `datamirai agent import <path> [--environment <env_id>]`
    - Lee archivo YAML del path
    - Valida antes de importar (muestra errores/warnings)
    - Si `--environment` no se especifica, pide seleccionar interactivamente o usa default
    - Retorna ID del agente creado
  - `datamirai agent validate <path>`
    - Valida archivo YAML sin importar
    - Retorna exit code 0 si valido, 1 si hay errores
    - Muestra errores/warnings en stderr
  - `datamirai agent list [--environment <env_id>] [--format json|table]`
    - Lista agentes disponibles
    - Util para obtener agent_id para export
- Integracion con server:
  - Si el CLI se ejecuta en el mismo directorio que la app, usa la DB SQLite directa
  - Si `--server <url>` se pasa, usa los endpoints REST

**Entidades**: No se crean tablas nuevas.

**Contratos API**: Usa los endpoints definidos en 13.1.

**Pantallas**: No tiene (es CLI).

**Reglas**:

| ID | Invariante | Enforcement |
|---|---|---|
| REGLA-225 | CLI export produce EXACTAMENTE el mismo YAML que el endpoint REST. La logica de serializacion es la misma (usa AgentYAMLSerializer) | Shared code, no duplicacion |
| REGLA-226 | CLI validate retorna exit code 0 si valid, 1 si errors. Compatible con CI/CD pipelines (ej: Github Actions `if: steps.validate.outcome == 'success'`) | Exit codes en CLI handler |
| REGLA-227 | CLI nunca escribe a stdout excepto el output solicitado. Logs y mensajes informativos van a stderr | Click/typer output conventions |

**Archivos a crear/modificar**:
- MODIFICAR `framework/src/datamirai_engine/cli.py` — agregar grupo de comandos `agent` con subcomandos export, import, validate, list
- CREAR `framework/src/datamirai_engine/cli/agent_commands.py` — implementacion de los comandos agent (si el CLI usa modulos separados)

---

### 13.3 — Version Control Integration

**Problema**: Aunque el YAML permite exportar agentes individualmente, no hay workflow para gestionar multiples agentes como un directorio versionable. El usuario tiene que exportar uno por uno, nombrar archivos manualmente, y recordar cual agente corresponde a cual archivo.

**Solucion**: Comando batch que exporta todos los agentes de un environment a un directorio (1 archivo por agente), y comando para importar un directorio completo. Los archivos usan naming convention git-friendly: `{agent_name_slug}.agent.yaml`.

**Arquitectura**:
- Comandos CLI adicionales:
  - `datamirai agent export-all --environment <env_id> --dir <path>`
    - Crea directorio si no existe
    - Exporta cada agente como `{slug}.agent.yaml`
    - Genera `_manifest.yaml` con lista de agentes exportados + hashes
  - `datamirai agent import-all --dir <path> --environment <env_id>`
    - Lee todos los `*.agent.yaml` del directorio
    - Importa/actualiza cada uno en el environment
    - Respeta `_manifest.yaml` para detectar agentes eliminados (marca como deleted, no borra)
  - `datamirai agent diff --dir <path> --environment <env_id>`
    - Compara estado del directorio con la DB
    - Muestra: nuevos (en dir pero no en DB), modificados (hash diferente), eliminados (en DB pero no en dir)
- `_manifest.yaml` format:
  ```
  datamirai: "1.0"
  kind: "AgentManifest"
  environment: str
  exported_at: str
  agents:
    - file: str         # nombre del archivo
      name: str         # nombre del agente
      hash: str         # SHA256 del contenido
  ```
- Naming convention: nombre del agente → slug (lowercase, spaces → hyphens, remove special chars) + `.agent.yaml`

**Entidades**: No se crean tablas nuevas.

**Contratos API**:
- `GET /api/environments/{envId}/agents/export-all` — exportar todos los agentes del environment como ZIP. Response: `application/zip` con todos los `*.agent.yaml` + `_manifest.yaml`
- `POST /api/environments/{envId}/agents/import-all` — importar ZIP con agentes. Body: `multipart/form-data` con archivo ZIP. Response: `{ imported: int, updated: int, errors: ValidationError[] }`

**Pantallas**:
- **Environment page → menu de acciones**:
  - "Export All Agents" → descarga ZIP
  - "Import Agents from Directory" → upload ZIP

**Reglas**:

| ID | Invariante | Enforcement |
|---|---|---|
| REGLA-228 | Un directorio de agentes tiene exactamente 1 archivo por agente. El nombre del archivo (slug) se deriva del nombre del agente. No se permite renombrar el archivo sin cambiar el nombre del agente | Validation en import-all |
| REGLA-229 | `_manifest.yaml` es informativo, no autoritativo. Si un archivo .agent.yaml existe en el directorio pero no en el manifest, se importa igual. El manifest solo sirve para diff y detectar eliminaciones | Import-all lee directorio, no manifest |
| REGLA-230 | import-all NUNCA elimina agentes de la DB. Si un agente existe en DB pero no en el directorio, se reporta como "not in directory" pero NO se borra | Safety: no data loss |
| REGLA-231 | Los diffs de YAML deben ser legibles en git. Esto significa: keys ordenados alfabeticamente, strings multiline con `|` (block scalar), listas con `- ` indent consistente | YAML dump config en serializer |

**Archivos a crear/modificar**:
- MODIFICAR `framework/src/datamirai_engine/cli.py` — agregar comandos export-all, import-all, diff
- CREAR `framework/src/datamirai_engine/yaml/manifest.py` — AgentManifest dataclass + serializer
- CREAR `framework/src/datamirai_engine/yaml/batch.py` — logica de export-all, import-all, diff
- CREAR `app/server/datamirai_app/routes/yaml_agents.py` — agregar endpoints export-all, import-all (si no existe ya de 13.1, fusionar)
- MODIFICAR `app/web/src/app/universes/[id]/environments/[envId]/page.tsx` — botones Export All / Import All

---

## Dependencias entre features

```
13.1 (YAML Format) ← independiente, se implementa primero (base para todo)
13.2 (CLI) ← depende de 13.1 para serializer/deserializer
13.3 (Version Control) ← depende de 13.1 + 13.2 para formato y CLI
```

Orden de implementacion: 13.1 → 13.2 → 13.3

---

## Entidades nuevas (resumen consolidado)

### Columna en agents (existente)
| Columna | Tipo | Constraint |
|---|---|---|
| yaml_hash | TEXT | NULL |

No se crean tablas nuevas. El YAML es formato de intercambio, no de persistencia.

---

## Reglas de negocio nuevas (consolidado)

| ID | Invariante | Enforcement |
|---|---|---|
| REGLA-220 | YAML nunca incluye datos operacionales (id, status, timestamps) | Serializer exclude list |
| REGLA-221 | Round-trip fidelity: export(import(yaml)) == yaml | Test de CI |
| REGLA-222 | Validacion obligatoria antes de import. Errors bloquean, warnings no | Validate interno |
| REGLA-223 | `datamirai: "1.0"` obligatorio. Version check en deserialize | Version parser |
| REGLA-224 | API keys NUNCA en YAML. Providers por nombre, no por key | Serializer strip |
| REGLA-225 | CLI export == REST export (misma logica) | Shared AgentYAMLSerializer |
| REGLA-226 | CLI exit codes: 0=valid, 1=error. CI-friendly | Exit code handler |
| REGLA-227 | CLI output solo a stdout. Info a stderr | Output conventions |
| REGLA-228 | 1 archivo por agente. Nombre derivado del agent name | Slug validation |
| REGLA-229 | Manifest informativo, no autoritativo. Import lee directorio | Import-all logic |
| REGLA-230 | import-all NUNCA elimina agentes de DB | Safety guard |
| REGLA-231 | YAML git-friendly: keys sorted, block scalars, consistent indent | YAML dump config |

---

## Notas de implementacion

- **PyYAML como dependencia**. Se usa `pyyaml` (ya comun en el ecosistema Python) con `yaml.dump(default_flow_style=False, sort_keys=True)` para output legible y diff-friendly.
- **El campo `spec_yaml` ya existe en la tabla agents** pero se usa para el YAML generado por el diseñador conversacional. Este PRD formaliza el formato y agrega export/import bidireccional. El serializer debe producir YAML compatible con spec_yaml.
- **Validacion usa el ToolRegistry del framework**. Al validar tool_types, se instancia el registry para verificar que cada tipo de herramienta existe. Esto significa que la validacion requiere el framework instalado (no es solo parsing YAML).
- **Posiciones de nodos se incluyen en el YAML**. Son necesarias para reconstruir el grafo visual en el editor. Sin posiciones, el usuario tendria que re-layoutear todo manualmente al importar.
- **ZIP para batch export/import via API**. El CLI trabaja con directorios nativos del filesystem. La API usa ZIP porque HTTP no soporta directorios. El servidor descomprime en memoria, no en disco.
- **Naming convention para slugs**: `re.sub(r'[^a-z0-9-]', '', name.lower().replace(' ', '-'))`. Collision: si dos agentes producen el mismo slug, se agrega suffix numerico (`-1`, `-2`).

---

## doc_refs
- `docs/prd/draft/DATAMIRAI-ENGINE-PRD.md` — PRD base del engine
- `docs/ARCHITECTURE.md` — stack, convenciones
- `docs/prd/draft/FEAT-001.md` — MVP features (context: agentes como YAML)
