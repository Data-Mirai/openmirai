# PRD-009 — Soporte Multimedia en AI Tools (Multimodal File Input)

| Campo | Valor |
|-------|-------|
| **ID** | PRD-009 |
| **Fecha** | 2026-05-28 |
| **Estado** | in_progress |
| **Branch** | prd/PRD-009 |
| **Target** | v0.5.0 |

---

## Diagrama General

```
┌─────────────────────────────────────────────────────────────────────────────┐
│                PRD-009 — MULTIMODAL FILE INPUT                              │
│                                                                             │
│  HOY (v0.4.5) — solo texto                                                 │
│  ──────────────────────────                                                 │
│                                                                             │
│  Developer escribe YAML:                                                    │
│    tool_type: ai/transcribe                                                 │
│    file_path: "/path/to/audio.m4a"                                          │
│         │                                                                   │
│         ▼                                                                   │
│  TranscribeTool.execute()                                                   │
│    prompt = "Transcribe the audio file at: /path/to/audio.m4a"              │
│         │                                                                   │
│         ▼                                                                   │
│  LLM recibe TEXTO con el path ──▶ "I cannot access local files"             │
│                                                                             │
│  DESPUES (v0.5.0) — multimodal                                              │
│  ─────────────────────────────                                              │
│                                                                             │
│  Developer escribe YAML (mismo config):                                     │
│    tool_type: ai/transcribe                                                 │
│    file_path: "/path/to/audio.m4a"                                          │
│         │                                                                   │
│         ▼                                                                   │
│  TranscribeTool.execute()                                                   │
│    1. read_media_file(file_path)                                            │
│       ├─ verificar existe + tamaño ≤ 20MB                                   │
│       ├─ detectar MIME: .m4a → audio/mp4                                    │
│       ├─ verificar MIME soportado por provider                              │
│       └─ leer bytes → codificar base64                                      │
│    2. construir mensaje multimodal                                          │
│       ├─ parte texto: "Transcribe this audio..."                            │
│       └─ parte media: {mime_type, base64_data}                              │
│         │                                                                   │
│         ▼                                                                   │
│  Adapter convierte a formato nativo del provider                            │
│                                                                             │
│  ┌─────────────┐  ┌─────────────┐  ┌─────────────┐  ┌───────────┐         │
│  │   Gemini     │  │   Claude    │  │   OpenAI    │  │   Ollama  │         │
│  │ inline_data  │  │ image block │  │  image_url  │  │  images[] │         │
│  │ audio+video  │  │ imagen only │  │ imagen+audio│  │imagen only│         │
│  │ +imagen      │  │             │  │             │  │           │         │
│  └─────────────┘  └─────────────┘  └─────────────┘  └───────────┘         │
│         │                                                                   │
│         ▼                                                                   │
│  LLM recibe ARCHIVO REAL ──▶ "Buenos dias, hoy vamos a hablar sobre..."     │
│                                                                             │
│  TAMBIEN INCLUIDO:                                                          │
│  ─────────────────                                                          │
│                                                                             │
│  ai/llm_call + media_path opcional                                          │
│  ┌───────────┐     ┌──────────────────┐     ┌───────────┐                  │
│  │ trigger    │──▶  │ ai/llm_call      │──▶  │ output    │                  │
│  │ image_path │     │ prompt + media   │     │ response  │                  │
│  └───────────┘     └──────────────────┘     └───────────┘                  │
│       data_map:                                                             │
│         media_path: "trigger.payload.image_path"                            │
│                                                                             │
│  system/bash + env vars from data_map                                       │
│  ┌───────────┐     ┌──────────────────┐                                    │
│  │ trigger    │──▶  │ system/bash      │                                    │
│  │ audio_path │     │ $MIRAI_audio_path│                                    │
│  └───────────┘     └──────────────────┘                                    │
│       data_map:                                                             │
│         audio_path: "trigger.payload.path"                                  │
│         → env: MIRAI_audio_path=/path/to/file                               │
└─────────────────────────────────────────────────────────────────────────────┘
```

---

## Problema

- **Tipo**: feature
- **Resumen**: Los tools ai/transcribe y ai/llm_call solo manejan texto. No pueden enviar archivos locales (audio, video, imagenes) como contenido multimodal a LLMs que lo soportan.
- **Actores**: Developer, Engine
- **Flujos tocados**: ai/transcribe execution, ai/llm_call execution, Message construction, LLM adapter request building, system/bash execution
- **Que cambia**:
  - HOY: ai/transcribe recibe file_path pero lo pasa como string al LLM ("Transcribe the audio file at: /path"). ai/llm_call solo maneja texto. system/bash no recibe data_map como env vars.
  - DESPUES: ai/transcribe lee el archivo real, lo codifica base64, y lo envia como contenido multimodal. ai/llm_call soporta media_path opcional para enviar archivos junto al prompt. system/bash recibe valores del data_map como variables de entorno.

---

## Actores y Permisos

| Actor | Capacidad | Accion | Visibilidad |
|-------|-----------|--------|-------------|
| Developer | disenar_agente | Define media_path en config/data_map de YAML | Spec completo |
| Engine | leer_archivo_media | Lee archivos del filesystem, codifica y envia al LLM | Interno |
| Engine | ejecutar_agente | Ejecuta ai tools con contenido multimodal | Resultado de ejecucion |

---

## Entidades

### MediaContent (nueva — runtime, no persistida)

| Campo | Tipo logico | Requerido | Descripcion |
|-------|-------------|-----------|-------------|
| mime_type | texto | si | Tipo MIME del archivo (audio/mp4, image/png, etc.) |
| data | texto | si | Contenido del archivo codificado en base64 |
| source_path | texto | no | Ruta original del archivo (para logging/debug) |

**Restricciones**: Existe solo en memoria durante la ejecucion del tool. No se persiste.

### SupportedMediaFormat (referencia — tabla estatica)

| Tipo | Extensiones | MIME Types | Providers |
|------|-------------|------------|-----------|
| Audio | .m4a, .mp3, .wav, .ogg, .flac | audio/mp4, audio/mpeg, audio/wav, audio/ogg, audio/flac | Gemini, OpenAI |
| Video | .mov, .mp4, .webm | video/quicktime, video/mp4, video/webm | Gemini |
| Imagen | .png, .jpg, .jpeg, .webp, .gif | image/png, image/jpeg, image/webp, image/gif | Gemini, Claude, OpenAI, Groq, Ollama |

### Message (modificada — runtime)

Campo nuevo:

| Campo | Tipo logico | Requerido | Descripcion |
|-------|-------------|-----------|-------------|
| media | lista de [MediaContent] | no | Contenido multimedia adjunto al mensaje |

**Backward compat**: `media` es opcional con default vacio. Messages sin media funcionan identico a hoy.

### LlmCallTool config (modificada)

Campos nuevos:

| Campo | Tipo logico | Requerido | Descripcion |
|-------|-------------|-----------|-------------|
| media_path | texto | no | Ruta a archivo multimedia. Se lee, codifica, y envia junto al prompt |

`media_path` se puede recibir via data_map (dinamico) o via config (estatico). data_map tiene prioridad.

### TranscribeTool (sin cambio de schema — cambio de comportamiento)

El input `file_path` que ya existe ahora se lee como archivo real en vez de pasar como texto.

### BashTool (sin cambio de schema — cambio de comportamiento)

Los inputs que llegan del data_map (excepto `command`) se inyectan como variables de entorno al subprocess.

---

## Ciclos de Vida

No aplica. MediaContent es efimero (runtime-only, no persistido). No tiene estados ni transiciones.

---

## Reglas de Negocio

### archivo-debe-existir

- **Invariante**: El file_path (transcribe) o media_path (llm_call) debe apuntar a un archivo existente y legible.
- **Cuando se verifica**: al iniciar la ejecucion del tool, antes de leer el archivo
- **Si se viola**: error: "media file not found: {path}"

### archivo-no-vacio

- **Invariante**: El archivo debe tener al menos 1 byte de contenido.
- **Cuando se verifica**: al leer el archivo
- **Si se viola**: error: "media file is empty: {path}"

### tamano-maximo-archivo

- **Invariante**: El archivo no debe exceder 20 MB. Base64 de 20MB = ~27MB, dentro del limite de la mayoria de APIs.
- **Cuando se verifica**: al leer el archivo (check size antes de leer bytes)
- **Si se viola**: error: "media file too large: {size_mb}MB (max 20MB)"

### extension-conocida

- **Invariante**: La extension del archivo debe estar en la tabla SupportedMediaFormat. Archivos sin extension o con extension desconocida son rechazados.
- **Cuando se verifica**: al detectar el MIME type
- **Si se viola**: error: "unsupported file extension: '.{ext}'. Supported: .m4a, .mp3, .wav, .ogg, .flac, .mov, .mp4, .webm, .png, .jpg, .jpeg, .webp, .gif"

### mime-soportado-por-provider

- **Invariante**: El MIME type detectado debe estar soportado por el provider activo (segun tabla SupportedMediaFormat).
- **Cuando se verifica**: despues de detectar MIME, antes de construir el request
- **Si se viola**: error: "media type '{mime}' not supported by provider '{provider}'. Supported: {list}"

### media-opcional-backward-compat

- **Invariante**: Todos los agents existentes (sin media_path, YAML v0.4.x) funcionan identicamente. La presencia de media es siempre opcional.
- **Cuando se verifica**: siempre
- **Si se viola**: N/A — por construccion

### bash-env-vars-prefijadas

- **Invariante**: Los valores de data_map inyectados como env vars usan prefijo `MIRAI_` para evitar colisiones con env vars del sistema. Key `audio_path` → env var `MIRAI_audio_path`.
- **Cuando se verifica**: al construir el subprocess
- **Si se viola**: N/A — por construccion

### provider-formato-nativo

- **Invariante**: Cada provider convierte MediaContent a su formato nativo de API. El engine produce un formato unificado; cada adapter lo traduce.
- **Cuando se verifica**: en la capa del adapter, al construir el request HTTP
- **Si se viola**: N/A — por construccion

---

## Patrones de Diseno

### Adapter → Provider-specific multimodal format

- **Aplica a**: Conversion de MediaContent a formato nativo de cada provider (Gemini, Claude, OpenAI, Ollama, Groq)
- **Por que**: Cada LLM API tiene su propio formato para contenido multimodal. El engine produce un formato unificado (MediaContent) y cada adapter lo traduce a su formato nativo sin que los tools conozcan los detalles.
- **Participantes**: MediaContent (formato unificado), GeminiAdapter/ClaudeAdapter/OpenAICompatAdapter/OllamaAdapter (traductores)

### Strategy → MIME detection

- **Aplica a**: Deteccion de MIME type por extension de archivo
- **Por que**: Mapa estatico extension → MIME type. Simple, deterministico, extensible.
- **Participantes**: Extension map (estrategia), read_media_file (contexto)

### Template Method → Tool execution con media pre-processing

- **Aplica a**: ai/transcribe y ai/llm_call ambos siguen: leer media → construir mensaje → llamar LLM
- **Por que**: El pre-procesamiento (read + encode + validate) es identico para ambos tools. Se comparte como funcion utilitaria, cada tool la invoca con su propio prompt.
- **Participantes**: read_media_file (shared step), TranscribeTool/LlmCallTool (consumidores)

---

## Operaciones

### read_media_file (interna)

- **Actor**: Engine
- **Input**:
  - file_path (texto, requerido) — ruta absoluta o relativa al archivo
  - provider_name (texto, requerido) — nombre del provider activo
- **Logica**:
  1. Verificar que archivo existe → error si no
  2. Verificar que tamaño > 0 → error si vacio
  3. Verificar que tamaño <= 20MB → error si excede
  4. Extraer extension del path
  5. Mapear extension a MIME type via tabla estatica → error si extension desconocida
  6. Verificar que MIME esta soportado por provider → error si no
  7. Leer bytes del archivo
  8. Codificar a base64
  9. Retornar MediaContent
- **Output exitoso**: MediaContent con mime_type, data (base64), source_path
- **Errores posibles**:
  - "media file not found: {path}"
  - "media file is empty: {path}"
  - "media file too large: {size_mb}MB (max 20MB)"
  - "unsupported file extension: '.{ext}'. Supported: ..."
  - "media type '{mime}' not supported by provider '{provider}'. Supported: {list}"
  - "failed to read media file '{path}': {io_error}"

### transcribe_audio (modificada — ai/transcribe.execute)

- **Actor**: Engine
- **Input**: file_path (texto, requerido)
- **Config**: model (texto, opcional)
- **Logica nueva**:
  1. read_media_file(file_path, provider_name) → MediaContent
  2. Construir prompt: "Transcribe this audio. Output only the raw transcription text, no timestamps, no speaker labels."
  3. Construir mensaje con parte texto (prompt) + parte media (MediaContent)
  4. Llamar LLM con mensaje multimodal
  5. Retornar respuesta como text
- **Output**: text (texto), duration_seconds (numero, placeholder 0.0)
- **Errores**: todos los de read_media_file + errores del LLM

### llm_call_con_media (modificada — ai/llm_call.execute)

- **Actor**: Engine
- **Input**: prompt (texto), media_path (texto, opcional — nuevo)
- **Config**: model, temperature, max_tokens, system_prompt (sin cambio) + media_path (nuevo, opcional)
- **Logica nueva**:
  1. Resolver media_path: inputs["media_path"] || config["media_path"] || None
  2. Si media_path presente y no vacio:
     a. read_media_file(media_path, provider_name) → MediaContent
     b. Adjuntar MediaContent al mensaje del usuario (junto con prompt)
  3. Si media_path ausente o None: comportamiento identico a hoy
  4. Resto del flujo sin cambio (system_prompt, schema validation, retries)
- **Output**: sin cambio (response, model, tokens_input, tokens_output, etc.)
- **Errores**: todos los de read_media_file (si media_path presente) + errores existentes

### bash_con_env_vars (modificada — system/bash.execute)

- **Actor**: Engine
- **Input**: command (texto, requerido) + inputs adicionales del data_map
- **Config**: timeout, cwd (sin cambio)
- **Logica nueva**:
  1. Para cada input que no sea "command":
     - Crear env var: `MIRAI_{key}` = valor (stringified: String as-is, Number/Bool to_string, Object JSON.stringify)
  2. Inyectar env vars al proceso hijo via Command.env()
  3. Resto del flujo sin cambio
- **Output**: sin cambio (stdout, stderr, exit_code, timed_out)

### convert_media_to_provider_format (interna — por adapter)

- **Actor**: Engine (cada LLM adapter)
- **Input**: Message con media adjunto (lista de MediaContent)
- **Logica por provider**:

  **Gemini**: Cada MediaContent → parte adicional en el array `parts`:
  ```
  parts: [
    {"text": "prompt..."},
    {"inline_data": {"mime_type": "audio/mp4", "data": "<base64>"}}
  ]
  ```
  Soporta: audio, video, imagen.

  **Claude**: Solo imagenes. Cada MediaContent de imagen → content block:
  ```
  content: [
    {"type": "text", "text": "prompt..."},
    {"type": "image", "source": {"type": "base64", "media_type": "image/png", "data": "<base64>"}}
  ]
  ```
  Audio/video → error en read_media_file (mime-soportado-por-provider).

  **OpenAI-compatible** (OpenAI, Groq, OpenRouter, NVIDIA): Imagenes → content array:
  ```
  content: [
    {"type": "text", "text": "prompt..."},
    {"type": "image_url", "image_url": {"url": "data:image/jpeg;base64,<data>"}}
  ]
  ```
  OpenAI tambien soporta audio input. Video → error.

  **Ollama**: Solo imagenes. Campo `images` en el mensaje:
  ```
  {"role": "user", "content": "prompt...", "images": ["<base64>"]}
  ```
  Audio/video → error.

- **Output**: Request JSON nativo del provider con media incluido

---

## Interfaces

> Engine no tiene frontend. Las interfaces son tools YAML, CLI, y HTTP API.

### ai/transcribe — YAML (sin cambio de schema, cambio de comportamiento)

**Antes (no funciona)**:
```yaml
- id: stt
  tool_type: ai/transcribe
  config:
    model: "gemini-2.5-flash"
# Engine construye: "Transcribe the audio file at: /path/to/file.m4a"
# LLM responde: "I cannot access local files on your computer"
```

**Despues (funciona)**:
```yaml
- id: stt
  tool_type: ai/transcribe
  config:
    model: "gemini-2.5-flash"
# Engine LEE el archivo .m4a, codifica base64, envia como inline_data
# LLM recibe audio real + prompt de transcripcion
# LLM responde: "Buenos dias, hoy vamos a hablar sobre..."
```

### ai/llm_call con media_path — YAML (campo nuevo)

**Analisis de imagen**:
```yaml
name: image-analyzer
version: v1

graph:
  nodes:
    - id: start
      tool_type: trigger/manual
    - id: analyze
      tool_type: ai/llm_call
      config:
        prompt: "Describe what you see in this image in detail."
        model: "gemini-2.5-flash"
    - id: respond
      tool_type: output/response

  edges:
    - source: start
      target: analyze
      data_map:
        media_path: "start.payload.image_path"
    - source: analyze
      target: respond
```

**Transcripcion + resumen multi-step**:
```yaml
name: meeting-analyzer
version: v1

graph:
  nodes:
    - id: start
      tool_type: trigger/manual
    - id: transcribe
      tool_type: ai/transcribe
      config:
        model: "gemini-2.5-flash"
    - id: summarize
      tool_type: ai/llm_call
      config:
        prompt: "Summarize this meeting transcript in 3 bullet points."
        model: "gemini-2.5-flash"
    - id: respond
      tool_type: output/response

  edges:
    - source: start
      target: transcribe
      data_map:
        file_path: "start.payload.audio_path"
    - source: transcribe
      target: summarize
      data_map:
        prompt: "Summarize this meeting transcript in 3 bullet points:\n\n${transcribe.text}"
    - source: summarize
      target: respond
```

**Video analysis (Gemini)**:
```yaml
- id: analyze_video
  tool_type: ai/llm_call
  config:
    prompt: "Describe key moments in this screen recording."
    model: "gemini-2.5-flash"
# edge data_map: media_path: "start.payload.video_path"
```

### system/bash con env vars — YAML

**Antes (data_map values no llegan)**:
```yaml
edges:
  - source: start
    target: my_bash
    data_map:
      audio_path: "start.payload.path"
# En bash: $audio_path esta vacio, $MIRAI_audio_path no existe
```

**Despues (data_map values como env vars)**:
```yaml
edges:
  - source: start
    target: my_bash
    data_map:
      audio_path: "start.payload.path"
# En bash: $MIRAI_audio_path = "/path/to/file.m4a"
```

### CLI: mirai run (sin cambio de interfaz)

```
# Transcripcion de audio
mirai run agents/stt.yaml --input '{"audio_path": "/path/to/recording.m4a"}'

# Analisis de imagen
mirai run agents/image-analyzer.yaml --input '{"image_path": "/path/to/screenshot.png"}'

# Video analysis
mirai run agents/video-analyzer.yaml --input '{"video_path": "/path/to/recording.mov"}'
```

### Error messages (nuevos)

| Condicion | Mensaje |
|-----------|---------|
| Archivo no encontrado | `media file not found: /path/to/file.xyz` |
| Archivo vacio | `media file is empty: /path/to/empty.m4a` |
| Archivo demasiado grande | `media file too large: 45.2MB (max 20MB)` |
| Extension no soportada | `unsupported file extension: '.xyz'. Supported: .m4a, .mp3, .wav, .ogg, .flac, .mov, .mp4, .webm, .png, .jpg, .jpeg, .webp, .gif` |
| Provider no soporta tipo | `media type 'audio/mp4' not supported by provider 'claude'. Supported for claude: image/png, image/jpeg, image/webp, image/gif` |
| Error de lectura | `failed to read media file '/path/to/file': Permission denied` |

### mirai tools output (modificado)

`mirai tools` debe reflejar el nuevo campo `media_path` en ai/llm_call y el comportamiento actualizado de ai/transcribe.

---

## Matriz de Permutaciones

### ai/transcribe

| Flujo | Permutacion | Actor | Resultado |
|---|---|---|---|
| transcribe_audio | Happy: .m4a valido, Gemini | Engine | Transcripcion texto en output.text |
| transcribe_audio | Happy: .mp3 valido, OpenAI | Engine | Transcripcion texto en output.text |
| transcribe_audio | Happy: .wav valido, Gemini | Engine | Transcripcion texto en output.text |
| transcribe_audio | Archivo no encontrado | Engine | Error: media file not found |
| transcribe_audio | Archivo vacio (0 bytes) | Engine | Error: media file is empty |
| transcribe_audio | Archivo >20MB | Engine | Error: media file too large |
| transcribe_audio | Extension desconocida (.xyz) | Engine | Error: unsupported file extension |
| transcribe_audio | Sin extension | Engine | Error: unsupported file extension |
| transcribe_audio | Provider no soporta audio (Claude) | Engine | Error: media type not supported |
| transcribe_audio | Permission denied | Engine | Error: failed to read media file |

### ai/llm_call con media

| Flujo | Permutacion | Actor | Resultado |
|---|---|---|---|
| llm_call_media | Happy: imagen .png + prompt, Gemini | Engine | Response con analisis |
| llm_call_media | Happy: imagen .png + prompt, Claude | Engine | Response con analisis |
| llm_call_media | Happy: imagen .jpg + prompt, OpenAI | Engine | Response con analisis |
| llm_call_media | Happy: audio .m4a + prompt, Gemini | Engine | Response con analisis |
| llm_call_media | Happy: video .mp4 + prompt, Gemini | Engine | Response con analisis |
| llm_call_media | Sin media_path (solo texto) | Engine | Comportamiento identico a hoy |
| llm_call_media | media_path presente pero vacio | Engine | Ignorar (tratar como sin media) |
| llm_call_media | Archivo no encontrado | Engine | Error: media file not found |
| llm_call_media | Extension no soportada | Engine | Error: unsupported file extension |
| llm_call_media | Provider no soporta tipo | Engine | Error: media type not supported |
| llm_call_media | media_path via data_map (dinamico) | Engine | Resuelve expression, lee archivo |
| llm_call_media | media_path via config (estatico) | Engine | Lee archivo desde path literal |

### system/bash env vars

| Flujo | Permutacion | Actor | Resultado |
|---|---|---|---|
| bash_env | Happy: data_map con valor string | Engine | $MIRAI_{key} disponible en subprocess |
| bash_env | data_map con valor numerico | Engine | Stringify del numero como env var |
| bash_env | data_map con valor JSON object | Engine | JSON stringify como env var |
| bash_env | data_map con valor null | Engine | Env var con string "null" |
| bash_env | data_map vacio | Engine | Sin env vars extra (backward compat) |
| bash_env | Sin data_map (como hoy) | Engine | Comportamiento identico a hoy |
| bash_env | Key command en data_map | Engine | Se usa como command, NO como env var |

### Provider-specific media conversion

| Flujo | Permutacion | Actor | Resultado |
|---|---|---|---|
| provider_media | Gemini + audio | Engine | inline_data part con audio/mp4 |
| provider_media | Gemini + video | Engine | inline_data part con video/mp4 |
| provider_media | Gemini + imagen | Engine | inline_data part con image/png |
| provider_media | Claude + imagen | Engine | image content block con base64 |
| provider_media | OpenAI + imagen | Engine | image_url content part |
| provider_media | Ollama + imagen | Engine | images array con base64 |
| provider_media | Groq + imagen | Engine | image_url content part (hereda OpenAI) |
| provider_media | Message sin media | Engine | Request identico a hoy |

### Backward compatibility

| Flujo | Permutacion | Actor | Resultado |
|---|---|---|---|
| backward_compat | YAML v0.4.x sin media_path | Developer | Funciona identico |
| backward_compat | ai/transcribe con provider texto-only | Engine | Error claro de provider no soportado |
| backward_compat | Messages sin campo media | Engine | Todos los adapters funcionan sin cambio |
| backward_compat | system/bash sin data_map extra | Engine | Sin env vars adicionales |

---

## Escenarios GWT

### Journey: Engine — Transcribe Audio

TEST-095: Transcripcion audio .m4a con Gemini — happy path
  Given: Agent con ai/transcribe, model: gemini-2.5-flash, archivo .m4a valido de 2MB
  When: Engine ejecuta ai/transcribe con file_path apuntando a dictation.m4a
  Then: output.text contiene transcripcion del audio. duration_seconds = 0.0 (placeholder).

TEST-096: Transcripcion — archivo no encontrado
  Given: Agent con ai/transcribe, file_path apunta a archivo inexistente
  When: Engine ejecuta ai/transcribe
  Then: ToolError con message "media file not found: /path/to/missing.m4a"

TEST-097: Transcripcion — archivo demasiado grande
  Given: Agent con ai/transcribe, archivo de 25MB
  When: Engine ejecuta ai/transcribe
  Then: ToolError con message "media file too large: 25.0MB (max 20MB)"

TEST-098: Transcripcion — extension no soportada
  Given: Agent con ai/transcribe, file_path = "/path/to/file.xyz"
  When: Engine ejecuta ai/transcribe
  Then: ToolError con message "unsupported file extension: '.xyz'"

TEST-099: Transcripcion — provider no soporta audio
  Given: Agent con ai/transcribe, provider activo es Claude
  When: Engine ejecuta ai/transcribe con archivo .m4a
  Then: ToolError con message "media type 'audio/mp4' not supported by provider 'claude'"

TEST-100: Transcripcion — archivo vacio
  Given: Agent con ai/transcribe, archivo .m4a de 0 bytes
  When: Engine ejecuta ai/transcribe
  Then: ToolError con message "media file is empty: /path/to/empty.m4a"

### Journey: Engine — LLM Call con Media

TEST-101: Analisis imagen con Gemini — happy path
  Given: Agent con ai/llm_call, prompt: "Describe this image", media_path a screenshot.png de 500KB
  When: Engine ejecuta ai/llm_call
  Then: output.response contiene descripcion de la imagen. output.model = modelo usado.

TEST-102: Analisis imagen con Claude — happy path
  Given: Agent con ai/llm_call, prompt: "Describe this image", media_path a screenshot.png, provider Claude
  When: Engine ejecuta ai/llm_call
  Then: output.response contiene descripcion. Claude recibe image content block.

TEST-103: Analisis video con Gemini — happy path
  Given: Agent con ai/llm_call, prompt: "Describe key moments", media_path a video.mp4 de 15MB, provider Gemini
  When: Engine ejecuta ai/llm_call
  Then: output.response contiene descripcion del video.

TEST-104: LLM call sin media_path — backward compat
  Given: Agent con ai/llm_call, solo prompt texto, sin media_path en inputs ni config
  When: Engine ejecuta ai/llm_call
  Then: Comportamiento identico a v0.4.5. Sin lectura de archivo. Sin media en mensaje.

TEST-105: LLM call con media_path vacio
  Given: Agent con ai/llm_call, media_path = "" (string vacio)
  When: Engine ejecuta ai/llm_call
  Then: Se ignora media_path vacio. Funciona como solo texto.

TEST-106: LLM call con video + Claude (no soportado)
  Given: Agent con ai/llm_call, media_path a video.mp4, provider Claude
  When: Engine ejecuta ai/llm_call
  Then: ToolError: "media type 'video/mp4' not supported by provider 'claude'"

### Journey: Engine — System/Bash con Env Vars

TEST-107: Bash recibe data_map como env vars — happy path
  Given: Agent con system/bash, command: "echo $MIRAI_audio_path"
  When: Engine ejecuta con data_map input audio_path = "/tmp/file.m4a"
  Then: stdout = "/tmp/file.m4a". exit_code = 0.

TEST-108: Bash recibe multiples valores del data_map
  Given: Agent con system/bash, command: "echo $MIRAI_name $MIRAI_count"
  When: Engine ejecuta con data_map inputs name = "test" y count = 42
  Then: stdout contiene "test 42". exit_code = 0.

TEST-109: Bash sin data_map extra — backward compat
  Given: Agent con system/bash, command: "echo hello", sin inputs adicionales
  When: Engine ejecuta system/bash
  Then: stdout = "hello". Sin env vars MIRAI_* extra. Identico a hoy.

TEST-110: Bash con valor JSON object en data_map
  Given: Agent con system/bash, command: "echo $MIRAI_data"
  When: Engine ejecuta con data_map input data = {"key": "value"}
  Then: stdout = '{"key":"value"}'. JSON stringified.

### Journey: Engine — Provider Media Conversion

TEST-111: Gemini adapter — audio inline_data
  Given: Message con content "Transcribe" + media: [MediaContent(audio/mp4, base64)]
  When: GeminiAdapter construye request
  Then: Request contiene parts: [{"text": "Transcribe"}, {"inline_data": {"mime_type": "audio/mp4", "data": "..."}}]

TEST-112: Claude adapter — imagen content block
  Given: Message con content "Describe" + media: [MediaContent(image/png, base64)]
  When: ClaudeAdapter construye request
  Then: Request contiene content: [{"type": "text", "text": "Describe"}, {"type": "image", "source": {"type": "base64", "media_type": "image/png", "data": "..."}}]

TEST-113: OpenAI adapter — imagen image_url
  Given: Message con content "Describe" + media: [MediaContent(image/jpeg, base64)]
  When: OpenAICompatAdapter construye request
  Then: Request contiene content: [{"type": "text", "text": "Describe"}, {"type": "image_url", "image_url": {"url": "data:image/jpeg;base64,..."}}]

TEST-114: Cualquier adapter — message sin media (backward compat)
  Given: Message con content "Hello", media = None
  When: Cualquier adapter construye request
  Then: Request identico a la implementacion actual. Sin campos de media.

---

## Fuera de Alcance

- **Streaming de archivos grandes**: v1 lee todo el archivo en memoria. Streaming/chunked upload para >20MB es futuro.
- **Media URL (archivos remotos)**: v1 solo soporta archivos locales. Soporte de URLs http:// es futuro.
- **Gemini File API**: Gemini tiene File API para archivos >20MB (upload → referencia). v1 no la usa; todo va como inline_data.
- **Media en output (generacion)**: v1 solo soporta media como INPUT. TTS, image generation son futuro.
- **OCR / PDF processing**: No se incluye soporte especifico para PDFs. Solo formatos multimedia crudos.
- **Audio duration detection**: duration_seconds retorna 0.0 como placeholder. Parseo real es futuro.
- **Multiples archivos por mensaje**: v1 soporta un archivo por tool invocation (un media_path/file_path). Multiples archivos es futuro.
- **Media en system_prompt**: Solo el mensaje del usuario lleva media. System prompt sigue siendo texto puro.

---

## Dependencias

- **No depende de PRD-008** (Live Agent): feature independiente, se puede implementar en paralelo o secuencial.
- **Reutiliza LLMAdapter trait**: se extiende Message con campo media opcional.
- **Reutiliza adapter_bridge**: se extiende para pasar media del LLMResource al LLMAdapter.
- **Reutiliza tool execution pipeline**: read_media_file es interna al tool, no cambia el runner.
- **Sin dependencias externas nuevas**: `base64` ya existe en Rust stdlib. MIME mapping es un HashMap estatico.

### Orden de implementacion recomendado

```
Fase 1 (foundation — media infrastructure):
  1. MediaContent struct (nuevo en llm/adapter.rs o llm/media.rs)
  2. MIME detection map (extension → mime type)
  3. Provider support map (provider → mime types soportados)
  4. read_media_file function (read + validate + encode)
  5. Message.media field (Option<Vec<MediaContent>>)
  6. Tests unitarios: read_media_file con archivos reales y edge cases

Fase 2 (provider adapters):
  7. Gemini adapter — inline_data parts para audio, video, imagen
  8. Claude adapter — image content blocks
  9. OpenAI-compat adapter — image_url + audio content
  10. Ollama adapter — images field
  11. Adapter bridge — pasar media de context Value a Message.media
  12. Tests unitarios por adapter (message → request JSON)

Fase 3 (tools):
  13. ai/transcribe — leer archivo + enviar multimodal (reemplaza placeholder)
  14. ai/llm_call — media_path input/config + enviar multimodal
  15. system/bash — inyectar data_map inputs como env vars MIRAI_*
  16. Tests de integracion por tool

Fase 4 (validation con app real):
  17. E2E: transcripcion dictation.m4a via Gemini (validar con Aftrmeet)
  18. E2E: analisis de imagen via Claude + Gemini
  19. E2E: bash con env vars del data_map
  20. mirai tools output actualizado
```
