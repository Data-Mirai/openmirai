<!--
BLUEPRINT SEED — PANTALLAS.md
Responsable: → blueprint/agents/06-SCREENS.md

Estructura esperada:
- Por módulo, por pantalla: ruta, acceso, datos consumidos, acciones, estados UI, wireframe ASCII

Reglas:
- No repetir permisos ni contratos — referenciar a DOMINIO.md / API.md
- Estados UI son propiedad exclusiva de este archivo
- Wireframe = guía de layout, no diseño final (el diseño lo define DESIGNER)
- No código de componentes — apuntar a COMPONENTS.md si se usan componentes del catálogo
-->

# PANTALLAS.md — Interfaces de OpenMirai

## Resumen

**El núcleo de OpenMirai es _headless_, pero el binario incluye tres interfaces locales: la terminal interactiva (CLI), la HTTP API, y un editor visual de agentes (`mirai edit`, PRD-013).** No hay GUI multiusuario ni dashboard — el editor es una herramienta local de un agente a la vez.

Este documento clarifica qué interfaces existen y dónde se definen las experiencias de usuario downstream.

---

## Interfaces Disponibles

OpenMirai expone dos interfaces principales para interactuar con el motor:

### 1. **Terminal Interactiva (CLI)**

**Punto de entrada:** `mirai` (binario Rust)  
**Implementación:** → `/cli/src/terminal.rs`  
**Disponibilidad:** REPL interactivo agentico con sesiones persistentes

#### Sesión interactiva por defecto

```bash
mirai                  # Inicia wizard de setup → terminal interactiva
```

**Componentes visuales:**
- Setup wizard: configuración inicial de proveedor LLM + credenciales
- REPL: bucle interactivo usuario ↔ LLM ↔ tools
- Token tracker: resumen de tokens por sesión
- Colores + formatos ANSI: feedback visual de estado

**Autonomía configurable:**
- **assisted:** máx 1 ronda, usuario confirma writes
- **copilot:** máx 25 rondas de herramientas (sin confirmaciones)
- **autopilot:** máx 50 rondas sin confirmaciones
- **self_driving:** máx 100 rondas sin confirmaciones

**Subcomandos CLI:**

| Comando | Tipo | Descripción |
|---------|------|-------------|
| `mirai run <file.yaml>` | Ejecución | Ejecuta un agente de archivo; output JSON |
| `mirai validate <file.yaml>` | Validación | Valida spec sin ejecutar |
| `mirai serve --port N` | Servidor | Inicia HTTP API (ver abajo) |
| `mirai edit <file.yaml>` | Editor visual | Abre el mini-IDE en el navegador (ver §3) |
| `mirai version` | Info | Muestra versión |
| `mirai tools [type]` | Catálogo | Lista o detalla herramientas |
| `mirai templates` | Catálogo | Lista templates disponibles |
| `mirai new --template ID --name N` | Creación | Genera agente desde template |
| `mirai describe <file.yaml>` | Contrato | Muestra inputs/outputs del agente |
| `mirai eval` | Evaluación | Evalúa outputs (relevance, faithfulness, etc.) |
| `mirai rag search` | RAG | Búsqueda semántica en documentos |
| `mirai agent load` | Importación | Carga agente desde YAML (no implementado) |
| `mirai agent list` | Listado | Lista agentes cargados (no implementado) |

**Sin interfaz visual — todo es texto:**
- No hay pantallas, botones, diálogos gráficos
- Interacción vía STDIN/STDOUT en la terminal
- El LLM "ve" el catálogo de herramientas y razona sobre ellas
- Sesiones persistidas en SQLite local

---

### 2. **HTTP API**

**Punto de entrada:** HTTP Server (Axum)  
**Comando:** `mirai serve --port 3000`  
**Documentación completa:** → `/docs/backend/API.md`  
**Implementación:** → `/engine/src/server/`

**Endpoints raíz (sin auth):**
- `GET /health` — health check
- `GET /version` — versión del servidor

**Endpoints v1 (autenticación opcional vía `X-API-Key`):**

| Recurso | Métodos | Descripción |
|---------|---------|-------------|
| `/api/v1/graphs` | POST, GET | CRUD de grafos de agentes |
| `/api/v1/agents` | POST, GET | CRUD de agentes |
| `/api/v1/agents/from-spec` | POST | Crea agente desde spec |
| `/api/v1/agents/{id}` | GET | Obtiene agente |
| `/api/v1/agents/{id}/execute` | POST | Ejecuta agente (síncrono) |
| `/api/v1/agents/{id}/stream` | POST | Ejecuta agente (streaming) |
| `/api/v1/agents/{id}/spec` | GET | Obtiene spec del agente |
| `/api/v1/agents/{id}/schema` | GET | Obtiene schema (inputs/outputs) |
| `/api/v1/agents/{id}/play` | POST | Registra agente Live; el arranque actual no inicia el bucle del scheduler |
| `/api/v1/agents/{id}/stop` | POST | Detiene agente Live |
| `/api/v1/agents/{id}/cycles` | GET | Ciclos de un agente Live |
| `/api/v1/agents/{id}/memory` | GET, DELETE | Memoria del agente |
| `/api/v1/tools` | GET | Catálogo de herramientas |
| `/api/v1/templates` | GET | Templates disponibles |
| `/api/v1/sessions` | GET | Lista sesiones |
| `/api/v1/sessions/{id}` | GET | Obtiene sesión |
| `/api/v1/universe/message` | POST | Mensaje al universo de agentes |
| `/api/v1/universe/groupchat` | POST | Groupchat entre agentes |
| `/api/v1/metrics` | GET | Métricas del servidor |
| `/api/v1/rag/search` | POST | Búsqueda RAG |
| `/api/v1/eval` | POST | Evaluación de outputs |
| `/webhooks/{*path}` | POST | Placeholder: confirma recepción, pero no despacha a agentes |

**Autenticación:**
```
X-API-Key: <valor>
```
En loopback puede omitirse para desarrollo. En una dirección no-loopback, el
servidor se niega a arrancar si `MIRAI_API_KEY` no está configurada. Los
endpoints `/health`, `/version` y el contenido estático `/ui` no pasan por el
middleware de autenticación.

**Respuestas:**
- JSON estructurado con `status`, `data`, `error`, `trace`
- Streaming: SSE (Server-Sent Events) para `/agents/{id}/stream`

---

### 3. **Editor Visual (mini-IDE)** — `mirai edit`

**Punto de entrada:** `mirai edit <archivo.yaml>` (PRD-013)
**Implementación:** → `/engine/src/server/editor.rs` + SPA embebida `editor_index.html`
**Disponibilidad:** editor visual local de **un** agente a la vez, servido por el propio binario en loopback, sin dependencias externas.

```bash
mirai edit examples/hello-world.yaml    # abre el editor en el navegador
```

**Qué ofrece** (estilo n8n, tema blanco/negro plano):
- Canvas del grafo: nodos arrastrables, edges, auto-layout L→R
- Palette de las 50 tools por categoría + panel de propiedades por nodo (config según ToolSpec)
- Vista **Visual ⇄ YAML crudo** sincronizada
- **Compilar/validar** (diagnósticos), **Guardar** (YAML canónico + snapshots para undo/redo)
- **Probar** el agente con paso a paso (resaltado vía SSE)

**Estados UI:** cargando · nuevo (lienzo en blanco) · editando · inválido (diagnósticos) · parse_error · ejecutando.
**Endpoints propios:** `/api/edit/{agent,compile,undo,redo}` (reusa `/api/v1/*` para catálogo de tools y run).

---

## Dónde viven las Interfaces de Usuario

Las experiencias de usuario **NO están en este repositorio** (openmirai-engine):

| Aplicación | Tecnología | Ubicación | Propósito |
|---|---|---|---|
| **Mirai Local** (desktop) | Electron / React / Tauri | Repo separado | GUI local para OpenMirai |
| **Mirai Cloud** | Web app (React / Next.js) | Repo separado | SaaS frontend |
| **Integraciones SDK** | Python / TypeScript / Go | Clientes de openmirai | Embebida en apps |

Este repositorio (**openmirai-engine**) es solo el **motor sin interfaz**. Las pantallas, componentes, layouts las define el consumidor downstream.

---

## Por qué el núcleo es headless

OpenMirai prioriza **portabilidad y descentralización**; el editor visual es una capa local opcional, no parte del runtime:

✅ Un binario — corre en laptop, servidor, edge, CI/CD  
✅ Cero dependencias — no necesita GTK, Qt, web server, base de datos externa  
✅ Agnóstico a UI — cada consumidor (desktop, web, CLI) elige su stack  
✅ Compacto — ~30 MB compiled, bundled SQLite, batteries included  

OpenMirai es **infraestructura**; el editor `mirai edit` es una herramienta local opcional embebida (no un servicio ni dependencia de runtime). Las GUIs de producto (Mirai Local, Mirai Cloud) viven en sus repos.

---

## Resumen para el Equipo

**No busques (no existen aquí):**
- Pantallas de login / multiusuario
- Dashboard visual con métricas
- Edición colaborativa de varios agentes a la vez

**En su lugar:**
- Terminal interactiva REPL (`mirai`)
- HTTP API JSON (`mirai serve`)
- **Editor visual local de un agente (`mirai edit`)**
- Definiciones YAML versionables
- Integración en apps externas

El repositorio contiene el **motor agentico**. Las **pantallas** las construye Mirai Local o Mirai Cloud. Documentar eso en **sus** repos, no aquí.
