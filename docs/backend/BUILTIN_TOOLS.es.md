# BUILTIN_TOOLS.md

Referencia para implementadores de la librería de herramientas integradas de OpenMirai (`engine/src/tools/builtin/`).

Este documento describe los **contratos de implementación, límites y expectativas de seguridad** de las herramientas integradas. Está escrito para colaboradores que extienden la librería de herramientas, auditan su comportamiento o la conectan a un nuevo host.

> **División de audiencia.** [`USAGE.md`](../../USAGE.md) es el catálogo de herramientas orientado al usuario: qué hace cada herramienta y cómo llamarla desde un YAML de agente. Este documento es la vista del implementador: los traits que una herramienta debe satisfacer, los recursos que puede tocar, los modos de fallo que debe producir y los límites que no debe cruzar. Para las definiciones subyacentes de traits/patrones, consulte [`PRIMITIVES.md`](./PRIMITIVES.md).

Versión del motor al momento de escribir este documento: **0.7.0**.

---

## 1. Descripción general del registro

Todas las herramientas integradas residen bajo `engine/src/tools/builtin/` y se conectan a un [`ToolRegistry`](../../engine/src/tools/registry.rs) mediante `register_all_builtin_tools()` en `engine/src/tools/builtin/mod.rs`:

```rust
pub fn register_all_builtin_tools(registry: &mut ToolRegistry) {
    logic::register_logic_tools(registry);
    ai::register_ai_tools(registry);
    data::register_data_tools(registry);
    filesystem::register_filesystem_tools(registry);
    system::register_system_tools(registry);
    git::register_git_tools(registry);
    output::register_output_tools(registry);
    agent::register_agent_tools(registry);
    mcp::register_mcp_tools(registry);
    trigger::register_trigger_tools(registry);
    state::register_state_tools(registry);
}
```

Cada familia expone una función `register_*_tools()`. Hay **52 herramientas registradas** a través de 11 familias (el conteo se afirma en `builtin::tests::register_all_builtin_tools_adds_all`):

| Familia | Módulo | Conteo | Prefijo `tool_type` |
|---------|--------|-------:|---------------------|
| Logic | `logic.rs` | 7 | `logic/` |
| AI | `ai.rs` | 6 | `ai/` |
| Data | `data/` | 11 | `data/` |
| Filesystem | `filesystem/` | 12 | `filesystem/` |
| System | `system.rs` | 3 | `system/` |
| Git | `git.rs` | 4 | `git/` |
| Output | `output.rs` | 1 | `output/` |
| Agent | `agent.rs` | 1 | `agent/` |
| MCP | `mcp.rs` | 1 | `mcp/` |
| Trigger | `trigger.rs` | 5 | `trigger/` |
| State | `state.rs` | 1 | `state/` |

### Nomenclatura de tipos de herramientas

Un `tool_type` es una cadena calificada por categoría, `"<category>/<name>"` (por ejemplo, `"ai/llm_call"`, `"filesystem/read_file"`). El prefijo de categoría coincide con el campo `category` en el [`ToolSpec`](#3-toolspec-and-schema-conventions) y es utilizado por el editor y `list_tools()` para la agrupación.

### Aliases (compatibilidad hacia atrás)

El registro admite aliases legados que se resuelven a un `tool_type` canónico:

```rust
registry.register_alias("fs/read_file", "filesystem/read_file");
registry.register_alias("mcp/mcp_call", "mcp/call");
```

`ToolRegistry::get()` recurre al mapa de aliases en caso de no encontrar una coincidencia directa. Los aliases **no** son devueltos por `list_tools()`. Los aliases actuales (`fs/*` → `filesystem/*`, `mcp/mcp_call` → `mcp/call`) están marcados como *eliminar en v0.3.0* en el código fuente y no se debe confiar en ellos para nuevos trabajos. Pruebas: `filesystem::tests::legacy_fs_aliases_resolve`, `mcp::tests::legacy_mcp_alias_resolves`.

---

## 2. Contrato de implementación de herramientas

Tres traits en `engine/src/tools/registry.rs` definen el contrato. Consulte [`PRIMITIVES.md` → Tool + ToolFactory Trait Pattern](./PRIMITIVES.md).

### `Tool` — la instancia ejecutable

```rust
#[async_trait]
pub trait Tool: Send + Sync {
    async fn execute(
        &self,
        inputs: HashMap<String, Value>,
        config: &HashMap<String, Value>,
        context: &dyn ExecutionContext,
    ) -> Result<HashMap<String, Value>, ToolError>;
}
```

- **`inputs`** — valores en tiempo de ejecución resueltos desde el `data_map` del nodo (aristas de nodos ascendentes). Propiedad de la llamada; la herramienta puede consumirlos.
- **`config`** — configuración estática a nivel de nodo desde el bloque `config:` del YAML del agente. Prestado.
- **`context`** — el [`ExecutionContext`](#4-executioncontext-requirements); el único canal legítimo hacia los recursos de DB/LLM/almacenamiento/autenticación.
- **Retorno** — un `HashMap<String, Value>` cuyas claves DEBERÍAN coincidir con los `outputs` declarados en el `ToolSpec` de la herramienta. Estos pasan a estar disponibles para los nodos descendentes.

Una instancia de `Tool` es **sin estado y económica**: todos los componentes integrados son structs de unidad (`pub struct BashTool;`). El estado por llamada vive en variables locales; el estado entre llamadas debe pasar a través de un recurso en el `ExecutionContext`.

### `ToolFactory` — el dueño de la especificación y constructor

```rust
pub trait ToolFactory: Send + Sync {
    fn create(&self) -> Arc<dyn Tool>;
    fn spec(&self) -> &ToolSpec;
}
```

Cada `tool_type` tiene exactamente una fábrica. La fábrica posee el `ToolSpec` y genera un nuevo `Arc<dyn Tool>` por ejecución de nodo a través de `create()`.

### La macro de código repetitivo

Cada familia define una macro declarativa local (`ai_tool!`, `logic_tool!`, `fs_tool!`, `system_tool!`, `git_tool!`, `data_tool!`, `output_tool!`, `agent_tool!`, `mcp_tool!`, `trigger_tool!`) que genera el struct de unidad, el struct de la fábrica, `new()`/`Default`, y la implementación de `ToolFactory` a partir de un literal de especificación. Usted solo escribe el bloque `#[async_trait] impl Tool`. `state/memory` es la única herramienta escrita manualmente (sin macro) porque documenta la inyección de `__memory_spec` en detalle.

### Cómo el ejecutor maneja una herramienta

`RegistryExecutor::execute()` (en `registry.rs`) es el puente entre el ejecutor de grafos y una herramienta. Para cada nodo:

1. Busca la fábrica por `node.tool_type`; si falta → `ToolError::NotFound`.
2. **Valida las entradas** contra `ToolSpec.inputs` a través de `validate_node_inputs()` (PRD-004 Capa 2). Si faltan campos obligatorios o hay desajustes de tipo → `ToolError::ExecutionFailed`. Los campos opcionales con un `default` declarado se inyectan aquí.
3. Llama a `factory.create()` para obtener una nueva instancia.
4. Ejecuta `execute()` dentro de `catch_unwind`: **un pánico de la herramienta se convierte en un `ToolError::ExecutionFailed` ("tool panic: …"), nunca en un cierre inesperado del proceso.**

Implicaciones para los implementadores:
- Puede confiar en que las entradas obligatorias estén presentes y correctamente tipadas *si* las declaró en la especificación. Las entradas que lea pero no haya declarado **no** se validan: léalas defensivamente.
- Un pánico es capturado, pero es un bug, no una estrategia de manejo de errores. Devuelva `Err(ToolError::ExecutionFailed { … })` para fallos esperados.

---

## 3. ToolSpec y convenciones de esquema <a id="3-toolspec-and-schema-conventions"></a>

`ToolSpec` y `ToolField` se definen en `engine/src/tools/base.rs`.

```rust
pub struct ToolSpec {
    pub tool_type: String,   // "ai/llm_call"
    pub name: String,        // "LLM Call" (human label)
    pub description: String,
    pub version: String,     // "1.0.0" for all built-ins
    pub category: String,    // "ai"
    pub inputs: Vec<ToolField>,
    pub outputs: Vec<ToolField>,
    pub config_fields: Vec<ToolField>,
}

pub struct ToolField {
    pub name: String,
    pub field_type: FieldType,
    pub required: bool,
    pub description: Option<String>,
    pub default: Option<Value>,
}
```

Construya los campos con el constructor canónico `field()` — nunca cree manualmente un `ToolField`:

```rust
field("prompt", FieldType::String, /* required */ false, "Instruction for the LLM")
```

### FieldType

`FieldType` es un enum cerrado (sin cadenas de tipo de forma libre): `String`, `Number`, `Boolean`, `Array`, `Object`, `Integer`, `File`. `FieldType::matches(&Value)` respalda la validación de entradas; `FieldType::File` coincide con un objeto de referencia a archivo (`crate::llm::media::is_file_ref` — `{ "_type": "file_ref", "path": … }`, PRD-010).

### Convenciones observadas en la librería

- **`inputs` vs `config`.** Las entradas son valores del flujo de datos por ejecución; la configuración es el ajuste estático del nodo. Muchas herramientas resuelven un valor **primero de la entrada, luego de la configuración como respaldo** (por ejemplo, el prompt de `ai/llm_call`, el `agent_id` de `agent/run_agent`, la consulta/documentos de `data/rag_search`). Documente esto en la descripción del campo.
- **Opcional + predeterminado.** Declare campos opcionales con `required = false`. Si establece `ToolField.default`, el ejecutor lo inyecta en caso de ausencia, por lo que su `execute()` también puede confiar en él. La mayoría de las herramientas integradas aplican valores predeterminados en línea con `.unwrap_or(...)`, lo cual está bien pero significa que el valor predeterminado es invisible para el editor; prefiera los valores predeterminados declarados para valores orientados al editor.
- **Las claves de salida deben coincidir con las salidas declaradas.** El cableado del `data_map` descendente hace referencia a los nombres de salida. Manténgalos sincronizados con la especificación.
- **Las claves con prefijo `__` son portadores internos**, no forman parte del esquema público. Son inyectadas por el host/ejecutor o pasadas entre fases y nunca se declaran en la especificación. Ejemplos: `__memory_spec`, `__agent_id`, `__mcp_servers`, `__agent_call_chain__`, `__memory_write`, `__user_media`. Trátelas como un canal lateral privado.
- **Version** es `"1.0.0"` para cada herramienta integrada hoy; auméntela si realiza un cambio de esquema que rompa la compatibilidad en una sola herramienta.

`ToolSpec` y `ToolField` son `Serialize`/`Deserialize`; las pruebas de ida y vuelta residen en `base.rs::tests`.

---

## 4. Requisitos del ExecutionContext <a id="4-executioncontext-requirements"></a>

`ExecutionContext` (`engine/src/core/context.rs`) es la **única** vía sancionada para que una herramienta acceda a recursos compartidos. Las herramientas reciben `&dyn ExecutionContext`.

```rust
pub trait ExecutionContext: Send + Sync {
    fn db(&self) -> Option<&dyn DBResource>;        // optional
    fn llm(&self) -> &dyn LLMResource;              // always present
    fn storage(&self) -> Option<&dyn StorageResource>;
    fn vector(&self) -> Option<&dyn VectorResource>;
    fn auth(&self) -> &AuthContext;
    fn session_id(&self) -> &str;
    fn node_id(&self) -> Option<&str>;
    fn system_prompt(&self) -> Option<&str>;
    fn scratch_dir(&self) -> Option<&str> { None }  // PRD-010
}
```

Reglas para las herramientas:

- **`llm()` es el único recurso siempre disponible.** `db()`, `storage()`, `vector()` devuelven `Option`; una herramienta que necesite uno DEBE manejar `None` con un `ToolError::ExecutionFailed` claro, no con `.unwrap()`. Consulte `data::tests::db_read_no_db_fails`, `storage_read_no_storage_fails`.
- **Los recursos devuelven `ResourceError`**, que usted mapea a `ToolError::ExecutionFailed`. `ResourceError` transporta `PermissionDenied`, `NotFound`, `Database`, `Llm`, `Storage`, `Other`.
- **`auth()`** expone `AuthContext { user_id, role, universe_id, environment_id }` con `Role ∈ {Owner, Admin, Editor, Viewer}`. El RBAC se aplica en la capa de recurso/host (consulte las pruebas de autenticación de la Comunidad 27), no se vuelve a implementar por herramienta; lea `auth()` solo cuando una herramienta necesite delimitar por usuario/universo.
- **`scratch_dir()`** es un directorio temporal por ejecución (PRD-010), que se limpia después de la ejecución. `system/bash` lo expone a subprocesos como `MIRAI_SCRATCH_DIR` y busca en él los `output_files` declarados. Úselo para archivos transitorios: nunca escriba estado duradero allí.
- **`system_prompt()`** es el prompt del sistema a nivel de agente; `ai/llm_call` lo concatena antes de la configuración `system_prompt` a nivel de nodo.

Para las pruebas, `InMemoryContext` (`#[cfg(test)]`, en `context.rs`) proporciona un stub de LLM y no proporciona DB/almacenamiento/vector. Las pruebas de herramientas de datos utilizan un `TestContext` más rico en `data/mod.rs` con stubs de `DBResource`/`StorageResource`.

---

## 5. Convenciones de manejo de errores

```rust
pub enum ToolError {        // engine/src/core/runner/types.rs
    NotFound { tool_type: String },
    ExecutionFailed { tool_type: String, message: String },
}
```

- Las herramientas devuelven **solo** `ExecutionFailed` desde `execute()`; `NotFound` le corresponde al registro lanzarlo. Establezca siempre `tool_type` en su propio `tool_type` para que el ejecutor pueda atribuir el fallo al nodo correcto.
- **Validar y luego actuar.** Si faltan entradas obligatorias → `ExecutionFailed` con un mensaje que nombre el campo (`"input 'text' is required"`). Esto refleja la pre-validación del propio ejecutor y cubre las entradas no declaradas que usted lee directamente.
- **Distinguir "recurso ausente" de "operación fallida".** Una DB faltante es un error de configuración (`"no database resource configured"`); una consulta fallida es un error de ejecución (mapeado desde `ResourceError`).
- **Fallo suave (soft-fail) vs fallo duro (hard-fail).** Algunas herramientas devuelven un objeto de resultado *exitoso* con un campo `success: false` / `error: "…"` en lugar de un `Err`: se usa cuando el fallo es un resultado normal e inspeccionable sobre el cual un grafo puede ramificarse. `mcp/call` hace esto para "no servers configured" / "server not found". Reserve este patrón para resultados sobre los cuales se espera que el grafo tome rutas; de lo contrario, devuelva `Err`.
- Nunca use `panic!`/`unwrap()` sobre una entrada externa. La red de `catch_unwind` existe para bugs, no para el flujo de control.

---

## 6. Expectativas de seguridad y sandbox por familia

El riesgo aumenta bruscamente desde logic/output (puro) → data (mediado) → filesystem → system/git (acceso directo al host). El límite de mediación es importante: consulte [§13 No evada estos límites](#13-do-not-bypass-these-boundaries).

| Familia | Efectos secundarios | ¿Mediado por `ExecutionContext`? | Notas |
|---------|---------------------|----------------------------------|-------|
| `logic/*` | Ninguno (puro / sleep) | n/a | Determinista, seguro. |
| `output/*` | Ninguno | n/a | Solo formateo. |
| `state/*` | Buffers de escrituras de memoria | Indirecto (el host persiste) | Respeta las claves declaradas + modo de persistencia. |
| `ai/*` | Llamadas de red LLM + lecturas de archivos multimedia + subproceso (claude_code) | `llm()` para `llm_call`/`embeddings`/`transcribe`; **no** para `claude_code` | Escáner de inyección de prompts en `llm_call`. |
| `data/*` | DB / almacenamiento / vector / **HTTP saliente** | Sí (db/storage/vector); `web_scrape` realiza HTTP directo | Acceso delimitado por el recurso inyectado. |
| `agent/*` | Ejecución de sub-agente | Impulsado por el host (marcador de posición en la herramienta) | Protecciones de profundidad + ciclos. |
| `mcp/*` | Llamadas a servidores MCP externos | Vía `MCPManager` desde la configuración inyectada | Red/proceso por transporte. |
| `trigger/*` | Ninguno al momento de la ejecución | n/a | Nodos de metadatos de punto de entrada. |
| `filesystem/*` | **Lectura/escritura/eliminación directa en el FS del host** | **No** — `std::fs` nativo | Sin cárcel de rutas. Ver §13. |
| `system/*` | **Ejecución de shell / procesos / código** | No | Lista de bloqueo de `bash`; aislamiento de directorio temporal en `sandbox_exec`. |

Regla principal: **las herramientas de sistema de archivos y sistema tocan el host directamente y NO están aisladas por el `ExecutionContext`.** Solo `system/sandbox_exec` proporciona un aislamiento real, y solo a nivel de proceso (§9).

---

## 7. Herramientas de IA: comportamiento y dependencias de recursos LLM

`engine/src/tools/builtin/ai.rs` registra cuatro herramientas.

| `tool_type` | Dependencia de recurso | Notas |
|-------------|------------------------|-------|
| `ai/llm_call` | `context.llm()` | Nodo LLM principal. |
| `ai/embeddings` | `context.llm().embed()` | Devuelve vector + dimensiones. |
| `ai/transcribe` | `context.llm()` + lectura de archivo multimedia | Audio multimodal. |
| `ai/claude_code` | **Subproceso CLI local de `claude`** | Sin LLM del `ExecutionContext`; utiliza la suscripción Claude Code del host. |

### `ai/llm_call`

La herramienta más densa en características. Comportamiento:

- Resuelve `prompt` desde la entrada, luego desde la configuración; si está vacío → error.
- **Escaneo de inyección de prompts (activado por defecto).** Antes de llamar al LLM, recopila recursivamente cada cadena en `inputs`, las une y ejecuta `crate::security::scan`. En caso de bloqueo, devuelve `ExecutionFailed` con el tipo de amenaza y la confianza. Alterne a través de la configuración `security_scan` (bool), `security_sensitivity` (`low|medium|high`), `security_block` (bool). Consulte las pruebas de detección de inyección de la Comunidad 32 (`security`). **No desactive el escáner en rutas que alimenten contenido no confiable o recuperado por herramientas en los prompts.**
- Construye un bloque `=== Session Context ===` a partir de todas las entradas que no sean `prompt`, truncadas proporcionalmente a `max_context_length` caracteres (predeterminado 12000, mínimo por clave 200). Pruebas: `format_session_context_*`.
- **Salida estructurada.** Si se establece `output_schema` (cadena JSON u objeto), enriquece el prompt con una instrucción de JSON obligatorio, valida la respuesta (campos obligatorios, tipos, enums) y reintenta hasta `max_retries` (predeterminado 2) con retroalimentación de errores. `output_schema_strict` (predeterminado **true**) hace que un fallo de validación final sea un error duro; el modo no estricto devuelve el mejor esfuerzo con `schema_valid: false`. Pruebas: `validate_response_*`, `extract_json_*`, `build_retry_prompt_*`.
- **Multimodal.** `media_path` (entrada o configuración; ruta de cadena o referencia a archivo) se lee a través de `crate::llm::media::read_media_file` para el proveedor y se adjunta al turno del usuario.
- Salidas: `response`, `model`, `tokens_input`, `tokens_output`, `structured_output`, `schema_valid`.

### `ai/embeddings` / `ai/transcribe`

`embeddings` llama a `llm().embed(text, model)`; `transcribe` lee un archivo de audio, lo envía como contenido multimodal con un prompt de sistema de transcripción literal a temperatura 0. Ambos mapean `ResourceError` → `ExecutionFailed`.

### `ai/claude_code`

Genera el subproceso CLI de `claude -p` instalado localmente (detectado automáticamente en `$PATH` o vía `cli_path`), envía el prompt ensamblado a stdin, impone `timeout_ms` (predeterminado 60s) y devuelve stdout. **Esto evade por completo la abstracción LLM del `ExecutionContext`**: es un subproceso del host que utiliza la suscripción Claude del usuario, no una clave de API. Trátelo con la misma precaución que `system/bash`: ejecución local arbitraria bajo el usuario del motor. Los conteos de tokens son estimaciones aproximadas de longitud/4.

---

## 8. Herramientas de datos: comportamiento y dependencias de almacenamiento

`engine/src/tools/builtin/data/` registra 11 herramientas; cada una reside en su propio archivo y comparte la macro `data_tool!` de `data/mod.rs`.

| `tool_type` | Depende de | Comportamiento |
|-------------|------------|----------------|
| `data/db_read` | `context.db()` | Lee filas; `mode = one\|all`; devuelve `rows`/`row` + `count`. |
| `data/db_write` | `context.db()` | Inserción/upsert; creación automática de tablas; analiza datos JSON en cadena. |
| `data/entity_query` | `context.db()` | Consulta entidades por tipo + filtros. |
| `data/entity_upsert` | `context.db()` | Crea/actualiza entidad; devuelve `id` + `action`. |
| `data/storage_read` | `context.storage()` | Lee blob; `mode = content\|presign`; flag `found`, `NotFound` → suave. |
| `data/storage_write` | `context.storage()` | Escribe blob; devuelve `bytes_written`. |
| `data/vault_read` | (marcador de posición) | Lectura de bóveda de conocimiento (knowledge-vault); devuelve vacío hasta que se conecte un backend de FS. |
| `data/vault_write` | (marcador de posición) | Confirma la escritura; devuelve la ruta + `written`. |
| `data/rag_search` | `context.llm().embed()` | Fragmento → embeber → coseno top-K. Embeddings reales. |
| `data/html_to_markdown` | ninguno | Transformación pura; **elimina `<script>`**. Pruebas: `html_to_markdown_strips_script`. |
| `data/web_scrape` | **HTTP directo** | Búsqueda (Google/Bing/DuckDuckGo) o recuperación de URL. |

Límites:

- El acceso a DB/almacenamiento/vector está **siempre mediado** por el recurso que el host inyectó en el `ExecutionContext`. Una herramienta no puede acceder a una base de datos que el host no proporcionó; recurso faltante → error. La implementación del recurso (no la herramienta) impone la tenencia/RBAC y la seguridad SQL.
- `data/web_scrape` es la excepción: realiza **HTTP saliente directamente** (no a través de un recurso del contexto). Aplica una `SessionFingerprint` por sesión (cabeceras de sigilo deterministas), normalización de URL (elimina parámetros de seguimiento), jitter de solicitud y una caché de respuesta TTL. Pruebas: `fingerprint_*`, `url_normalization_strips_tracking_params`, `serp_extraction_*`, `jitter_*`, `cache_*`. Debido a que recupera contenido remoto arbitrario, su salida **no es confiable**: cualquier elemento descendente que lo alimente a un LLM debe mantener activado el escáner de inyección de `ai/llm_call`.
- `vault_read`/`vault_write` son marcadores de posición hoy (sin persistencia real); documente la brecha si construye sobre ellos.

---

## 9. Herramientas de sistema de archivos y sistema: riesgo operativo

### Sistema de archivos (`engine/src/tools/builtin/filesystem/`, 12 herramientas)

`filesystem/read_file`, `write_file`, `edit_file`, `glob_files`, `grep_files`, `list_dir`, `tree`, `copy`, `move`, `delete`, `mkdir`, `file_info`.

- Estas utilizan **`std::fs` nativo** en el sistema de archivos del host. **No hay cárcel de rutas (path jail) ni mediación del `ExecutionContext`**: operan en cualquier lugar donde el usuario del SO del motor pueda llegar. `read_file`/`delete` llaman a `fs::canonicalize` (que resuelve enlaces simbólicos y rechaza rutas inexistentes) pero **no** confinan el resultado a ninguna raíz.
- `filesystem/delete` es **recursivo para directorios** (`fs::remove_dir_all`). `filesystem/write_file` crea los directorios padres faltantes. `filesystem/edit_file` rechaza reemplazos ambiguos a menos que se establezca `replace_all` (`edit_file_ambiguous_without_replace_all`).
- El confinamiento de rutas, si es necesario, es responsabilidad del **host** (ejecutar el motor como un usuario con privilegios mínimos, contenedorizar o chroot). Ver §13.

### Sistema (`engine/src/tools/builtin/system.rs`, 3 herramientas)

| `tool_type` | Aislamiento | Riesgo |
|-------------|-------------|--------|
| `system/bash` | Solo lista de bloqueo | Ejecuta `sh -c <command>` en el host. |
| `system/process_list` | ninguno | Ejecuta `ps aux`. |
| `system/sandbox_exec` | Dir temporal + timeout + ulimit (mejor esfuerzo) | Ejecución de código de menor riesgo. |

- **`system/bash`** ejecuta el comando a través de `sh -c` con privilegios completos del host. Bloquea un pequeño conjunto de patrones catastróficos (`rm -rf /`, bomba fork, `mkfs`, `dd of=/dev/…`, `> /dev/sd*`, `curl|sh`, `wget|sh` — consulte `blocked_patterns()`), impone un tiempo de espera (predeterminado 120s, máx. 600s), valida el `cwd`, trunca la salida a 30 000 caracteres, inyecta las entradas que no son `command` como variables de entorno `MIRAI_<key>` y expone `MIRAI_SCRATCH_DIR`. **La lista de bloqueo es una barandilla contra accidentes, no un límite de seguridad**: es trivialmente evadible y no debe tratarse como un sandbox. Pruebas: `bash_blocks_*`, `bash_timeout`, `bash_with_cwd`.
- **`system/sandbox_exec`** es la única herramienta que ofrece un aislamiento real, y solo a *nivel de proceso* (`engine/src/sandbox.rs`, Fase 1): un directorio de trabajo temporal nuevo, finalización por tiempo de espera, un entorno restringido y límites de memoria de mejor esfuerzo a través de `ulimit` en los SO compatibles. `network_access` tiene como valor predeterminado **false**. Admite `python`/`javascript`/`bash`. **No** es un sandbox de kernel (sin namespaces/seccomp): no confíe en él para contener código hostil.

---

## 10. Herramientas de estado y salida: efecto en el estado de ejecución

### `state/memory` (`state.rs`)

Persiste las claves declaradas a través de ciclos/ejecuciones de agentes (PRD-008). Mecánica:

- El host inyecta `__memory_spec` (`{ persist, keys }`) y `__agent_id` en `config` antes de la ejecución (realizado por `run_agent_spec`).
- `persist: none` → **no-op**, devuelve `persisted_keys` vacío.
- De lo contrario, filtra `inputs` a **solo las claves declaradas en `graph.memory`** (las claves no declaradas se descartan con una advertencia), devuelve la lista de claves persistidas y almacena el mapa filtrado bajo la clave de salida interna `__memory_write`.
- **La herramienta no persiste nada por sí misma.** Prepara los datos; el host (servidor/planificador) lee `__memory_write` del estado después de la ejecución y lo escribe en el almacén de memoria. Pruebas: `persist_none_is_noop`, `persist_cycle_filters_declared_keys`, `persist_execution_writes_data`.

Este filtro de claves declaradas es un límite deliberado: un agente no puede persistir un estado arbitrario, solo aquello por lo que optó en su especificación.

### `output/response` (`output.rs`)

Nodo terminal que formatea el resultado final. `format ∈ {text, json, markdown (predeterminado), bullets}`, con sustitución de `template` `${key}` y `title` opcionales. `extract_content` busca la cadena significativa en claves comunes (`result`, `output`, `text`, `content`, `response`, `summary`). Formateo puro, sin efectos secundarios. Pruebas: `response_json_format`, `response_bullets_format`, `response_template`.

---

## 11. Herramientas de Agente, MCP y disparadores: límites de composición

### `agent/run_agent` (`agent.rs`)

Ejecuta otro agente como una subtarea síncrona. La herramienta en sí es un **marcador de posición que impone las barandillas de composición**; la ejecución real del hijo ocurre en la capa del host/aplicación, que lee `_input_data` y `_call_chain` de la salida.

- `agent_id` se resuelve de entrada → configuración; obligatorio.
- **Límite de anidamiento:** `MAX_NESTING_DEPTH = 3`. Lee `__agent_call_chain__` de las entradas; exceder el límite → error.
- **Protección contra ciclos:** un `agent_id` que ya esté en la cadena → error "Circular agent reference".
- Emite la cadena extendida para que el host pueda recurrir de forma segura. Pruebas: `run_agent_nesting_depth_exceeded`, `run_agent_circular_reference`, `run_agent_missing_id_fails`.

Cuando conecte el ejecutor real de sub-agentes, **preserve estos dos invariantes**: son la única protección contra bucles/recursividad para la composición de agentes.

### `mcp/call` (`mcp.rs`)

Invoca una herramienta en un servidor externo del Protocolo de Contexto de Modelo (MCP).

- Lee `server_name`/`tool_name` de la configuración y `arguments` de la entrada/configuración.
- El host inyecta los servidores disponibles como `__mcp_servers` (`Vec<AgentMcpServerSpec>`). Si no hay servidores configurados, o el nombre del servidor no está en la especificación → un **fallo suave** (`success: false`, `error` descriptivo), no un `Err`.
- Construye un `MCPManager`, llama a la herramienta, extrae el contenido de texto del array `content` de MCP para mayor comodidad, y **siempre cierra todas las conexiones (`close_all()`)** (éxito o fallo). El transporte es por servidor (`stdio`/HTTP). Pruebas: `mcp_call_without_servers_config_returns_descriptive_error`, `mcp_call_with_nonexistent_server_returns_not_found`.

MCP es el límite de extensión sancionado: permite a los agentes llamar a servidores de herramientas externos sin añadir código Rust al motor. La exposición de red/proceso se hereda del transporte configurado.

### `trigger/*` (`trigger.rs`, 5 herramientas)

`trigger/webhook`, `trigger/manual`, `trigger/schedule`, `trigger/event`, `trigger/heartbeat`. Estos son **puntos de entrada al grafo**: transportan metadatos de configuración del disparador y producen una carga útil inicial; no realizan efectos secundarios en el host al momento de `execute()`. La programación/gestión de eventos real reside en el host (servidor/planificador).

---

## 12. Cómo añadir una nueva herramienta integrada

1. **Elija una familia** y abra su módulo (`engine/src/tools/builtin/<family>.rs`, o un nuevo archivo bajo `data/` o `filesystem/`).
2. **Declare la herramienta** con la macro de la familia: proporcione `tool_type`, `name`, `description` e `inputs`/`outputs`/`config_fields` construidos con `field()`. Mantenga `tool_type` como `"<category>/<name>"` coincidiendo con la categoría de la familia.
3. **Implemente `#[async_trait] impl Tool`.** Valide las entradas obligatorias/no declaradas por adelantado, acceda a los recursos solo a través de `context`, mapee `ResourceError` → `ToolError::ExecutionFailed { tool_type: "<su tipo>", … }` y devuelva un `HashMap` cuyas claves coincidan con sus `outputs` declarados.
4. **Regístrela** en el `register_*_tools()` de la familia y aumente la prueba de afirmación de conteo de esa familia. Si renombra una herramienta existente, añada un `register_alias(old, new)` por una versión.
5. **Para una nueva familia**, añada `pub mod <family>;` y una llamada a `register_*_tools()` en `builtin/mod.rs`, luego actualice el total en `register_all_builtin_tools_adds_all` (actualmente 52).
6. **Respete los límites.** Si la herramienta toca el sistema de archivos, el shell o la red, vuelva a leer el §13 primero y prefiera el enrutamiento a través de un recurso de `ExecutionContext` (`db`/`storage`/`vector`) o `system/sandbox_exec` en lugar del acceso directo al host.
7. **Actualice `USAGE.md`** (el catálogo orientado al usuario) y, si añade un patrón reutilizable, `PRIMITIVES.md`.

---

## 13. No evada estos límites <a id="13-do-not-bypass-these-boundaries"></a>

Estas reglas se aplican específicamente a las familias **filesystem, system y git**, que actúan directamente sobre el host y no están mediadas por el `ExecutionContext`.

1. **No trate la lista de bloqueo de `system/bash` como un límite de seguridad.** Captura un puñado de patrones catastróficos y nada más. Es una barandilla contra accidentes. El aislamiento real = ejecutar el motor como un usuario con privilegios mínimos, en un contenedor/VM, con el sistema de archivos y la red bloqueados a nivel del SO. Nunca amplíe la lista de bloqueo y la llame "reforzada".

2. **No añada una herramienta de sistema de archivos que confine las rutas en el proceso y asuma que es segura.** Las herramientas existentes **no tienen cárcel de rutas**; `canonicalize` resuelve enlaces simbólicos pero no confina a una raíz, y `delete` es recursivo. Si un despliegue necesita confinamiento de rutas, este DEBE provenir del entorno del host (chroot/contenedor/usuario dedicado), no de comprobaciones de cadenas en una herramienta. No construya una media cárcel que invite a la confianza.

3. **No enrute el trabajo privilegiado fuera de `ExecutionContext`.** El acceso a DB, almacenamiento y vector existe precisamente para que el host pueda delimitar la tenencia, el RBAC y la seguridad SQL en la capa de recurso. Una herramienta que abre su propia conexión a la DB o su propio cliente S3 anula eso. Las únicas excepciones autorizadas de red directa hoy son `data/web_scrape`, `ai/claude_code` y `mcp/call`, cada una delimitada y documentada anteriormente; no añada más sin revisión.

4. **No ejecute código no confiable fuera de `system/sandbox_exec`.** Si necesita ejecutar código generado por el modelo o el usuario, use `sandbox_exec` (directorio temporal, tiempo de espera, `network_access: false`), nunca `system/bash` o `ai/claude_code`, que se ejecutan con los privilegios completos del motor.

5. **No permita que `git/commit` enmiende, fuerce o salte los hooks.** `git/commit` está limitado intencionalmente: prepara archivos nombrados, rechaza un mensaje vacío, rechaza cuando no hay nada preparado y ejecuta un simple `git commit -m` (los comentarios de la fuente mencionan esto: *NUNCA enmendar, NUNCA saltar hooks*). Otras herramientas de git (`status`, `diff`, `log`) son de solo lectura. Mantenga las herramientas de git incapaces de reescribir el historial o de mutación remota (sin `push`, `reset --hard`, `commit --amend`, `--no-verify`). Añadir estas convertiría una familia de bajo riesgo en una destructiva.

6. **No desactive el escáner de inyección de `ai/llm_call` en entradas no confiables.** El contenido de `data/web_scrape`, `mcp/call` o cualquier herramienta que ingiera texto externo puede transportar cargas útiles de inyección de prompts. El escáner está activado por defecto por una razón; déjelo activado donde dicho contenido pueda llegar a un prompt.

7. **No evada las protecciones de profundidad y ciclos de `agent/run_agent`** cuando implemente la recursividad del lado del host. Son la única protección contra bucles de sub-agentes ilimitados.

---

## 14. Expectativas de pruebas

- **Coloque las pruebas unitarias** en el `#[cfg(test)] mod tests` de cada módulo de herramienta. Siga la nomenclatura existente (`<tool>_<scenario>`, por ejemplo, `bash_blocks_fork_bomb`).
- **Use los contextos de prueba.** `InMemoryContext::new("test-run")` para herramientas que solo necesitan el stub de LLM; el `TestContext` en `data/mod.rs` (stub de `DBResource`/`StorageResource`, filas configurables) para herramientas de datos. Las pruebas de sistema de archivos utilizan `tempfile::TempDir`.
- **Cubra, como mínimo:**
  - el camino feliz (happy path) con entradas declaradas;
  - cada fallo por falta de entrada obligatoria / recurso faltante (`*_no_db_fails`, `*_no_storage_fails`, `*_missing_*_fails`);
  - cada protección de seguridad que añada (el conjunto `bash_blocks_*` es el modelo);
  - cualquier ayudante puro directamente (por ejemplo, `evaluate_condition`, `validate_response`, `normalize_url`, `convert_html_to_md`): estos son testeables unitariamente sin un contexto.
- **Prueba de registro por familia.** Cada `register_*_tools()` tiene una prueba que afirma que las herramientas registradas se resuelven y el conteo de `list_tools()` es exacto (`register_*_tools_adds_*`). Actualícela cuando añada o elimine una herramienta, y mantenga sincronizado el agregado `register_all_builtin_tools_adds_all` (52).
- **Los aliases** obtienen su propia prueba de resolución (`legacy_fs_aliases_resolve`, `legacy_mcp_alias_resolves`).

Ejecute la suite desde `engine/`:

```bash
cargo test -p openmirai-engine tools::builtin
```

---

## Ver también

- [`USAGE.md`](../../USAGE.md) — catálogo de herramientas orientado al usuario y patrones YAML de agentes.
- [`PRIMITIVES.md`](./PRIMITIVES.md) — patrones `Tool`/`ToolFactory`/`ToolRegistry` y otras primitivas reutilizables del motor.
- [`API.md`](./API.md) — API HTTP que expone la ejecución del grafo.
- `engine/src/tools/registry.rs` — registro, ejecutor, validación, red de pánico.
- `engine/src/core/context.rs` — `ExecutionContext` y traits de recursos.
- `engine/src/core/runner/types.rs` — `ToolError` y tipos del ejecutor.
