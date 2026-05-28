# PRD-006 — Apple-Grade Refinement: Craft, Coherencia y Elegancia

| Campo | Valor |
|-------|-------|
| **ID** | PRD-006 |
| **Fecha** | 2026-05-27 |
| **Estado** | in_progress |
| **Target** | v0.4.0 |
| **Principio** | El código es una extensión del alma de quien lo escribe |

---

## Contexto

Este PRD nace de un code review profundo de las 37,327 líneas de Rust + 363 líneas de Python del motor datamirai-engine v0.3.1. Se evaluó desde 6 perspectivas: arquitectura, experiencia del consumidor, código limpio, DDD, producción, y finalmente — **la perspectiva Apple**: craft, coherencia, elegancia, atención al detalle.

El motor funciona. Los 636+ tests pasan. El core es sólido. Lo que falta es la **última milla de artesanía** — los detalles que separan "funciona bien" de "esto lo hizo alguien que ama lo que hace".

Este PRD se organiza en **3 fases** con **20 secciones** independientes. Cada sección es un trabajo autocontenido que puede abordarse sin depender de las demás (salvo dependencias explícitas).

---

## Diagrama General

```
PRD-006: Apple-Grade Refinement
│
├── FASE 1: "Ship it right" ─── Antes de publicar en GitHub
│   ├── S01: README como packaging
│   ├── S02: Examples como "aha moment"
│   ├── S03: Eliminar código muerto
│   ├── S04: Auth básico en servidor
│   └── S05: Graceful shutdown
│
├── FASE 2: "Make it elegant" ─── v0.4.0
│   ├── S06: Partir runner.rs (God File #1)
│   ├── S07: Partir app.rs (God File #2)
│   ├── S08: Partir data.rs y filesystem.rs (God Files #3 y #4)
│   ├── S09: Unificar sistema de tipos
│   ├── S10: Extraer código duplicado
│   ├── S11: Eliminar magic strings
│   ├── S12: Naming cleanup
│   ├── S13: Session eviction + request timeout
│   └── S14: Defaults consistentes
│
└── FASE 3: "Think different" ─── v0.5.0
    ├── S15: Renombrar resources → adapters
    ├── S16: Feature flags en Cargo.toml
    ├── S17: Consolidar doble abstracción LLM
    ├── S18: Python SDK mejorado
    ├── S19: API versioning
    └── S20: run_from() → arquitectura de 8 líneas
```

---

# FASE 1: "Ship it right"

Esta fase se completa **antes** de que el repo sea público. Son los cambios que definen la primera impresión. En Apple, el packaging del producto es tan importante como el producto mismo.

---

## S01: README como packaging

### Qué es hoy

El README.md actual tiene estos problemas concretos:

1. La primera línea dice `Motor open source de ejecucion de grafos agentivos` — sin acento en "ejecución", en español (barrera para comunidad internacional), y sin gancho emocional.

2. La sección "Estructura" referencia dos directorios que **no existen** en el repo:
   ```
   rust/          Motor Rust (crate principal + CLI)    ← NO EXISTE
   legacy/        Paquete Python (referencia historica)  ← NO EXISTE
   ```
   Estos directorios están en `.gitignore`. Un developer que clone el repo y lea el README verá una mentira en los primeros 15 segundos.

3. El Quick Start dice `cd rust && cargo build --release` — el directorio `rust/` no existe. El comando correcto sería `cargo build --release` desde la raíz.

4. El conteo de tests dice "553" pero ahora son 636+.

5. No hay ningún ejemplo funcional que se pueda copiar-pegar y ejecutar en 30 segundos.

6. El idioma es español. Para un proyecto open source con audiencia global, esto limita adopción.

### Qué tiene que ser

Un README que funcione como el packaging de un producto Apple. La persona que llega al repo debe sentir:
- **En 5 segundos**: qué es esto y por qué me importa
- **En 30 segundos**: puedo hacerlo funcionar
- **En 2 minutos**: entiendo la arquitectura

### Lo que hay que hacer

1. **Reescribir en inglés** como idioma primario. Puede haber una sección en español al final o un link a docs en español, pero el README principal va en inglés.

2. **Primera línea** — debe ser un headline que inspire, no una descripción técnica. Algo en la línea de:
   > Open-source agent execution engine. One binary. Any LLM. Your rules.

   Seguido de una línea que posicione: qué problema resuelve y por qué es diferente (single binary Rust, 48 tools, 7 LLM providers, YAML-first).

3. **Quick Start funcional** — un bloque de 4-5 comandos que realmente funcionen:
   - Instalar/compilar
   - Crear un agente YAML mínimo (inline o referencia a examples/)
   - Ejecutar el agente
   - Ver el resultado

   Cada comando debe ser copy-pasteable y funcionar. Probar antes de commitear.

4. **Estructura de directorios** — solo los directorios que realmente existen:
   ```
   engine/        Core library (datamirai-engine crate)
   cli/           CLI binary (mirai)
   sdks/python/   Python SDK
   examples/      Ready-to-run agent examples
   docs/          Technical documentation
   ```

5. **Tabla de tools** — la actual está bien, mantenerla pero con conteo correcto.

6. **Diagrama de arquitectura** — el de 3 capas (Agent = JSON, Motor = Rust binary, Host = any language) es excelente. Mantenerlo, asegurarse de que los números sean correctos.

7. **Badges** — version, license, tests passing. Son señales de profesionalismo.

### Criterio de aceptación

- [ ] Un developer que no habla español puede entender el proyecto en 30 segundos
- [ ] Cada comando en el Quick Start funciona al copy-paste
- [ ] No hay referencias a directorios o archivos que no existan
- [ ] El conteo de tests/tools es correcto

---

## S02: Examples como "aha moment"

### Qué es hoy

No existe un directorio `examples/`. El único ejemplo de agente está en el README (un JSON de 8 líneas) y no se puede ejecutar directamente porque no incluye instrucciones de cómo correrlo.

### Por qué importa

En Apple, la primera experiencia con un producto es diseñada obsesivamente. El "Turn on your new iPhone → Hello" es un momento mágico. Para un framework, ese momento es: "cloné el repo, corrí un ejemplo, y vi un agente funcionando". Sin ese momento, la mayoría de developers cierra la pestaña.

### Lo que hay que hacer

Crear un directorio `examples/` en la raíz del proyecto con al menos estos 3 archivos YAML:

**1. `examples/hello-world.yaml`** — El más simple posible. Un solo nodo LLM que responde una pregunta. Debe demostrar que el motor funciona en su forma más básica. Incluir un comment header que explique qué hace y cómo ejecutarlo:
```yaml
# Hello World — the simplest possible agent
# Run: mirai run examples/hello-world.yaml
```
Estructura: trigger/manual → ai/llm_call → output/response. Tres nodos, dos edges. Un developer lo lee y entiende el concepto en 10 segundos.

**2. `examples/conditional-routing.yaml`** — Demuestra branching condicional. Un agente que clasifica un input y toma caminos diferentes según el resultado. Muestra el poder de las edge conditions. Estructura: trigger → llm (clasifica) → condition → rama A / rama B → output. Incluir comments que expliquen cada edge condition.

**3. `examples/data-pipeline.yaml`** — Un caso de uso real: agente que busca en un vault, pasa el contexto a un LLM, y guarda el resultado. Demuestra data_map entre nodos, múltiples tool types, y el flujo de datos. Más complejo, pero realista.

Opcionalmente:
- **`examples/multi-agent.yaml`** — Universe con 2 souls y routing
- **`examples/python-quickstart.py`** — Uso del SDK en 15 líneas

Cada archivo YAML debe:
- Ser autocontenido (no depender de archivos externos)
- Tener comments que expliquen qué hace cada nodo y edge
- Incluir en la primera línea el comando para ejecutarlo
- Funcionar con Ollama (provider local, sin API key) como default
- Funcionar también con cualquier otro provider via flags

### Criterio de aceptación

- [ ] `mirai run examples/hello-world.yaml` funciona out of the box con Ollama
- [ ] Cada example tiene un header comment con el comando de ejecución
- [ ] Los examples cubren: básico, condicional, y pipeline de datos
- [ ] Un developer lee hello-world.yaml y entiende qué es un agente en 10 segundos

---

## S03: Eliminar código muerto

### Qué es hoy

Hay 3 piezas de código que existen en el repo pero **no hacen nada funcional**:

**1. `engine/src/voice.rs` (149 líneas)**
Define tipos para STT/TTS: `STTProvider`, `TTSProvider`, `VoiceConfig`, `VoiceState`, `TranscriptionResult`, `SynthesisResult`. Ninguno tiene implementación real. No hay ningún otro archivo en todo el codebase que importe o use estos tipos. Son tipos fantasma.

**2. `engine/src/channels.rs` (181 líneas)**
Define tipos para channel adapters: `ChannelType` (WhatsApp, Telegram, Slack, Discord), `ChannelMessage`, `ChannelResponse`, `ChannelAdapter` trait. Solo `WebhookAdapter` tiene implementación, pero tampoco se usa desde ningún otro archivo. Los channels de WhatsApp, Telegram, Slack, Discord son solo variantes de un enum — no hay código que se conecte a estas plataformas.

**3. `AgentRuntime.execute()` en `engine/src/runtime/agent_runtime.rs`**
El método público `execute()` contiene esta línea:
```rust
tracing::info!("Simulated execution — caller should wire GraphRunner directly");
```
Un método público llamado `execute` que no ejecuta nada. Imprime un log diciendo que es simulado. Es confuso para cualquier developer que intente usarlo.

### Por qué importa

Steve Jobs: *"People think focus means saying yes to the thing you've got to focus on. It means saying no to the hundred other good ideas."*

Código muerto en un repo open source comunica: "esto no está terminado" o "esto no se revisa". Cada archivo que existe debe **ganarse su lugar** en el codebase. Si no funciona, no debería estar.

Esto NO significa que estas features no se van a implementar jamás. Significa que cuando se implementen, se van a implementar bien, con su PRD, sus tests, y su integración real. Mientras tanto, no deben estar ahí como promesas vacías.

### Lo que hay que hacer

**Para `voice.rs`:**
1. Eliminar el archivo `engine/src/voice.rs` completo
2. Eliminar la línea `pub mod voice;` de `engine/src/lib.rs`
3. Verificar que ningún otro archivo importe de `voice` (no debería, pero confirmar con grep)
4. Correr `cargo build` para confirmar que compila
5. Correr `cargo test` para confirmar que los tests pasan

**Para `channels.rs`:**
1. Eliminar el archivo `engine/src/channels.rs` completo
2. Eliminar la línea `pub mod channels;` de `engine/src/lib.rs`
3. Verificar que ningún otro archivo importe de `channels`
4. Si algún archivo importa `ChannelType` o similar, evaluar si se puede eliminar esa referencia también
5. Correr `cargo build` + `cargo test`

**Para `AgentRuntime.execute()`:**
1. Abrir `engine/src/runtime/agent_runtime.rs`
2. Eliminar el método `execute()` completo (el que dice "Simulated execution")
3. Si hay tests que llaman a este método, eliminar esos tests también
4. Verificar que nada en el codebase llame a `agent_runtime.execute()` — si algo lo llama, evaluar si se debe remover esa llamada o si el caller necesita usar `GraphRunner.run()` directamente
5. Correr `cargo build` + `cargo test`

**NO hacer:**
- No crear stubs o placeholders tipo `// TODO: implement voice`
- No mover los archivos a una carpeta "experimental" — o existe y funciona, o no existe
- No dejar re-exports huérfanos en `lib.rs`

### Criterio de aceptación

- [ ] `voice.rs` eliminado, `pub mod voice` eliminado de lib.rs
- [ ] `channels.rs` eliminado, `pub mod channels` eliminado de lib.rs
- [ ] `AgentRuntime.execute()` eliminado
- [ ] `cargo build` compila sin warnings
- [ ] `cargo test` pasa todos los tests existentes (los que no dependían del código eliminado)
- [ ] `grep -r "voice\|channels\|ChannelType\|VoiceConfig" engine/src/` no encuentra imports huérfanos

---

## S04: Auth básico en servidor

### Qué es hoy

El servidor HTTP (`engine/src/server/app.rs`) tiene:
```rust
.layer(CorsLayer::permissive())
```

Y cero middleware de autenticación. Cualquiera que tenga acceso a la IP y puerto del servidor puede:
- Crear y eliminar agentes
- Ejecutar agentes (consumiendo el API key del LLM configurado)
- Leer todas las sesiones y sus resultados
- Eliminar grafos

Para desarrollo local esto es aceptable. Para un deployment en un VPS accesible por internet, esto es un problema de seguridad real.

### Lo que hay que hacer

Implementar un middleware de autenticación **simple** basado en API key. No OAuth, no JWT, no sesiones — un simple header `X-API-Key` que se compara contra un valor configurado.

**Comportamiento:**

1. Al iniciar el servidor, se lee la API key de una de estas fuentes (en orden de prioridad):
   - Variable de entorno `MIRAI_API_KEY`
   - Flag de CLI `--api-key <key>`

2. Si no se proporciona API key, el servidor arranca **sin auth** (modo desarrollo). Pero imprime un warning visible:
   ```
   ⚠ WARNING: No API key configured. Server is running without authentication.
   ⚠ Set MIRAI_API_KEY or use --api-key to enable authentication.
   ```

3. Si se proporciona API key, **cada request** (excepto `GET /health` y `GET /version`) debe incluir el header `X-API-Key` con el valor correcto. Si no lo incluye o el valor es incorrecto, responder con:
   ```json
   {"error": "unauthorized"}
   ```
   HTTP status 401.

4. Los endpoints `GET /health` y `GET /version` son públicos siempre (para health checks de load balancers, monitoreo, etc.)

**Dónde implementar:**

- Crear un middleware de axum (usando `axum::middleware::from_fn_with_state`) que intercepte todas las requests
- El middleware lee `X-API-Key` del header, lo compara con el valor en `AppState`
- Si no hay API key configurada en AppState (modo dev), el middleware permite todo
- Si hay API key pero la request no la trae o es incorrecta → 401

**Cambios necesarios:**

1. Agregar `api_key: Option<String>` a `AppState`
2. Crear la función middleware de auth
3. Registrar el middleware en `create_router()` — después del CORS layer
4. En `serve()`, aceptar un parámetro `api_key: Option<String>`
5. En el CLI (`main.rs`), pasar el API key desde env/flag al llamar `serve()`

**Sobre CORS:**

`CorsLayer::permissive()` es demasiado permisivo para producción. Pero cambiarlo ahora podría romper integraciones (la app Local, por ejemplo). Dejarlo permissive por ahora pero documentar que en producción debería restringirse a origins específicos.

### Criterio de aceptación

- [ ] Sin `MIRAI_API_KEY`, el servidor funciona sin auth (backward compat)
- [ ] Sin `MIRAI_API_KEY`, se muestra warning visible al arrancar
- [ ] Con `MIRAI_API_KEY`, requests sin header reciben 401
- [ ] Con `MIRAI_API_KEY`, requests con header correcto pasan normalmente
- [ ] `GET /health` y `GET /version` son siempre públicos
- [ ] Tests existentes siguen pasando (ejecutan sin API key)

---

## S05: Graceful shutdown

### Qué es hoy

```rust
// server/app.rs
pub async fn serve(...) -> Result<(), Box<dyn std::error::Error>> {
    // ...setup...
    axum::serve(listener, app).await?;
    Ok(())
}
```

El servidor no maneja signals del sistema operativo. Cuando haces `systemctl stop` o `kill <pid>` en el VPS, el proceso recibe SIGTERM y muere inmediatamente. Si hay agentes ejecutándose en ese momento, sus ejecuciones se cortan a la mitad sin cleanup.

### Por qué importa

"It just works" — incluyendo cuando lo apagas. Un producto Apple que crashea al apagarse no es Apple-grade.

### Lo que hay que hacer

Implementar manejo de SIGTERM y SIGINT (Ctrl+C) en la función `serve()`.

**Comportamiento deseado:**

1. Al recibir SIGTERM o SIGINT, el servidor:
   - Deja de aceptar nuevas conexiones
   - Imprime un mensaje: `Shutting down gracefully...`
   - Espera hasta 30 segundos para que las requests en vuelo terminen
   - Si después de 30 segundos aún hay requests pendientes, fuerza el cierre
   - Imprime: `Server stopped.`

2. Axum ya tiene soporte para graceful shutdown via `axum::serve(...).with_graceful_shutdown(signal)`. Solo necesitamos crear el future de signal.

**Implementación:**

1. Crear una función async `shutdown_signal()` que espere a SIGTERM o SIGINT usando `tokio::signal`:
   ```
   - En Unix: tokio::signal::unix::signal(SignalKind::terminate())
   - En todos: tokio::signal::ctrl_c()
   ```

2. Pasar esta función a `axum::serve(listener, app).with_graceful_shutdown(shutdown_signal())`

3. Imprimir los mensajes de shutdown en la función signal.

**Consideraciones:**
- La función debe ser cross-platform (macOS y Linux al menos)
- No bloquear el event loop — todo async
- El timeout de 30 segundos para graceful shutdown es manejado por axum internamente

### Criterio de aceptación

- [ ] Ctrl+C en la terminal detiene el servidor sin errores
- [ ] `kill <pid>` (SIGTERM) detiene el servidor limpiamente
- [ ] Se imprime mensaje de shutdown
- [ ] Requests en vuelo tienen tiempo de terminar (no se cortan instantáneamente)
- [ ] Funciona en macOS y Linux

---

# FASE 2: "Make it elegant"

Esta fase transforma el código de "funciona" a "elegante". Es donde la artesanía se hace visible para cualquier developer que abra los archivos.

---

## S06: Partir runner.rs (God File #1)

### Qué es hoy

`engine/src/core/runner.rs` tiene **3,290 líneas** en un solo archivo. Contiene:

- Líneas 1-11: Imports
- Líneas 22-57: Error types (`RunnerError`, `ToolError`)
- Líneas 63-166: `HookHandler` trait + `HookResult` enum (hook system completo)
- Líneas 172-187: `CheckpointCallback` trait + `Checkpoint` struct
- Líneas 193-200: `InterruptInfo` struct
- Líneas 207-216: `TranscriptEntry` struct
- Líneas 222-275: `RetryPolicy` + `BackoffStrategy` + `FailureMode`
- Líneas 281-331: `ExecutionResult` + `TraceEntry` + `ExecutionStatus` + `TraceStatus`
- Líneas 337-350: `ToolExecutor` trait
- Líneas 356-1611: `GraphRunner` struct completo (1,255 líneas del struct + todos sus métodos)
- Líneas 1617-1652: Free helper functions
- Líneas 1658-3290: Tests (~1,632 líneas de tests)

Esto es como meter toda una casa en una sola habitación.

### Por qué importa

Cuando un developer abre `runner.rs` buscando entender cómo funciona el motor, se enfrenta a 3,290 líneas de scroll. No sabe dónde buscar. La estructura no le ayuda a navegar.

Apple diseña sus SDKs con módulos granulares. SwiftUI tiene `View.swift`, `Modifier.swift`, `Layout.swift` — no un `SwiftUI.swift` de 100,000 líneas.

### Lo que hay que hacer

Convertir `engine/src/core/runner.rs` en un **directorio** `engine/src/core/runner/` con estos archivos:

**1. `engine/src/core/runner/mod.rs`** (~30 líneas)
- Solo `pub mod` declarations y `pub use` re-exports
- Todos los tipos públicos se re-exportan aquí para que la API pública no cambie
- Cualquier código que hoy haga `use crate::core::runner::GraphRunner` sigue funcionando igual

**2. `engine/src/core/runner/types.rs`** (~200 líneas)
- `RunnerError` enum
- `ToolError` enum
- `HookResult` enum
- `BackoffStrategy` enum
- `FailureMode` enum
- `ExecutionStatus` enum
- `TraceStatus` enum
- `RetryPolicy` struct
- `TraceEntry` struct
- `TranscriptEntry` struct
- `Checkpoint` struct
- `InterruptInfo` struct
- `ExecutionResult` struct
- Los `Default` impls de cada uno
- El `Display` impl de `TraceStatus`

**3. `engine/src/core/runner/traits.rs`** (~80 líneas)
- `HookHandler` trait con sus 7 métodos default
- `CheckpointCallback` trait
- `ToolExecutor` trait

**4. `engine/src/core/runner/graph_runner.rs`** (~200 líneas)
- El struct `GraphRunner` con sus campos
- El constructor `new()` y los métodos builder (`with_event_emitter`, `with_hook_handler`, etc.)
- Los métodos públicos `run()` y `resume()`
- El método `request_pause()` y `pause_flag()`
- El método privado `run_from()` — pero simplificado (ve S20 para el refactor completo, aquí solo se parte el archivo)

**5. `engine/src/core/runner/execution.rs`** (~300 líneas)
- Los métodos extraídos de `run_from()`:
  - `resolve_inputs()` — resolución de inputs desde edges + data_map
  - `resolve_expression()` — parsing de templates `${node.field}`
  - `resolve_next_node()` — evaluación de edges condicionales
  - `resolve_all_next_nodes()` — detección de fan-out
  - `evaluate_condition()` — comparación de valores
- Las funciones helper: `compare_numbers()`, `as_f64()`

**6. `engine/src/core/runner/retry.rs`** (~80 líneas)
- `execute_with_retry()` — el loop de retry con backoff
- `calculate_backoff()` — cálculo de delay
- `retry_policy_for()` — lookup de policy desde node config

**7. `engine/src/core/runner/fanout.rs`** (~130 líneas)
- `execute_fanout()` — ejecución paralela con join_all
- `find_fanout_join()` — detección del nodo de convergencia

**8. `engine/src/core/runner/helpers.rs`** (~30 líneas)
- `now_ts()` — timestamp helper
- `run_hook_with_timeout()` — wrapper de timeout para hooks
- `emit_event()` y `stream_event()` helpers del GraphRunner

### Regla fundamental

**La API pública no cambia.** Todo lo que hoy se importa con `use crate::core::runner::GraphRunner` o `use datamirai_engine::GraphRunner` sigue funcionando exactamente igual. El re-export en `mod.rs` garantiza esto.

Los tests se mueven a archivos `_test` dentro del directorio runner/ o se dejan en un archivo `tests.rs` dentro del módulo. Los test helpers (`EchoExecutor`, `FailExecutor`, `FailNExecutor`, `TestContext`, `StubLLM`) van en un `test_support.rs` compartido.

### Criterio de aceptación

- [ ] El directorio `core/runner/` contiene los 8 archivos descritos
- [ ] `cargo build` compila sin cambios en la API pública
- [ ] `cargo test` pasa todos los tests existentes
- [ ] Ningún archivo en el nuevo directorio supera las 350 líneas
- [ ] Los imports externos (`use datamirai_engine::GraphRunner`) siguen funcionando

---

## S07: Partir app.rs (God File #2)

### Qué es hoy

`engine/src/server/app.rs` tiene **1,502 líneas** con:
- AppState + LLMFactory (líneas 1-63)
- Request/Response models (líneas 69-107)
- Router factory + serve() (líneas 113-182)
- Health/Version handlers (líneas 188-210)
- Graph CRUD handlers (~90 líneas)
- Agent CRUD + execution handlers (~300 líneas)
- Streaming handler (~80 líneas)
- Session handlers (~50 líneas)
- Universe + GroupChat handlers (~200 líneas)
- Webhook handler (~30 líneas)
- Tool list handler (~20 líneas)
- Template handler (~20 líneas)
- RAG search handler (~60 líneas)
- Eval handler (~80 líneas)
- Metrics handler (~30 líneas)
- Helper functions (~100 líneas)
- Tests (~300 líneas)

### Lo que hay que hacer

Convertir en un directorio `engine/src/server/` con:

**1. `mod.rs`** — Re-exports + `create_router()` + `serve()`
**2. `state.rs`** — `AppState`, `LLMFactory`, request/response types compartidos
**3. `handlers/mod.rs`** — Re-exports de handlers
**4. `handlers/health.rs`** — `health()`, `version()`, `get_metrics()`
**5. `handlers/graphs.rs`** — CRUD de grafos
**6. `handlers/agents.rs`** — CRUD de agentes + execute + stream + schema
**7. `handlers/sessions.rs`** — List + get sessions
**8. `handlers/universe.rs`** — Universe message + groupchat
**9. `handlers/tools.rs`** — List tools + templates
**10. `handlers/webhooks.rs`** — Webhook handler
**11. `handlers/eval.rs`** — Eval + RAG search
**12. `helpers.rs`** — `run_agent_spec()` y otras funciones compartidas entre handlers

### Criterio de aceptación

- [ ] Mismos criterios que S06 — API pública intacta, todos los tests pasan
- [ ] Ningún handler file supera 200 líneas
- [ ] `create_router()` lee como un índice: cada línea = una ruta

---

## S08: Partir data.rs y filesystem.rs (God Files #3 y #4)

### Qué es hoy

- `tools/builtin/data.rs`: **2,400 líneas**, 11 tools
- `tools/builtin/filesystem.rs`: **1,798 líneas**, 12 tools

### Lo que hay que hacer

Cada tool debe vivir en su propio archivo dentro de un subdirectorio:

```
tools/builtin/
  data/
    mod.rs              → re-exports + registro
    db_read.rs          → DbReadTool
    db_write.rs         → DbWriteTool
    db_query.rs         → DbQueryTool
    storage_read.rs     → StorageReadTool
    storage_write.rs    → StorageWriteTool
    storage_delete.rs   → StorageDeleteTool
    vault_read.rs       → VaultReadTool
    vault_write.rs      → VaultWriteTool
    entity_store.rs     → EntityStoreTool
    web_scrape.rs       → WebScrapeTool + HtmlToMarkdownTool
    rag_search.rs       → RagSearchTool
  filesystem/
    mod.rs              → re-exports + registro
    read_file.rs        → ReadFileTool
    write_file.rs       → WriteFileTool
    edit_file.rs        → EditFileTool
    glob.rs             → GlobTool
    grep.rs             → GrepTool
    list_dir.rs         → ListDirTool
    tree.rs             → TreeTool
    copy_file.rs        → CopyFileTool
    move_file.rs        → MoveFileTool
    delete_file.rs      → DeleteFileTool
    mkdir.rs            → MkdirTool
    file_info.rs        → FileInfoTool
```

### Por qué 1 tool = 1 file

1. Un contributor que quiere agregar un tool nuevo ve exactamente el patrón a seguir
2. Un developer que busca cómo funciona `db_read` abre un archivo de ~100 líneas, no uno de 2,400
3. Code review de un cambio en `web_scrape` no requiere revisar 2,400 líneas de diff
4. Cada tool es independiente — cambiar uno no toca los demás

### El `mod.rs` de cada directorio

Solo contiene:
1. `pub mod` declarations para cada tool
2. Una función `register_data_tools(registry: &mut ToolRegistry)` que registra todos los tools de esa categoría
3. El `register_all_builtin_tools()` existente llama a `register_data_tools()`, `register_filesystem_tools()`, etc.

### Criterio de aceptación

- [ ] Cada tool en su propio archivo
- [ ] Ningún archivo supera 300 líneas
- [ ] `register_all_builtin_tools()` sigue funcionando igual
- [ ] Todos los tests pasan
- [ ] Los tool_type strings no cambian (e.g., "data/db_read" sigue siendo "data/db_read")

---

## S09: Unificar sistema de tipos

### Qué es hoy

Hay dos enums que representan el mismo concepto — "el tipo de un valor":

**`InputType`** en `core/agent_spec.rs`:
```
Text, Number, Boolean, Json, File
```
Usado por: validación de inputs del agente (nivel spec YAML)

**`FieldType`** en `tools/base.rs`:
```
String, Number, Boolean, Array, Object, Integer
```
Usado por: validación de inputs de herramientas (nivel tool spec)

Problemas:
- `InputType::Text` y `FieldType::String` son lo mismo
- `InputType::Json` mapea a `FieldType::Array` O `FieldType::Object`
- `FieldType::Integer` no tiene equivalente en `InputType`
- `InputType::File` no tiene equivalente en `FieldType`
- Hay dos funciones de validación separadas: `validate_agent_inputs()` y `validate_node_inputs()`
- Hay dos funciones helper separadas: `value_type_name()` y `value_type_label()`

### Lo que hay que hacer

Crear un **único** enum `ValueType` en un archivo central y usarlo en ambos contextos.

**1. Crear `engine/src/core/value_type.rs`** con:
```
ValueType {
    Text,       // Strings (antes InputType::Text y FieldType::String)
    Number,     // Floats and integers (antes InputType::Number y FieldType::Number)
    Integer,    // Integers only (antes FieldType::Integer, nuevo para InputType)
    Boolean,    // Booleans
    Array,      // JSON arrays (antes parte de InputType::Json y FieldType::Array)
    Object,     // JSON objects (antes parte de InputType::Json y FieldType::Object)
    File,       // File paths as strings (antes InputType::File)
}
```

**2. Un solo método `matches(&self, value: &Value) -> bool`**

**3. Un solo `fn value_type_label(v: &Value) -> &'static str`**

**4. Migrar `InputType` → `ValueType`** en agent_spec.rs:
- `InputType::Text` → `ValueType::Text`
- `InputType::Number` → `ValueType::Number`
- `InputType::Boolean` → `ValueType::Boolean`
- `InputType::Json` → se reemplaza por `ValueType::Object` o `ValueType::Array` según el caso. Si el spec dice `type: json`, mapearlo a "acepta Object o Array" internamente.
- `InputType::File` → `ValueType::File`

**5. Migrar `FieldType` → `ValueType`** en tools/base.rs:
- Reemplazar el enum y actualizar todas las tool specs que lo usan

**6. Eliminar `validate_agent_inputs()` y `validate_node_inputs()`** como funciones separadas. Crear una sola función de validación genérica que ambos contextos usen.

### Backward compatibility YAML

Los YAML de agentes que hoy dicen `type: text` deben seguir funcionando. El serde deserializer de `ValueType` debe aceptar tanto `text` como `string` para el tipo texto (alias).

### Criterio de aceptación

- [ ] Un solo enum `ValueType` en todo el codebase
- [ ] `InputType` y `FieldType` eliminados
- [ ] Una sola función de validación
- [ ] Una sola función `value_type_label()`
- [ ] YAML existentes siguen parseando correctamente
- [ ] Todos los tests pasan

---

## S10: Extraer código duplicado

### Las 5 duplicaciones concretas

**1. `auto_generate_edge_ids()`**
- Hoy: implementada idénticamente en `GraphDef` (graph.rs:121) y `AgentSpec` (agent_spec.rs:458)
- Además, `AgentSpec::validate()` llama su propia versión Y luego llama `GraphDef::validate()` que podría llamar la suya
- Solución: eliminar de `AgentSpec`, que `AgentSpec::validate()` solo llame a `graph.auto_generate_edge_ids()` a través de `to_graph()`

**2. `map_reqwest_error()`**
- Hoy: función idéntica en claude.rs, gemini.rs, ollama.rs, openai_compat.rs
- Solución: crear `engine/src/llm/error.rs` con una sola función `pub fn map_reqwest_error(e: reqwest::Error) -> LLMError`

**3. Test helpers `make_node()` + `make_edge()`**
- Hoy: definidos en graph.rs tests, runner.rs tests, agent_spec.rs tests
- Solución: crear un módulo `#[cfg(test)] pub mod test_fixtures` en `engine/src/core/` que exporte estos helpers. Los 3 módulos de test importan de ahí.

**4. `StubLLM` test struct**
- Hoy: definida en runner.rs tests y context.rs tests
- Solución: una sola definición en el módulo test_fixtures mencionado arriba

**5. `value_type_name()` / `value_type_label()`**
- Resuelto por S09 (unificación de tipos)

### Criterio de aceptación

- [ ] Cada pieza de lógica existe en exactamente un lugar
- [ ] Los test helpers se comparten via módulo `test_fixtures`
- [ ] `grep -r "fn map_reqwest_error" engine/src/` retorna exactamente 1 resultado
- [ ] `grep -r "fn auto_generate_edge_ids" engine/src/` retorna exactamente 1 resultado

---

## S11: Eliminar magic strings

### Qué es hoy

```rust
// runner.rs:614
if node.tool_type == "logic/human_input" {

// runner.rs:744
if node.tool_type.starts_with("ai/") {

// runner.rs:1044
error_output.insert("__error__".to_string(), ...);

// Transcript entry types (runner.rs, múltiples ubicaciones):
entry_type: "started".to_string(),
entry_type: "block_start".to_string(),
entry_type: "completed".to_string(),
entry_type: "error".to_string(),
entry_type: "decision".to_string(),
entry_type: "fanout_start".to_string(),
```

### Lo que hay que hacer

**1. Crear `engine/src/core/well_known.rs`:**

Este archivo contiene constantes con nombre semántico para todos los strings "mágicos" del motor:

- Tool type identifiers que el runner necesita reconocer:
  - `HUMAN_INPUT_TOOL` = `"logic/human_input"`
  - `AI_TOOL_PREFIX` = `"ai/"`

- Keys especiales en el state:
  - `ERROR_FIELD` = `"__error__"`

- Transcript entry types (opcionalmente un enum en vez de strings):
  - `TRANSCRIPT_STARTED`, `TRANSCRIPT_BLOCK_START`, `TRANSCRIPT_BLOCK_END`, `TRANSCRIPT_COMPLETED`, `TRANSCRIPT_ERROR`, `TRANSCRIPT_DECISION`, `TRANSCRIPT_FANOUT_START`, `TRANSCRIPT_FANOUT_END`

**2. Reemplazar cada magic string** en runner.rs y donde aplique con la constante correspondiente.

### Criterio de aceptación

- [ ] Cero strings literales "magic" en runner.rs que representen tool types, state keys, o entry types
- [ ] Cada constante tiene un nombre que explica su propósito
- [ ] `cargo test` pasa

---

## S12: Naming cleanup

### Los cambios de naming

Estos son renombramientos que mejoran claridad sin romper la API pública (gracias a re-exports):

| Actual | Nuevo | Por qué |
|---|---|---|
| `SimpleExecutionContext` | `DefaultExecutionContext` | "Simple" mina confianza. "Default" es el patrón Rust estándar. |
| `SharedState` | `ExecutionState` | ¿Compartido con quién? "Execution" describe qué estado es. |
| `GraphDef` | (mantener) | Aunque "Def" es frío, cambiarlo rompe demasiado. Mantener por ahora. |
| `AgentNodeSpec` / `AgentEdgeSpec` | `NodeSpec` / `EdgeSpec` | El prefijo "Agent" es redundante — ya están dentro de `AgentSpec` |

**Implementación:**

Para cada rename:
1. Renombrar el struct/tipo internamente
2. Agregar un re-export con el nombre viejo como alias (para backward compat):
   ```rust
   pub type SimpleExecutionContext = DefaultExecutionContext;
   ```
3. En la documentación y nuevos usos, usar el nombre nuevo
4. Deprecar el nombre viejo con `#[deprecated]` en una versión futura

### Criterio de aceptación

- [ ] Nombres nuevos en uso internamente
- [ ] Re-exports de backward compat funcionando
- [ ] `cargo build` sin warnings (excepto deprecation si se activan)

---

## S13: Session eviction + request timeout

### Session eviction

Hoy:
```rust
pub sessions: Arc<RwLock<HashMap<String, ExecutionResult>>>,
```
Sin límite. Crece infinitamente.

**Lo que hay que hacer:**

Implementar un límite configurable de sesiones en memoria con eviction LRU (Least Recently Used):

1. Agregar un campo `max_sessions: usize` a `AppState` (default: 10,000)
2. Cuando se inserta una nueva sesión y el HashMap supera el límite, eliminar la sesión más antigua
3. Para determinar "más antigua", agregar un campo `created_at` al tipo que se guarda, o usar un `IndexMap`/`LinkedHashMap` que preserve orden de inserción

Alternativa más simple: usar un `Vec<(String, ExecutionResult, Instant)>` con rotating buffer. Cuando se llena, el más viejo se elimina.

### Request timeout

Hoy: sin timeout. Un agente en loop infinito bloquea el handler indefinidamente.

**Lo que hay que hacer:**

Agregar un timeout de request configurable (default: 300 segundos) para los endpoints de ejecución:

1. Wrappear la ejecución del agente en un `tokio::time::timeout(duration, ...)`
2. Si el timeout expira, devolver HTTP 504 Gateway Timeout con un mensaje claro
3. El timeout aplica a: `POST /api/agents/{id}/execute` y `POST /api/agents/{id}/stream`
4. Configurable via `AppState` o variable de entorno `MIRAI_TIMEOUT_SECS`

### Criterio de aceptación

- [ ] Las sesiones nunca superan `max_sessions` en memoria
- [ ] Requests de ejecución tienen timeout configurable
- [ ] Timeout expirado retorna 504 con mensaje claro

---

## S14: Defaults consistentes

### Qué es hoy

```rust
// En AgentRetryConfig::default()
max_retries: 3
backoff: BackoffStrategy::Exponential
on_failure: FailureMode::Stop

// En RetryPolicy::default()
max_retries: 0
backoff: BackoffStrategy::None
on_failure: FailureMode::Stop
```

Dos structs que representan el mismo concepto, con defaults opuestos.

### Lo que hay que hacer

1. `RetryPolicy::default()` debe alinearse con `AgentRetryConfig::default()`:
   - `max_retries: 3`
   - `backoff: Exponential`

2. O alternativamente, que `AgentRetryConfig` use `RetryPolicy` internamente (eliminar la duplicación del struct).

3. Documentar con un comment por qué los defaults son esos valores (e.g., "3 retries with exponential backoff is the industry standard for transient failures").

### Criterio de aceptación

- [ ] Un solo comportamiento por defecto para retries en todo el sistema
- [ ] Tests que verifican que los defaults son consistentes

---

# FASE 3: "Think different"

Estos cambios son estructurales. Cambian cómo se siente el código a nivel arquitectónico.

---

## S15: Renombrar resources → adapters

### Qué es hoy

```
engine/src/resources/
  adapter_bridge.rs     ← ya tiene "adapter" en el nombre
  context.rs
  in_memory_db.rs
  in_memory_storage.rs
  local_storage.rs
  mock_llm.rs
  ollama_llm.rs
  simple_vector.rs
  sqlite_db.rs
```

Los traits (ports) viven en `core/context.rs`: `DBResource`, `LLMResource`, `StorageResource`, `VectorResource`.
Las implementaciones (adapters) viven en `resources/`.

En arquitectura hexagonal, los traits son **ports** y las implementaciones son **adapters**. Llamar "resources" a los adapters es genérico y no comunica la relación.

### Lo que hay que hacer

1. Renombrar el directorio: `engine/src/resources/` → `engine/src/adapters/`
2. Actualizar `lib.rs`: `pub mod resources` → `pub mod adapters`
3. Agregar re-export para backward compat: `pub mod resources { pub use crate::adapters::*; }` — o directamente actualizar todos los imports internos
4. Actualizar los comentarios del módulo

### Criterio de aceptación

- [ ] El directorio se llama `adapters/`
- [ ] La relación ports/adapters es obvia al leer la estructura
- [ ] API pública preservada (via re-exports si necesario)

---

## S16: Feature flags en Cargo.toml

### Lo que hay que hacer

Definir features en `engine/Cargo.toml` para que el motor compile solo lo necesario:

```toml
[features]
default = ["server", "builtin-tools"]
server = ["dep:axum", "dep:tower-http"]
builtin-tools = []
render = []
mcp = []
energy = []
search = []
vault = []
```

Los features-flagged modules se compilan condicionalmente:
```rust
#[cfg(feature = "server")]
pub mod server;

#[cfg(feature = "render")]
pub mod render;
```

### Por qué

Un usuario que solo quiere el motor como librería (sin servidor HTTP) no necesita `axum` como dependencia. Un usuario que no necesita renderizar charts no necesita el módulo `render`.

Esto reduce el tamaño del binary y el tiempo de compilación para quien solo usa el core.

### Criterio de aceptación

- [ ] `cargo build --no-default-features` compila solo el core
- [ ] `cargo build` (default) compila todo como hoy
- [ ] Cada feature flag está documentada

---

## S17: Consolidar doble abstracción LLM

### Qué es hoy

```
LLMAdapter (llm/adapter.rs)
  → call(), call_with_messages(), stream_with_messages(), list_models()
  → Implementado por: ClaudeAdapter, GeminiAdapter, OllamaAdapter, etc.

LLMResource (core/context.rs)
  → call(), embed()
  → Implementado por: MockLLMResource, OllamaLLMResource

AdapterBridgeLLMResource (resources/adapter_bridge.rs)
  → Convierte LLMAdapter en LLMResource
```

Dos traits para el mismo concepto con un bridge obligatorio en el medio.

### Lo que hay que hacer

Evaluar si se puede unificar. Dos opciones:

**Opción A: LLMResource absorbe la funcionalidad de LLMAdapter**
- Agregar `call_with_messages()` y `list_models()` a `LLMResource`
- Los adapters implementan `LLMResource` directamente
- Eliminar `LLMAdapter` trait y `AdapterBridgeLLMResource`

**Opción B: Documentar explícitamente por qué existen las dos capas**
- Si hay una razón arquitectónica real (e.g., `LLMResource` es la interfaz interna simplificada, `LLMAdapter` es la interfaz completa del provider), documentarlo con comments de arquitectura
- Agregar un diagrama en el módulo que explique la relación

La opción a elegir depende de si los tools/runner realmente necesitan solo `call()` + `embed()` (LLMResource simplificado) o si eventualmente necesitarán `call_with_messages()` (LLMAdapter completo). Evaluar en el momento de implementación.

### Criterio de aceptación

- [ ] O una sola abstracción, o documentación explícita de por qué son dos
- [ ] No hay "bridge" sin explicación

---

## S18: Python SDK mejorado

### Qué es hoy

55 líneas para Agent, 216 para Engine, 45 para types. Funcional pero mínimo.

### Lo que hay que hacer

1. **Type hints completos** — `Engine.run()` debería retornar `ExecutionResult` con autocompletion en IDEs
2. **`Agent.validate()`** — validación client-side antes de enviar al motor (verificar que nodes y edges existen, que tool_types son conocidos)
3. **Mejor error reporting** — cuando `_run_via_cli` falla, el error debería incluir qué comando se intentó, qué salida dio stderr, y una sugerencia de qué verificar
4. **`Agent.from_template()` funcional** — que realmente llame al motor para obtener el template, no un placeholder
5. **Tests** — al menos tests unitarios para Agent y tipos

### Criterio de aceptación

- [ ] `from datamirai import Engine, Agent` funciona con autocompletion en VS Code
- [ ] `Agent.validate()` detecta errores básicos sin necesitar el motor
- [ ] Hay al menos 10 tests para el SDK
- [ ] Los errors incluyen contexto útil para debugging

---

## S19: API versioning

### Lo que hay que hacer

Prefixar todas las rutas de la API con `/api/v1/`:

```
/api/agents       → /api/v1/agents
/api/graphs       → /api/v1/graphs
/api/sessions     → /api/v1/sessions
/api/tools        → /api/v1/tools
/api/templates    → /api/v1/templates
/api/universe     → /api/v1/universe
/api/rag          → /api/v1/rag
/api/eval         → /api/v1/eval
/api/metrics      → /api/v1/metrics
```

Los endpoints sin versión (`/health`, `/version`, `/webhooks`) quedan sin prefijo.

Opcionalmente, mantener las rutas sin versión como aliases que redireccionan a v1 (backward compat durante una versión).

### Criterio de aceptación

- [ ] Todas las rutas de API tienen prefijo `/api/v1/`
- [ ] `/health` y `/version` siguen sin prefijo
- [ ] Python SDK actualizado para usar las nuevas rutas
- [ ] Documentación actualizada

---

## S20: run_from() → arquitectura de 8 líneas

### Qué es hoy

El método `run_from()` en `GraphRunner` tiene 638 líneas. Hace absolutamente todo: setup, loop principal, hook calls, event emission, streaming, checkpoint saves, fan-out detection, condition evaluation, error handling con failure modes, transcript generation, y cleanup.

### Lo que hay que hacer

Descomponer `run_from()` para que el **algoritmo principal** sea visible en ~10-15 líneas, con toda la complejidad delegada a métodos con nombres descriptivos.

La estructura objetivo (pseudocódigo, no código Rust literal):

```
async fn run_from(graph, context, state, entry_node) {
    validate(graph)?;
    emit_graph_started(graph, context);
    call_hook_on_graph_start(graph, context)?;

    let mut cursor = Cursor::starting_at(graph, entry_node);

    while let Some(node) = cursor.current_node() {
        check_pause_requested()?;
        check_max_iterations(node)?;
        check_human_input_interrupt(node)?;

        let inputs = resolve_and_merge_inputs(node, graph, state);
        let inputs = call_hook_pre_block(node, inputs, context)?;

        let result = execute_node_with_retry(node, inputs, context);

        match result {
            Ok(output) => {
                call_hook_post_block(node, output, context);
                save_to_state(node, output, state);
                record_trace_success(node, output);
                cursor.advance_to_next(node, output, graph);
            }
            Err(error) => {
                let action = call_hook_on_error(node, error, context);
                handle_failure(node, error, action, cursor, state)?;
            }
        }

        save_checkpoint(state, cursor);
    }

    call_hook_on_graph_end(graph, state, context);
    emit_graph_completed(state, trace);
    build_execution_result(state, trace, transcript)
}
```

Cada método llamado aquí tiene un nombre que explica exactamente qué hace. La complejidad de cada uno (event emission, streaming, transcript entries, etc.) está encapsulada.

**Esto NO es un refactor del algoritmo** — es un refactor de presentación. La lógica es la misma, pero organizada para que el flujo sea legible de un vistazo.

### Dependencia

Este cambio depende de S06 (partir runner.rs en archivos). Hacer S06 primero, luego S20.

### Criterio de aceptación

- [ ] `run_from()` no supera 50 líneas
- [ ] Cada submétodo tiene un nombre autodescriptivo
- [ ] El algoritmo de traversal es comprensible leyendo solo `run_from()`
- [ ] Todos los tests existentes pasan sin cambios
- [ ] El comportamiento es idéntico al actual (mismo orden de events, hooks, checkpoints)

---

# Dependencias entre secciones

```
S01 (README)           → independiente
S02 (Examples)         → independiente
S03 (Código muerto)    → independiente
S04 (Auth)             → independiente
S05 (Shutdown)         → independiente

S06 (Partir runner)    → independiente
S07 (Partir app)       → independiente (pero mejor después de S04/S05)
S08 (Partir tools)     → independiente
S09 (Tipos)            → independiente (pero S10 depende parcialmente)
S10 (Duplicados)       → S09 resuelve una duplicación
S11 (Magic strings)    → mejor después de S06
S12 (Naming)           → independiente
S13 (Eviction)         → mejor después de S07
S14 (Defaults)         → independiente

S15 (Adapters)         → independiente
S16 (Feature flags)    → mejor después de S06-S08
S17 (LLM unificación)  → independiente
S18 (Python SDK)       → después de S19 (API versioning)
S19 (API versioning)   → después de S07
S20 (run_from refactor) → DEPENDE de S06
```

---

# Prioridad de ejecución sugerida

```
Batch 1 (paralelo):  S01 + S02 + S03      → Primera impresión
Batch 2 (paralelo):  S04 + S05            → Production-ready
Batch 3 (paralelo):  S06 + S07 + S08      → Estructura elegante
Batch 4 (paralelo):  S09 + S11 + S14      → Coherencia interna
Batch 5 (secuencial): S10 (después de S09) → DRY
Batch 6 (paralelo):  S12 + S13            → Polish
Batch 7 (secuencial): S20 (después de S06) → La joya de la corona
Batch 8 (paralelo):  S15 + S16 + S17      → Arquitectura
Batch 9 (secuencial): S19 → S18           → API + SDK
```
