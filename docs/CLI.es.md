# OpenMirai — Referencia de CLI (`mirai`)

> 🌐 Available in [English](CLI.md).

El binario `mirai` es la forma principal de ejecutar OpenMirai localmente. Tiene dos modos distintos:

- **Comandos no interactivos** — subcomandos de ejecución única (`run`, `validate`, `serve`, `tools`, …) que se ejecutan y finalizan. Estos se mapean al `GraphRunner` del motor y son los que se usan en scripts, integraciones y llamadas desde CI.
- **Modo interactivo** — ejecutar `mirai` sin argumentos lanza un asistente de configuración seguido de una terminal de agente por chat. Este modo **no** utiliza el ejecutor de grafos; controla un adaptador de LLM directamente a través de un bucle de llamada a herramientas (tool-calling).

Fuente: `cli/src/main.rs` (despacho de comandos), `cli/src/terminal.rs` (bucle interactivo), `cli/src/setup_wizard.rs` (asistente), `cli/src/session_storage.rs` (persistencia), `cli/src/adapter_factory.rs` (proveedor→adaptador), `cli/src/colors.rs` (salida ANSI).

Documentación relacionada: [ARCHITECTURE.md](ARCHITECTURE.md) · [backend/API.md](backend/API.md) (la superficie HTTP de `serve`) · [frontend/PANTALLAS.md](frontend/PANTALLAS.md) y [frontend/DESIGN-GUIDE.md](frontend/DESIGN-GUIDE.md) (UX de la terminal) · [USAGE.md](../USAGE.md) (autoría de agentes en YAML).

---

## 1. Construcción del binario

```bash
# Desde la raíz del repositorio
cargo build --release
./target/release/mirai version       # → mirai 0.7.0

# O ejecutar sin optimizar durante el desarrollo
cargo run -p openmirai-cli -- run examples/hello-world.yaml
```

Todos los ejemplos a continuación asumen que `mirai` está en tu `PATH` (o sustitúyelo por `./target/release/mirai`).

---

## 2. Superficie de comandos

`mirai <comando> [args]`. El primer argumento selecciona el comando; cualquier otro argumento se procesa posicionalmente o como pares de `--flag valor`.

| Comando | ¿Interactivo? | Qué hace |
|---|---|---|
| `mirai` (sin argumentos) | **Sí** | Asistente de configuración → terminal de agente interactiva |
| `mirai run <archivo>` | No | Ejecuta una especificación YAML una vez e imprime el JSON resultante |
| `mirai validate <archivo>` | No | Analiza e informa el conteo de nodos/aristas; salida no cero en caso de error |
| `mirai serve` | No | Inicia el servidor HTTP (ver [API.md](backend/API.md)) |
| `mirai tools [<tool_type>]` | No | Lista todas las herramientas, o muestra las entradas/salidas/configuración de una herramienta |
| `mirai templates` | No | Lista las plantillas de agentes integradas |
| `mirai new --template <id> --name <n>` | No | Crea un archivo `<n>.yaml` a partir de una plantilla |
| `mirai describe <archivo>` | No | Imprime el contrato de entrada/salida de un agente |
| `mirai eval --output <texto> …` | No | Ejecuta evaluadores (relevancia, latencia, …) sobre un par de entrada/salida |
| `mirai rag search --query … --documents …` | No | Divide + incrusta (embed) + clasifica por coseno documentos locales |
| `mirai agent <load\|list>` | No | **Stubs** — imprime "not yet implemented" |
| `mirai version` (`--version`, `-V`) | No | Imprime la versión |
| `mirai help` (`--help`, `-h`) | No | Imprime el uso |

Los comandos desconocidos imprimen `Unknown command: <x>. Run \`mirai help\` for usage.` en stderr y salen con código `1`.

> **No existe el comando `mirai play`** aunque `mirai run` se refiere a él (ver [§11 Resolución de problemas](#11-modos-comunes-de-falla-y-resolución-de-problemas)).

---

## 3. Detección y configuración de proveedores

La mayoría de los comandos que interactúan con un LLM (`run`, `serve`, `eval`, `rag`) resuelven el proveedor, modelo, clave de API y URL base utilizando la misma cadena (`resolve_provider` en `main.rs`).

### Orden de resolución

**Proveedor:**
1. Flag `--provider <nombre>`
2. Variable de entorno `MIRAI_LLM_PROVIDER`
3. Auto-detección a partir del nombre del modelo (ver tabla abajo)
4. Por defecto: `ollama`

**Modelo:**
1. Flag `--model <nombre>`
2. Variable de entorno `MIRAI_LLM_MODEL`
3. Modelo por defecto del proveedor desde `adapter_factory::default_model`

### Auto-detección Modelo → proveedor

`detect_provider_from_model` inspecciona el nombre del modelo en minúsculas:

| Patrón de nombre de modelo | Proveedor detectado |
|---|---|
| `gpt*`, `o1*`, `o3*`, `o4*` | `openai` |
| `claude*` | `claude` |
| `gemini*`, `gemma*` | `gemini` |
| contiene `llama`, `mistral`, `qwen`, `phi`, `deepseek` | `ollama` |
| cualquier otra cosa | (sin detección → pasa al valor por defecto) |

### Proveedores soportados y valores por defecto

Desde `adapter_factory.rs`:

| `--provider` | Modelo por defecto | Entorno para clave API (o `--api-key`) | URL base por defecto |
|---|---|---|---|
| `ollama` (defecto) | `qwen3:8b` | ninguno (local) | `OLLAMA_BASE_URL` o `http://localhost:11434` |
| `openai` | `gpt-4o` | `OPENAI_API_KEY` | `https://api.openai.com/v1` |
| `claude` / `anthropic` | `claude-sonnet-4-20250514` | `ANTHROPIC_API_KEY` | `https://api.anthropic.com/v1` |
| `gemini` / `google` | `gemini-2.5-flash` | `GOOGLE_API_KEY` | `https://generativelanguage.googleapis.com/v1beta` |
| `groq` | `qwen-qwq-32b` | `GROQ_API_KEY` | (defecto del adaptador) |
| `nvidia` | `meta/llama-3.3-70b-instruct` | `NVIDIA_API_KEY` | (defecto del adaptador) |
| `openrouter` | `meta-llama/llama-3.3-70b-instruct` | `OPENROUTER_API_KEY` | (defecto del adaptador) |
| `mock` | — | — | usa `MockLLMResource` (solo pruebas) |
| *(desconocido)* | `qwen3:8b` | opcional | tratado como compatible con OpenAI, por defecto `http://localhost:11434/v1` |

Una clave de API se resuelve como: valor explícito de `--api-key` → variable de entorno del proveedor → cadena vacía. Para `claude`/`gemini`, una clave vacía imprime una advertencia pero aun así construye el adaptador.

`--provider mock` intercambia a `MockLLMResource` para pruebas deterministas — válido tanto para `run` como para `serve`.

---

## 4. Comandos no interactivos

### `mirai run <archivo>` — ejecutar un agente una vez

```bash
mirai run agent.yaml
mirai run agent.yaml --input '{"query": "hola"}'
mirai run agent.yaml --provider ollama --model gemma4
mirai run agent.yaml --provider openai --model gpt-4o
mirai run agent.yaml --provider claude --api-key $ANTHROPIC_API_KEY
mirai run agent.yaml --trace            # imprime árbol de traza + métricas en stderr
mirai run agent.yaml --benchmark        # registra tiempos en benchmarks.jsonl
```

**Flags:**

| Flag | Significado |
|---|---|
| `-i`, `--input <JSON>` | Payload de entrada. Los objetos se convierten en el payload del trigger; los no-objetos se envuelven como `{"_raw": …}` |
| `--provider`, `--model`, `--api-key`, `--base-url` | Configuración del proveedor (ver §3) |
| `--trace` | Después de la ejecución, renderiza el árbol de traza + métricas (nodos, ms totales/promedio, reintentos) en stderr |
| `--benchmark` | Habilita el registro de benchmarks (también vía `MIRAI_BENCHMARK=1`); archivo vía `MIRAI_BENCHMARK_FILE` (defecto `benchmarks.jsonl`) |

**Pipeline de ejecución** (este es el mapeo canónico CLI→motor; ver también §9):

1. `AgentSpec::from_file(path)` analiza una especificación `.yaml` o `.yml`. Los archivos JSON se rechazan.
2. Si `agent_type == Live`, la ejecución es **rechazada** (usa un servidor / ejecutor live).
3. `spec.to_graph()` → `auto_generate_edge_ids()` → `graph.validate()`.
4. Se puebla un `ToolRegistry` mediante `register_all_builtin_tools`.
5. Un `RegistryExecutor` se envuelve en un `GraphRunner`.
6. Si la especificación declara un `soul`, este se carga y se convierte en el prompt de sistema; de lo contrario se usa `spec.system_prompt`.
7. El `ExecutionContext` se construye con el LLM resuelto más una base de datos y almacenamiento **en memoria** (`InMemoryDBResource`, `InMemoryStorageResource`).
8. Si se proporciona `--input` y la especificación declara `inputs`, el payload se valida (`validate_agent_inputs`); las fallas salen con código `1` con errores por campo. El payload validado se inyecta en el primer nodo `trigger/*`.
9. El `model` se inyecta en cualquier nodo `ai/llm_call` que no haya configurado uno; las configuraciones del servidor MCP se inyectan en los nodos `mcp/call`.
10. `GraphRunner::run` ejecuta el grafo.

**Salida** — JSON formateado en stdout:

```json
{
  "status": "Completed",
  "state": { "...": "final state snapshot" },
  "trace": [ /* registros de ejecución por nodo */ ],
  "transcript": [ /* transcripción de mensajes */ ],
  "error": null,
  "interrupt_info": null
}
```

**Códigos de salida:** `0` cuando `status == Completed`; `1` en errores de carga/validación, falla de validación de entrada, rechazo de agente live, o cualquier estado que no sea `Completed`. La información del proveedor (`LLM: <provider>/<model>`) y las líneas de alma/advertencia van a **stderr**, por lo que stdout permanece como JSON limpio para tuberías (pipes).

> **Solo en memoria:** `mirai run` le otorga al agente una base de datos/almacenamiento efímeros. Las herramientas de datos y almacenamiento no persistirán entre ejecuciones. Usa `mirai serve` para un contexto de mayor duración.

### `mirai validate <archivo>`

```bash
mirai validate my-agent.yaml
# → ✓ Valid agent spec: 'my-agent' (3 nodes, 2 edges)
```

Analiza la especificación e informa los conteos de nodos/aristas. Las especificaciones inválidas imprimen `✗ Invalid agent spec: <error>` en stderr y salen con código `1`. **No** ejecuta el grafo ni contacta a ningún LLM.

### `mirai serve`

```bash
mirai serve --port 3000
mirai serve --host 127.0.0.1 --port 8080 --provider openai --model gpt-4o
MIRAI_API_KEY=secret mirai serve --port 3000      # requiere X-API-Key
```

| Flag / entorno | Defecto | Significado |
|---|---|---|
| `--port` | `3000` | Puerto de escucha |
| `--host` | `0.0.0.0` | Dirección de enlace |
| `--provider` / `--model` / `--api-key` / `--base-url` | ver §3 | Configuración de LLM |
| `--api-key` *(servidor)* / `MIRAI_API_KEY` | ninguno | Si se configura, las peticiones deben enviar un `X-API-Key` coincidente |

El servidor construye un recurso LLM **real** por petición a través de una fábrica de adaptadores (`mock` solo se usa cuando `--provider mock` se configura explícitamente). Los endpoints y contratos de petición/respuesta viven en [backend/API.md](backend/API.md).

### `mirai tools [<tool_type>]`

```bash
mirai tools                    # todas las herramientas, agrupadas por categoría
mirai tools ai/claude_code     # entradas, salidas y configuración para una herramienta
```

La vista de lista agrupa cada herramienta registrada por categoría y trunca las descripciones a 60 caracteres. La vista de detalle imprime los campos `INPUTS` (recibidos vía `data_map`), `OUTPUTS` (utilizables en el `data_map` de la siguiente arista) y `CONFIG` de la herramienta (indicando obligatorios/opcionales y valores por defecto). Los tipos de herramientas desconocidos salen con código `1`.

### `mirai templates` / `mirai new`

```bash
mirai templates
mirai new --template <id> --name my-agent
mirai new --template <id> --name my-agent --provider ollama
```

`templates` lista las plantillas integradas (id, categoría, descripción). `new` clona la especificación de una plantilla, sobrescribe el `name`, opcionalmente inyecta el `provider` en cada nodo `ai/llm_call` y escribe `<name>.yaml` en el directorio actual. Si falta `--template` se imprime el uso y sale con código `1`; un id de plantilla desconocido sale con código `1`. `--name` toma el id de la plantilla por defecto si se omite.

### `mirai describe <archivo>`

```bash
mirai describe my-agent.yaml
```

Imprime el nombre/versión/descripción del agente y sus contratos declarados de `inputs` y `outputs` (nombre, tipo, obligatorio/opcional, descripción). Los agentes sin entradas declaradas muestran `(none declared — accepts any payload)`.

### `mirai eval`

```bash
mirai eval --input "¿Cuánto es 2+2?" --output "4" --types relevance,format_compliance
```

| Flag | Defecto | Significado |
|---|---|---|
| `--input <texto>` | vacío | El prompt/entrada original |
| `--output <texto>` | — | **Obligatorio** — la respuesta a calificar |
| `--types <lista>` | `format_compliance,latency` | Evaluadores separados por comas |

`--types` válidos: `relevance`, `faithfulness`, `completeness`, `format_compliance`, `latency` (los nombres desconocidos se omiten silenciosamente). Los resultados se imprimen como un gráfico de barras etiquetado con puntuaciones en `[0,1]`. Si falta `--output` sale con código `1`.

### `mirai rag search`

```bash
mirai rag search --query "bases de datos vectoriales" --documents notas.md,paper.txt --top-k 5
```

| Flag | Defecto | Significado |
|---|---|---|
| `--query <texto>` | — | **Obligatorio** consulta de búsqueda |
| `--documents <rutas>` | — | **Obligatorio** rutas de archivos separadas por comas |
| `--top-k <n>` | `3` | Número de resultados |

Lee cada documento, lo divide por párrafos (tamaño 512 / solapamiento 50), incrusta la consulta y cada fragmento a través de los embeddings del proveedor resuelto, clasifica por similitud de coseno e imprime las vistas previas de los fragmentos principales con sus puntuaciones. Los archivos no legibles generan una advertencia y se omiten; si no se carga ningún documento, sale con código `1`. Cualquier subcomando distinto de `search` imprime el uso y sale con código `1`.

> `rag search` necesita un proveedor que soporte embeddings. Con Ollama, asegúrate de que esté disponible un modelo/endpoint capaz de generar embeddings.

### `mirai agent <load|list>` — no implementado

Ambos subcomandos imprimen actualmente un aviso de `not yet implemented`. Existen como marcadores de posición para la futura gestión del registro de agentes; no desarrolles scripts basados en ellos.

---

## 5. Modo interactivo — ciclo de vida

Ejecutar `mirai` sin argumentos llama a `run_default`:

```
mirai
  └─ setup_wizard::run_setup_wizard(false, "", "", "")   # siempre interactivo
        └─ Some(SessionConfig)  → terminal::run_interactive_session(config)
        └─ None                 → "Setup cancelled." + exit 0
```

**No hay un flag para pre-cargar el asistente o saltárselo** desde `main.rs` — `mirai` por sí solo siempre abre el asistente. (`run_setup_wizard` tiene una ruta rápida no interactiva, pero actualmente solo es accesible programáticamente, no vía flags de CLI).

---

## 6. Comportamiento del asistente de configuración

`setup_wizard.rs` utiliza el modo crudo (raw mode) de `crossterm` para la selección con teclas de flecha (↑/↓ o `k`/`j` para moverse, Enter para elegir, Esc/`q` para cancelar). El flujo:

### Paso 1 — Local o Cloud

```
Donde correra el modelo?
  -> Local (Ollama — corre en tu maquina)
     Cloud (necesita API key)
```

### Paso 2a — Configuración de proveedor local (Ollama)

`setup_local_provider` ejecuta un bucle de auto-corrección:

1. **¿Instalado?** Verifica `which ollama`. Si falta, ofrece instalarlo (Homebrew en macOS, `curl … | sh` en Linux, enlace manual en otros lugares), reintentar, cambiar a Cloud o salir.
2. **¿En ejecución?** Llama a `GET {base_url}/api/tags`. Si es `not_running`, intenta ejecutar `ollama serve` en segundo plano y sondea el estado de salud por ~6s.
3. **¿Tiene modelos?** Si existen modelos, eliges uno (se muestra el nombre del modelo, localidad, modalidad y ventana de contexto). Si no hay ninguno, ofrece una lista curada para hacer `ollama pull` (`qwen3:8b`, `llama3.2`, `gemma4`, `deepseek-r1:8b`), y luego vuelve a detectar.

Las ventanas de contexto por modelo provienen de `POST /api/show` (`*.context_length`). El soporte de visión se infiere del nombre del modelo (`llava`, `gpt-4o`, `gemini`, `claude-3`, `pixtral`, …).

### Paso 2b — Configuración de proveedor en la nube (Cloud)

`setup_cloud_provider` detecta qué claves de entorno de proveedores están configuradas y te permite elegir entre las disponibles:

| Proveedor | Modelo por defecto | Clave de entorno |
|---|---|---|
| Groq (nivel gratuito) | `qwen-qwq-32b` | `GROQ_API_KEY` |
| NVIDIA NIM (nivel gratuito) | `meta/llama-3.3-70b-instruct` | `NVIDIA_API_KEY` |
| OpenAI (pago) | `gpt-4o` | `OPENAI_API_KEY` |
| OpenRouter (multi-modelo) | `meta-llama/llama-3.3-70b-instruct` | `OPENROUTER_API_KEY` |

Si no hay claves configuradas, muestra sugerencias de `export …` y ofrece reintentar / cambiar a Local / salir. Los modelos en la nube tienen por defecto una ventana de contexto de 128K.

### Paso 3 — Ventana de contexto

Ofrece el valor por defecto (la capacidad propia del modelo) más ajustes preestablecidos de 4K / 8K / 16K / 32K / 64K (sin duplicados).

### Paso 4 — Nivel de autonomía

| Nivel | `max_tool_rounds` | ¿Confirma escrituras? |
|---|---|---|
| Assisted | 1 | Sí |
| **Copilot** (defecto) | 25 | No |
| Autopilot | 50 | No |
| Self-Driving | 100 | No |

El asistente devuelve un `SessionConfig { provider, model, autonomy_level, max_tool_rounds, confirm_writes, context_window, supports_vision, temperature: 0.3, max_tokens: 4096 }`.

---

## 7. Terminal interactiva — el bucle del agente

`run_interactive_session` (`terminal.rs`) es el REPL del chat. Al inicio imprime el banner, proveedor/modelo/CWD, registra todas las herramientas integradas, construye los esquemas de las herramientas, crea una sesión y entra en el bucle de entrada.

### Exposición de herramientas

Las herramientas integradas se convierten en esquemas de funciones al estilo de OpenAI (`spec_to_openai_schema`); el nombre de la función es el `tool_type` con `/` reemplazado por `_` (ej. `filesystem/read_file` → `filesystem_read_file`), y un `name_map` lo revierte para su ejecución.

- **Excluidos en todas partes:** `trigger/webhook`, `trigger/manual`, `trigger/schedule`, `trigger/heartbeat`, `output/response` (estos son bloques exclusivos para grafos, sin sentido en un chat).
- **Solo núcleo para Ollama:** cuando `provider == "ollama"`, solo se expone un conjunto más pequeño de herramientas de "núcleo" (lectura/escritura/edición/glob/grep/lista/árbol/mkdir/eliminación del sistema de archivos, `system/bash`, y `git status/diff/log/commit`) para ajustarse a ventanas de contexto más pequeñas. Los proveedores de la nube obtienen el catálogo completo.

### Un turno

Para cada mensaje del usuario, `agentic_loop` ejecuta hasta `max_tool_rounds` rondas:

1. Convierte el historial de mensajes en objetos `Message` del motor.
2. Intenta **streaming** (`stream_with_messages`, los tokens se imprimen según llegan); ante cualquier error, recurre al modo sin streaming (`call_with_messages`). Se muestra un indicador de carga (spinner) mientras espera.
3. Rastrea el uso de tokens (`TokenTracker`).
4. Si la respuesta **no tiene llamadas a herramientas**, es la respuesta final — la devuelve.
5. De lo contrario, añade el mensaje del asistente (con `tool_calls`) y ejecuta cada llamada:
   - Resuelve el nombre de la función de vuelta a un `tool_type`.
   - **Confirmación:** en la autonomía `assisted`, las herramientas de tipo escritura (`filesystem/write_file`, `edit_file`, `move`, `copy`, `delete`, `mkdir`, `system/bash`, `git/commit`) preguntan `? Confirm …? [Y/n]`. Rechazar inyecta un resultado de herramienta `"User declined this action"`.
   - Ejecuta a través del registro contra un `DefaultExecutionContext::default_dev()` fresco. Los argumentos de ruta relativos `path`/`source`/`destination` se resuelven contra el CWD, y el `cwd` se inyecta en la configuración para las herramientas que lo aceptan.
   - Registra la llamada y el resultado en la transcripción de la sesión; imprime una línea ✓/✗ con un resumen del resultado y el tiempo transcurrido.
   - Los resultados de herramientas que superan los 30,000 caracteres se truncan antes de ser devueltos al modelo.

Si se alcanza `max_tool_rounds`, el turno finaliza con `(Max tool-call rounds reached. Please continue or rephrase.)`.

### Prompt de sistema

`build_system_prompt` inyecta el CWD, la plataforma, reglas estrictas de uso de herramientas, salvaguardas de seguridad (nunca `rm -rf /`, nunca tocar `.env`/secretos, nunca `git push`/`reset --hard`/`push --force`, detenerse tras 5 intentos fallidos) y una sección específica de autonomía (Assisted L1 → una herramienta/turno; Copilot L2 → multi-herramienta; Autopilot L3 → autónomo; Self-Driving L4 → perseguir objetivos).

### Comandos de barra diagonal (Slash commands)

Gestionados por `handle_slash` (entrada que comienza con `/`):

| Comando | Comportamiento |
|---|---|
| `/help` | Muestra la lista de comandos |
| `/quit`, `/exit`, `/q` | Guarda la sesión y sale |
| `/clear` | Reinicia la conversación, manteniendo solo el prompt de sistema |
| `/compact` | Fuerza la compactación del contexto hacia ~40K tokens |
| `/tokens` | Muestra el uso de tokens de la sesión y el tamaño estimado del contexto |
| `/tools` | Lista las herramientas expuestas agrupadas por prefijo de categoría |
| `/session` | Muestra el id de la sesión actual, conteo de mensajes/checkpoints |
| `/sessions` | Lista hasta 15 sesiones guardadas (●activa / ○cerrada), marcando la actual |
| `/checkpoint [etiqueta]` | Registra un punto de control (checkpoint) en el índice de mensaje actual |

Los comandos de barra desconocidos imprimen `Unknown command: <x>. Type /help`. Presionar Ctrl+C mientras el modelo está pensando interrumpe; EOF (Ctrl+D) o `/quit` cierra la sesión.

---

## 8. Persistencia de sesión y almacenamiento de transcripción

`session_storage.rs` persiste cada sesión interactiva bajo:

```
~/.datamirai/sessions/<session_id>/
  manifest.json      # metadatos
  transcript.jsonl   # registro de eventos de solo adición (append-only)
```

> El nombre del directorio `.datamirai` es una **nomenclatura heredada intencional**, mantenida por compatibilidad hacia atrás. Se utiliza `HOME` para localizarlo (recurre a `/tmp` si no está disponible).

**ID de sesión:** `ses_<segundos_unix>_<8-hex-aleatorios>` (la aleatoriedad es un xorshift simple con semilla de tiempo, sin dependencias extra). **ID de checkpoint:** `chk_<6-hex>`.

**`manifest.json`** contiene `id, provider, model, cwd, created_at, updated_at, message_count, checkpoint_count, status` (`active` → `closed` al salir). Cada entrada añadida actualiza `updated_at` y `message_count`.

**`transcript.jsonl`** — un objeto JSON por línea, cada uno con `ts`, `role`, y opcionalmente `content` + metadatos aplanados. Roles registrados:

| Rol | Cuándo | Notas |
|---|---|---|
| `user` | mensaje del usuario | |
| `assistant` | texto final del asistente | |
| `tool_call` | una invocación de herramienta | metadatos: `tool`, `args`, `round` |
| `tool_result` | resultado de una herramienta | contenido truncado a 5,000 caracteres; metadatos: `tool`, `round` |
| `checkpoint` | `/checkpoint` | metadatos: `checkpoint_id`, `label`, `message_index` |

Existen ayudantes para `read_transcript`, `read_manifest`, `list_sessions` (más recientes primero), `list_checkpoints` y `rebuild_messages` (filtra a `user`/`assistant`/`system`).

> **Sin comando de reanudación.** Las sesiones se escriben y se pueden listar (`/sessions`), y `rebuild_messages` puede reconstruir el historial, pero actualmente **no existe un comando de CLI para reabrir una sesión previa** — cada lanzamiento de `mirai` inicia una conversación fresca.

---

## 9. Comportamiento de compactación de mensajes

Para mantener la conversación dentro del contexto del modelo, `terminal.rs` estima los tokens como `content.len() / 4` (más la longitud de los argumentos de llamadas a herramientas) y compacta en dos pases (`compact_messages`):

1. **Truncar resultados de herramientas antiguos.** Para mensajes anteriores a los últimos 12, cualquier mensaje de tipo `tool` con más de 500 caracteres se corta a 200 caracteres + `... (compacted)`.
2. **Eliminar el medio.** Si todavía supera el presupuesto y hay más de 13 mensajes, mantiene el prompt de sistema, inserta un único marcador de posición `(Earlier conversation was compacted to save context. Continue from here.)` y mantiene los **últimos 12** mensajes.

Activadores:
- **Automático:** después de cada mensaje del usuario, con un presupuesto de **80,000** tokens.
- **Manual:** `/compact` ejecuta la misma lógica con un presupuesto más ajustado de **40,000** tokens e imprime `Compacted: ~<antes> → ~<después> tokens`.

`/clear` es el reinicio completo — elimina todo excepto el prompt de sistema.

---

## 10. Cómo se mapea la ejecución de CLI en la ejecución del motor

Los dos modos llegan al motor de formas muy diferentes:

| | `mirai run` (y `serve`) | Terminal interactiva |
|---|---|---|
| Fuente de especificación | `AgentSpec` desde un archivo YAML | Ninguna — chat de forma libre |
| Orquestación | `GraphRunner` + `RegistryExecutor` sobre un grafo validado | `agentic_loop` hecho a medida |
| Invocación de herramientas | El motor recorre los nodos; las herramientas se ejecutan dentro del grafo | El LLM emite llamadas a funciones; el CLI las ejecuta directamente vía `ToolRegistry` |
| Contexto de ejecución | Construido por ejecución con LLM resuelto + DB/almacenamiento en memoria | Un `DefaultExecutionContext::default_dev()` fresco **por llamada a herramienta** |
| Interfaz de LLM | `LLMResource` (vía `AdapterBridgeLLMResource`) | `LLMAdapter` directamente (`stream_with_messages` / `call_with_messages`) |
| Triggers / outputs | `trigger/*` y `output/response` son nodos reales del grafo | Esos tipos de herramientas están excluidos del esquema del chat |

En resumen: **`run`/`serve` son ejecución de grafos; el modo interactivo es un bucle de llamada a herramientas alrededor de un adaptador puro.** Las implementaciones de herramientas integradas se comparten entre ambas rutas a través de `register_all_builtin_tools` y el `ToolRegistry`.

---

## 11. Modos comunes de falla y resolución de problemas

| Síntoma | Causa probable | Solución |
|---|---|---|
| `Error: live agents must be started with \`mirai play\`` | La especificación es `agent_type: Live`, y `mirai run` la rechaza — **pero aún no existe el subcomando `play`** | Cambia el agente a un tipo no-live, o ejecútalo a través de `mirai serve` / la API HTTP |
| `Error: Cannot connect to provider. Is it running?` (interactivo) | Ollama (o el proveedor configurado) no es alcanzable | Inicia Ollama (`ollama serve`); verifica `OLLAMA_BASE_URL` |
| `Error: Model timed out…` (interactivo) | Modelo demasiado grande / contexto demasiado extenso | Elige un modelo más pequeño o una ventana de contexto menor en el asistente |
| `Warning: Provider 'claude' requires API key…` | Falta la clave para un proveedor en la nube | Configura `--api-key` o la variable de entorno del proveedor (`ANTHROPIC_API_KEY`, etc.) |
| `input validation failed: missing required input: …` | A `--input` le falta un campo que la especificación declara como obligatorio | Añade el campo al JSON de `--input`, o verifica `mirai describe <archivo>` |
| `Graph validation failed: …` | Grafo mal formado (aristas incorrectas, nodos faltantes) | Ejecuta `mirai validate <archivo>` y corrige el error reportado |
| `mirai run` imprime ruido que no es JSON mezclado con JSON | Las líneas de proveedor/alma/traza van a **stderr** | Redirige: `mirai run a.yaml 2>/dev/null` para capturar JSON limpio en stdout |
| Las flechas del asistente no funcionan | Sin TTY (tubería/terminal no interactiva) | Ejecuta `mirai` en una terminal real; el asistente necesita entrada en modo crudo |
| Los datos/almacenamiento no persistieron entre llamadas a `mirai run` | `run` usa DB/almacenamiento en memoria | Usa `mirai serve` para un contexto de mayor duración |
| `rag search` no devuelve nada / errores en embeddings | El proveedor no soporta embeddings o no tiene el modelo | Usa un proveedor/modelo que soporte embeddings |
| `mirai agent load/list` no hace nada útil | Ambos son stubs no implementados | No dependas de ellos todavía |

---

## 12. Ejemplos para desarrollo local

```bash
# 0. Construir
cargo build --release
alias mirai=./target/release/mirai

# 1. Validar antes de ejecutar
mirai validate examples/hello-world.yaml

# 2. Ejecutar localmente contra Ollama (proveedor por defecto)
mirai run examples/hello-world.yaml --input '{"query": "¿Qué es Rust?"}'

# 3. Ejecución determinista con el proveedor mock (sin LLM, ideal para pruebas)
mirai run examples/hello-world.yaml --provider mock

# 4. Inspeccionar el contrato de una herramienta antes de conectarla a un grafo
mirai tools system/bash

# 5. Crear un nuevo agente a partir de una plantilla y ejecutarlo
mirai templates
mirai new --template <id> --name scratch-agent --provider ollama
mirai run scratch-agent.yaml

# 6. Ver la traza de ejecución y métricas
mirai run examples/hello-world.yaml --trace

# 7. Realizar benchmark de una ejecución (escribe benchmarks.jsonl)
MIRAI_BENCHMARK=1 mirai run examples/hello-world.yaml

# 8. Iniciar el servidor de desarrollo (sin auth) y consultarlo
mirai serve --port 3000
#   → ver docs/backend/API.md para los contratos de petición/respuesta

# 9. Abrir la terminal de programación interactiva
mirai
#   elige Local → Ollama → un modelo → ventana de contexto → autonomía
#   luego chatea; usa /help, /tools, /tokens, /compact, /quit
```

---

*Fuente de verdad: `cli/src/*.rs` en v0.7.0. Si el CLI cambia, actualiza este documento y las tablas de comandos anteriores para que coincidan con el código.*
