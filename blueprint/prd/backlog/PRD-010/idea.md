# PRD-010 — File-First Data Layer: Archivos como Ciudadanos de Primera Clase

| Campo | Valor |
|-------|-------|
| **ID** | PRD-010 |
| **Fecha** | 2026-05-28 |
| **Estado** | designed |
| **Branch** | prd/PRD-010 |
| **Target** | v0.5.0 |
| **Depende de** | PRD-009 (multimodal file input) |

---

## Diagrama General

```
┌─────────────────────────────────────────────────────────────────────────────┐
│                PRD-010 — FILE-FIRST DATA LAYER                              │
│                                                                             │
│  HOY (PRD-009) — archivos como caso especial                                │
│  ───────────────────────────────────────────                                │
│                                                                             │
│  trigger ──▶ ai/transcribe ──▶ ai/llm_call ──▶ output                      │
│    "path"      file_path          prompt          text                      │
│   (string)     (string)          (string)       (string)                    │
│                                                                             │
│  Los archivos son strings con paths. Cada tool tiene logica especial.       │
│  system/bash puede crear archivos pero no puede pasarlos al grafo.          │
│                                                                             │
│  DESPUES (PRD-010) — archivos como datos nativos del grafo                  │
│  ────────────────────────────────────────────────────                        │
│                                                                             │
│  trigger ──▶ system/bash ──▶ ai/llm_call ──▶ output                        │
│  {file_ref}   {file_ref}    {file_ref+text}   {file_ref+text}              │
│   (FileRef)    (FileRef)      (FileRef)         (FileRef)                   │
│                                                                             │
│  FileRef es un JSON object con forma estandar:                              │
│  ┌──────────────────────────────────────────────┐                           │
│  │  {                                            │                           │
│  │    "_type": "file_ref",                       │                           │
│  │    "path": "/abs/path/to/file.png",           │                           │
│  │    "mime_type": "image/png",                  │                           │
│  │    "size_bytes": 245760                       │                           │
│  │  }                                            │                           │
│  └──────────────────────────────────────────────┘                           │
│                                                                             │
│  FLUJO COMPLETO:                                                            │
│                                                                             │
│  1. Trigger recibe path → engine construye FileRef                          │
│     ┌────────┐     file_ref                                                 │
│     │trigger │──────────────▶ FileRef { path, mime, size }                  │
│     └────────┘                                                              │
│                                                                             │
│  2. system/bash genera archivos → engine construye FileRef por output       │
│     ┌────────┐     output_files: ["result.png"]                             │
│     │  bash  │──────────────▶ FileRef { path: scratch/result.png }          │
│     └────────┘                                                              │
│                                                                             │
│  3. data_map pasa FileRef → tool auto-resuelve                              │
│     data_map:                                                               │
│       media_path: "bash.file_result_png"                                    │
│     ┌────────────┐                                                          │
│     │ai/llm_call │ ← recibe FileRef, extrae .path, lee archivo             │
│     └────────────┘                                                          │
│                                                                             │
│  4. output/response incluye FileRef en respuesta                            │
│     ┌────────┐     { text: "desc...", file: FileRef }                       │
│     │ output │──────────────▶ response con archivos                         │
│     └────────┘                                                              │
│                                                                             │
│  SCRATCH DIR (por ejecucion):                                               │
│  ┌─────────────────────────────────────────────┐                            │
│  │  /tmp/mirai-exec-{uuid}/                    │                            │
│  │    result.png  ← bash lo creo               │                            │
│  │    audio.wav   ← otro nodo lo creo          │                            │
│  │  (limpiado al terminar la ejecucion)        │                            │
│  └─────────────────────────────────────────────┘                            │
└─────────────────────────────────────────────────────────────────────────────┘
```

---

## Problema

- **Tipo**: feature
- **Resumen**: Los archivos no son ciudadanos de primera clase en el grafo. Fluyen como strings (paths), cada tool tiene logica especial para manejarlos, y no hay forma estandar de que un nodo produzca archivos que otro consuma.
- **Actores**: Developer, Engine
- **Flujos tocados**: data_map resolution, tool input validation, system/bash execution, output/response, trigger output
- **Que cambia**:
  - HOY: Archivos son strings con paths. ai/transcribe y ai/llm_call (PRD-009) los leen internamente. system/bash puede crear archivos pero no hay forma de pasarlos al siguiente nodo. No hay scratch dir.
  - DESPUES: FileRef es un JSON object estandar que fluye por data_map como cualquier otro valor. FieldType::File valida FileRefs. Tools auto-resuelven FileRef.path cuando reciben uno. system/bash puede declarar output_files. Cada ejecucion tiene scratch dir.

---

## Actores y Permisos

| Actor | Capacidad | Accion | Visibilidad |
|-------|-----------|--------|-------------|
| Developer | disenar_agente | Usa FileRef en data_map, declara output_files en bash | Spec completo |
| Engine | gestionar_archivos | Crea scratch dir, construye FileRefs, auto-resuelve paths, limpia al terminar | Interno |

---

## Entidades

### FileRef (nueva — convencion de datos, no persistida)

| Campo | Tipo logico | Requerido | Descripcion |
|-------|-------------|-----------|-------------|
| _type | texto (literal "file_ref") | si | Discriminador para identificar FileRefs en el grafo |
| path | texto | si | Ruta absoluta al archivo |
| mime_type | texto | si | Tipo MIME detectado |
| size_bytes | numero | si | Tamano del archivo en bytes |

**Restricciones**: FileRef es un JSON object con forma fija. Existe en SharedState como `serde_json::Value`. No es un tipo nuevo del runtime — es una convencion sobre Value::Object.

```
FileRef como JSON:
{
  "_type": "file_ref",
  "path": "/tmp/mirai-exec-abc123/result.png",
  "mime_type": "image/png",
  "size_bytes": 245760
}
```

### FieldType (modificada — enum)

Nuevo variante:

| Variante | matches() | Descripcion |
|----------|-----------|-------------|
| File | Object con `_type == "file_ref"` y `path` presente | Referencia a archivo |

**Backward compat**: FieldType::String sigue aceptando strings. FieldType::File es un tipo nuevo que solo matchea FileRef objects. Los tools existentes que aceptan paths como String siguen funcionando.

### ScratchDir (nueva — runtime, por ejecucion)

| Campo | Tipo logico | Requerido | Descripcion |
|-------|-------------|-----------|-------------|
| path | texto | si | Ruta absoluta al directorio temporal |
| execution_id | texto | si | ID de la ejecucion que lo creo |

**Lifecycle**: creado al inicio de la ejecucion, eliminado al finalizar (exito o error).

### BashTool config (modificada)

Campo nuevo:

| Campo | Tipo logico | Requerido | Descripcion |
|-------|-------------|-----------|-------------|
| output_files | lista de [texto] | no | Nombres de archivos que el comando va a generar. Despues de ejecutar, el engine busca estos archivos y produce FileRef outputs |

### LlmCallTool / TranscribeTool (modificada — auto-resolucion)

Sin cambio de schema. Cambio de comportamiento: cuando `media_path` o `file_path` reciben un Value::Object con `_type: "file_ref"`, extraen `path` automaticamente en vez de tratar el objeto como string.

---

## Ciclos de Vida

### ScratchDir

Estados: CREATED | ACTIVE | CLEANED

| Desde | Hacia | Actor | Guard | Efecto |
|-------|-------|-------|-------|--------|
| — | CREATED | Engine | Inicio de ejecucion | Crea directorio /tmp/mirai-exec-{uuid}/ |
| CREATED | ACTIVE | Engine | Primer nodo ejecuta | Disponible para escritura |
| ACTIVE | CLEANED | Engine | Ejecucion termina (ok o error) | rm -rf del directorio |

```
  ── inicio ──▶ CREATED ──▶ ACTIVE ──▶ CLEANED
                                         │
                                    rm -rf scratch/
```

---

## Reglas de Negocio

### file-ref-shape

- **Invariante**: Un FileRef valido DEBE tener: `_type == "file_ref"`, `path` (texto no vacio), `mime_type` (texto no vacio), `size_bytes` (numero >= 0).
- **Cuando se verifica**: al construir un FileRef (funcion constructora), no al fluir por data_map
- **Si se viola**: N/A — por construccion (la funcion constructora valida)

### file-ref-path-absoluto

- **Invariante**: FileRef.path DEBE ser absoluto. Paths relativos se resuelven contra scratch_dir.
- **Cuando se verifica**: al construir el FileRef
- **Si se viola**: N/A — la funcion constructora convierte a absoluto

### auto-resolucion-file-ref

- **Invariante**: Cuando un tool declara un input como `file_path` o `media_path` (FieldType::String), y recibe un Value::Object que es un FileRef, el engine extrae `path` automaticamente. El tool recibe el path string como si el developer lo hubiera pasado directamente.
- **Cuando se verifica**: en tool.execute(), al resolver inputs
- **Si se viola**: N/A — por construccion

### auto-resolucion-backward-compat

- **Invariante**: Si un input `file_path`/`media_path` recibe un Value::String (path directo), funciona identico a hoy. La auto-resolucion solo aplica a FileRef objects.
- **Cuando se verifica**: siempre
- **Si se viola**: N/A — por construccion

### scratch-dir-cleanup

- **Invariante**: El scratch dir se elimina al finalizar la ejecucion, sin importar si fue exitosa o fallida. Archivos temporales no sobreviven entre ejecuciones.
- **Cuando se verifica**: en el runner, post-execution
- **Si se viola**: N/A — por construccion. Si cleanup falla, log warning (no error).

### scratch-dir-isolation

- **Invariante**: Cada ejecucion tiene su propio scratch dir. Ejecuciones concurrentes no comparten directorio.
- **Cuando se verifica**: al crear el scratch dir (usa UUID)
- **Si se viola**: N/A — por construccion (UUID unico)

### bash-output-files-validation

- **Invariante**: Despues de ejecutar bash, para cada nombre en `output_files`, el engine busca el archivo en: (1) cwd del bash, (2) scratch dir. Si no existe, el output correspondiente es null (no error — el comando pudo no haberlo generado).
- **Cuando se verifica**: post-ejecucion del bash
- **Si se viola**: output field es null + warning log

### field-type-file-validation

- **Invariante**: FieldType::File matchea solo Value::Object con `_type == "file_ref"` y `path` presente. No matchea strings, numeros, ni objects sin `_type`.
- **Cuando se verifica**: en validate_node_inputs (capa 2 de PRD-004)
- **Si se viola**: validation error como cualquier otro tipo mismatch

---

## Patrones de Diseno

### Strategy → FileRef auto-resolution

- **Aplica a**: Resolucion de inputs que pueden ser string (path directo) o FileRef (objeto)
- **Por que**: El mismo input acepta dos formas. La estrategia detecta cual es y extrae el valor correcto. Permite backward compat total.
- **Participantes**: resolve_file_input (estrategia), ai/transcribe, ai/llm_call (consumidores)

### Factory → FileRef construction

- **Aplica a**: Creacion de FileRef objects desde diferentes fuentes (paths, bash outputs)
- **Por que**: Un solo punto de creacion garantiza la forma correcta del JSON.
- **Participantes**: FileRef::from_path (constructor), system/bash, trigger (productores)

---

## Operaciones

### create_file_ref (interna — utilidad)

- **Actor**: Engine
- **Input**:
  - path (texto, requerido) — ruta al archivo (absoluta o relativa)
  - base_dir (texto, opcional) — directorio base para resolver paths relativos
- **Logica**:
  1. Resolver path: si relativo, prepend base_dir o scratch_dir
  2. Verificar que archivo existe
  3. Obtener metadata (size)
  4. Detectar MIME por extension (reutiliza mapa de PRD-009)
  5. Construir JSON object con forma FileRef
- **Output exitoso**: Value::Object con `_type`, `path`, `mime_type`, `size_bytes`
- **Errores posibles**:
  - Archivo no existe → retorna None (no error — el caller decide)

### resolve_file_input (interna — utilidad)

- **Actor**: Engine
- **Input**:
  - value (Value) — el valor recibido por el tool (puede ser string o FileRef object)
- **Logica**:
  1. Si Value::String → retornar string as-is (backward compat, path directo)
  2. Si Value::Object con `_type == "file_ref"` → extraer y retornar `path`
  3. Si Value::Object sin `_type` → stringify y retornar (edge case, improbable)
  4. Si otro tipo → stringify y retornar
- **Output**: String con el path del archivo

### create_scratch_dir (interna)

- **Actor**: Engine (GraphRunner)
- **Input**: execution_id (texto)
- **Logica**:
  1. Crear directorio: `/tmp/mirai-exec-{execution_id}/`
  2. Retornar path absoluto
- **Output**: String con path al scratch dir
- **Errores**: "failed to create scratch dir: {error}"

### cleanup_scratch_dir (interna)

- **Actor**: Engine (GraphRunner)
- **Input**: scratch_dir_path (texto)
- **Logica**:
  1. rm -rf del directorio
  2. Si falla: log warning (no error — best effort cleanup)
- **Output**: ninguno

### bash_con_output_files (modificada — system/bash.execute)

- **Actor**: Engine
- **Input**: command (texto), output_files (lista, opcional, desde config)
- **Config**: timeout, cwd, output_files (nuevo)
- **Logica nueva** (post-ejecucion):
  1. Para cada nombre en output_files:
     a. Buscar archivo en cwd del bash (si definido) o scratch_dir
     b. Si encontrado → create_file_ref(path) → agregar al output como `file_{nombre_sanitizado}`
     c. Si no encontrado → output field es null + warning log
  2. Outputs existentes (stdout, stderr, exit_code, timed_out) sin cambio
- **Output nuevo**: `file_{name}` (FileRef o null) por cada entrada en output_files

### transcribe_con_file_ref (modificada — ai/transcribe.execute)

- **Actor**: Engine
- **Logica nueva**: Antes de leer el archivo, resolver input con resolve_file_input(). Si recibe FileRef, extrae path. Si recibe string, usa directo (backward compat).

### llm_call_con_file_ref (modificada — ai/llm_call.execute)

- **Actor**: Engine
- **Logica nueva**: Antes de leer media, resolver media_path con resolve_file_input(). Si recibe FileRef, extrae path. Si recibe string, usa directo (backward compat).

---

## Interfaces

> Engine no tiene frontend. Las interfaces son tools YAML, CLI, y HTTP API.

### FileRef en data_map — YAML

**Archivo de trigger fluye a transcribe**:
```yaml
name: stt-pipeline
version: v1

graph:
  nodes:
    - id: start
      tool_type: trigger/manual
    - id: transcribe
      tool_type: ai/transcribe
      config:
        model: "gemini-2.5-flash"
    - id: respond
      tool_type: output/response

  edges:
    - source: start
      target: transcribe
      data_map:
        file_path: "start.payload.audio_path"
        # Si audio_path es un string → funciona como hoy
        # Si audio_path es un FileRef → auto-resuelve path
    - source: transcribe
      target: respond
```

### system/bash con output_files — YAML

**Bash genera imagen, LLM la analiza**:
```yaml
name: pdf-to-text
version: v1

graph:
  nodes:
    - id: start
      tool_type: trigger/manual
    - id: convert
      tool_type: system/bash
      config:
        command: "convert ${MIRAI_pdf_path}[0] /tmp/mirai-scratch/page.png"
        output_files: ["page.png"]
    - id: analyze
      tool_type: ai/llm_call
      config:
        prompt: "Extract all text from this image."
        model: "gemini-2.5-flash"
    - id: respond
      tool_type: output/response

  edges:
    - source: start
      target: convert
      data_map:
        pdf_path: "start.payload.pdf_path"
    - source: convert
      target: analyze
      data_map:
        media_path: "convert.file_page_png"
        # file_page_png es un FileRef — ai/llm_call auto-resuelve .path
    - source: analyze
      target: respond
```

### Bash multi-file output — YAML

```yaml
- id: split_video
  tool_type: system/bash
  config:
    command: "ffmpeg -i $MIRAI_video -ss 0 -t 10 frame1.jpg -ss 30 -t 10 frame2.jpg"
    output_files: ["frame1.jpg", "frame2.jpg"]
# Outputs:
#   split_video.file_frame1_jpg → FileRef { path: ".../frame1.jpg", mime: "image/jpeg", ... }
#   split_video.file_frame2_jpg → FileRef { path: ".../frame2.jpg", mime: "image/jpeg", ... }
#   split_video.stdout → "..."
#   split_video.exit_code → 0
```

### Scratch dir — transparente al developer

```yaml
# El developer NO necesita saber sobre scratch dir.
# El engine lo crea automaticamente y lo pasa a los tools que lo necesiten.
# system/bash puede escribir archivos ahi via $MIRAI_SCRATCH_DIR env var.

- id: process
  tool_type: system/bash
  config:
    command: "cp $MIRAI_input_file $MIRAI_SCRATCH_DIR/processed.wav && echo done"
    output_files: ["processed.wav"]
# $MIRAI_SCRATCH_DIR apunta al scratch dir de esta ejecucion
```

### Error messages

| Condicion | Mensaje |
|-----------|---------|
| output_file no encontrado | warning log: `output file 'result.png' not found in scratch dir or cwd` |
| scratch dir creation fails | error: `failed to create scratch dir: {io_error}` |
| FileRef con path invalido | tool error: `media file not found: {path}` (mismo error que PRD-009) |

---

## Matriz de Permutaciones

### FileRef auto-resolution

| Flujo | Permutacion | Actor | Resultado |
|---|---|---|---|
| resolve_file_input | Value::String (path directo) | Engine | Retorna string as-is (backward compat) |
| resolve_file_input | Value::Object FileRef valido | Engine | Extrae y retorna FileRef.path |
| resolve_file_input | Value::Object sin _type | Engine | Stringify del objeto (edge case) |
| resolve_file_input | Value::Number | Engine | Stringify "42" (edge case) |
| resolve_file_input | Value::Null | Engine | Retorna "" (empty string) |

### FieldType::File

| Flujo | Permutacion | Actor | Resultado |
|---|---|---|---|
| field_type_file | FileRef valido | Engine | matches() → true |
| field_type_file | String con path | Engine | matches() → false (es String, no File) |
| field_type_file | Object sin _type | Engine | matches() → false |
| field_type_file | Object con _type distinto | Engine | matches() → false |
| field_type_file | null | Engine | matches() → false |

### create_file_ref

| Flujo | Permutacion | Actor | Resultado |
|---|---|---|---|
| create_file_ref | Archivo existe, extension conocida | Engine | FileRef con mime y size |
| create_file_ref | Archivo no existe | Engine | None (no error) |
| create_file_ref | Path relativo + base_dir | Engine | Resuelve a absoluto, retorna FileRef |
| create_file_ref | Sin extension | Engine | FileRef con mime_type "application/octet-stream" |

### system/bash output_files

| Flujo | Permutacion | Actor | Resultado |
|---|---|---|---|
| bash_output_files | Happy: archivo generado en scratch dir | Engine | FileRef en output `file_{name}` |
| bash_output_files | Archivo generado en cwd | Engine | FileRef con path absoluto al cwd |
| bash_output_files | Archivo no generado | Engine | Output field es null + warning |
| bash_output_files | output_files no definido (como hoy) | Engine | Sin FileRef outputs, backward compat |
| bash_output_files | Multiples output_files | Engine | Un FileRef por archivo encontrado |
| bash_output_files | Comando falla (exit != 0) | Engine | Aun asi busca output_files (best effort) |

### Scratch dir lifecycle

| Flujo | Permutacion | Actor | Resultado |
|---|---|---|---|
| scratch_dir | Ejecucion exitosa | Engine | Scratch dir creado al inicio, eliminado al final |
| scratch_dir | Ejecucion falla | Engine | Scratch dir eliminado (cleanup en finally) |
| scratch_dir | Ejecuciones concurrentes | Engine | Cada una tiene UUID unico, sin colision |
| scratch_dir | Cleanup falla (permisos) | Engine | Warning log, no error. Best effort. |

### Backward compatibility

| Flujo | Permutacion | Actor | Resultado |
|---|---|---|---|
| backward_compat | YAML v0.4.x sin output_files | Developer | system/bash funciona identico |
| backward_compat | file_path recibe string | Engine | Funciona identico a PRD-009 |
| backward_compat | media_path recibe string | Engine | Funciona identico a PRD-009 |
| backward_compat | Grafos sin archivos | Engine | Sin scratch dir overhead (lazy creation) |

---

## Escenarios GWT

### Journey: Engine — FileRef Auto-Resolution

TEST-115: ai/transcribe recibe FileRef — auto-resuelve path
  Given: Agent con ai/transcribe, data_map pasa un FileRef object (no string)
  When: Engine ejecuta ai/transcribe
  Then: Tool recibe el path extraido del FileRef. Funciona como si fuera string.

TEST-116: ai/transcribe recibe string — backward compat
  Given: Agent con ai/transcribe, data_map pasa un string con path
  When: Engine ejecuta ai/transcribe
  Then: Funciona identico a PRD-009. Sin cambio.

TEST-117: ai/llm_call recibe FileRef en media_path
  Given: Agent con ai/llm_call, data_map pasa FileRef a media_path
  When: Engine ejecuta ai/llm_call
  Then: Tool extrae path del FileRef, lee archivo, envia como multimodal.

### Journey: Engine — FieldType::File

TEST-118: FieldType::File matchea FileRef valido
  Given: FieldType::File y Value::Object con _type "file_ref" y path presente
  When: field_type.matches(&value)
  Then: Retorna true.

TEST-119: FieldType::File no matchea string
  Given: FieldType::File y Value::String("/path/to/file.png")
  When: field_type.matches(&value)
  Then: Retorna false.

TEST-120: FieldType::File no matchea object sin _type
  Given: FieldType::File y Value::Object({"path": "/file.png"}) sin _type
  When: field_type.matches(&value)
  Then: Retorna false.

### Journey: Engine — system/bash output_files

TEST-121: Bash genera archivo declarado en output_files — happy path
  Given: system/bash con command que crea "result.png", output_files: ["result.png"]
  When: Engine ejecuta bash, comando exitoso
  Then: Output incluye file_result_png como FileRef con path, mime, size.

TEST-122: Bash output_file no generado — null output
  Given: system/bash con output_files: ["missing.png"], comando no crea el archivo
  When: Engine ejecuta bash
  Then: Output file_missing_png es null. Warning en log.

TEST-123: Bash sin output_files — backward compat
  Given: system/bash sin config output_files
  When: Engine ejecuta bash
  Then: Outputs identicos a hoy (stdout, stderr, exit_code, timed_out). Sin FileRef.

TEST-124: Bash multiples output_files — parcialmente generados
  Given: system/bash con output_files: ["a.png", "b.jpg"], solo "a.png" existe
  When: Engine ejecuta bash
  Then: file_a_png es FileRef, file_b_jpg es null.

### Journey: Engine — Scratch Dir

TEST-125: Scratch dir se crea al inicio de ejecucion
  Given: Ejecucion de un agente que usa system/bash con output_files
  When: Engine inicia la ejecucion
  Then: $MIRAI_SCRATCH_DIR apunta a directorio existente en /tmp/

TEST-126: Scratch dir se limpia al terminar
  Given: Ejecucion completa (exitosa)
  When: Engine finaliza
  Then: El directorio scratch ya no existe.

TEST-127: Scratch dir se limpia al fallar
  Given: Ejecucion falla a mitad del grafo
  When: Engine finaliza con error
  Then: El directorio scratch ya no existe.

TEST-128: Scratch dir es unico por ejecucion
  Given: Dos ejecuciones concurrentes del mismo agente
  When: Ambas crean scratch dir
  Then: Paths distintos (UUIDs diferentes).

### Journey: Engine — create_file_ref

TEST-129: FileRef desde archivo existente
  Given: Archivo /tmp/test.png de 1000 bytes
  When: create_file_ref("/tmp/test.png", None)
  Then: FileRef con _type "file_ref", path absoluto, mime "image/png", size 1000.

TEST-130: FileRef desde path relativo + base_dir
  Given: Archivo "result.png" en "/tmp/scratch/", base_dir = "/tmp/scratch"
  When: create_file_ref("result.png", Some("/tmp/scratch"))
  Then: FileRef con path "/tmp/scratch/result.png".

TEST-131: FileRef desde archivo sin extension
  Given: Archivo /tmp/data sin extension
  When: create_file_ref("/tmp/data", None)
  Then: FileRef con mime_type "application/octet-stream".

TEST-132: FileRef desde archivo inexistente
  Given: Path /tmp/nonexistent.png
  When: create_file_ref("/tmp/nonexistent.png", None)
  Then: Retorna None.

---

## Fuera de Alcance

- **Nodos generadores de archivos** (ai/image_gen, ai/tts): v1 solo infraestructura de datos. Los nodos nuevos son PRD futuro.
- **File upload via HTTP**: Los archivos deben existir en el filesystem local. Upload remoto es futuro.
- **File streaming**: v1 lee archivos completos en memoria. Streaming para archivos grandes es futuro.
- **Persistencia de scratch dir**: v1 limpia al terminar. Opcion de conservar outputs es futuro.
- **File deduplication**: Si dos nodos producen el mismo archivo, se duplica. Dedup es futuro.
- **Remote file references (URLs)**: FileRef solo apunta a filesystem local. URLs remotas son futuro.
- **File type conversion**: Si un nodo necesita PNG pero recibe JPG, no hay conversion automatica. Es futuro.

---

## Dependencias

- **Depende de PRD-009**: Reutiliza read_media_file, MIME detection map, MediaContent.
- **Reutiliza SharedState**: FileRef fluye como Value::Object, sin cambios al state.
- **Reutiliza resolve_expression**: data_map resuelve FileRef objects como cualquier Value.
- **Reutiliza validate_node_inputs**: Se extiende FieldType con File variant.

### Orden de implementacion recomendado

```
Fase 1 (foundation):
  1. FileRef constructor (en llm/media.rs o nuevo file_ref.rs)
  2. resolve_file_input utility
  3. FieldType::File variant + matches()
  4. Tests unitarios

Fase 2 (scratch dir):
  5. create_scratch_dir / cleanup_scratch_dir
  6. GraphRunner: crear scratch dir al inicio, limpiar al final
  7. ExecutionContext: exponer scratch_dir path
  8. system/bash: inyectar $MIRAI_SCRATCH_DIR como env var
  9. Tests

Fase 3 (output_files + auto-resolution):
  10. system/bash: output_files config → buscar archivos → producir FileRefs
  11. ai/transcribe: resolve_file_input para file_path
  12. ai/llm_call: resolve_file_input para media_path
  13. Tests de integracion

Fase 4 (validation):
  14. E2E: bash genera imagen → llm_call la analiza
  15. E2E: trigger pasa FileRef → transcribe lo consume
  16. E2E: backward compat (todo sin archivos funciona igual)
```
