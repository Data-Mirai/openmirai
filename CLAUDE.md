# Data Mirai Engine — Instrucciones

## Estructura del repositorio

```
Engine/
  engine/    → Crate Rust — EL MOTOR REAL. Aquí se trabaja.
  cli/       → CLI interactivo (Rust, consume engine/)
  legacy/    → Paquete Python LEGACY. NO TOCAR. Solo referencia histórica.
  docs/      → Documentación del proyecto
  blueprint/ → Blueprint Agents (workboard, config)
```

**REGLA CRITICA**: `legacy/` es código muerto. NO implementar features ahí. Todo va en `engine/` (Rust).

El motor se distribuye como:
- **Binario Rust**: FFI para Python/Swift/Go, WASM para browser, CLI directo
- **Agente = JSON portable**: configuración que se le pasa al motor desde cualquier lenguaje
- **Host = app que abraza motor + agente**: backend Python, app Swift, servicio Go, etc.

## Regla de testing — OBLIGATORIA

**NUNCA usar mocks, placeholders, ni implementaciones falsas en tests E2E.**

Los tests E2E con Playwright son la UNICA forma válida de verificar que el código funciona. Deben:
- Correr con `--headed` para ver la UI real
- Usar el backend REAL (no mocks)
- Ejecutar flujos REALES (web scraping real, LLM real, DB real)
- Verificar datos REALES en los resultados (no inventados)
- Los ciclos de desarrollo solo están completos cuando los E2E integration tests pasan con UI real

**NO se acepta:**
- Tests que pasan pero no prueban nada real
- Endpoints placeholder que retornan datos fake
- Mocks de servicios que deberían funcionar de verdad
- Decir "ya quedó" sin haber corrido los E2E con headed

Si un endpoint no está implementado, no se considera terminado. Punto.

---

Motor open source de ejecucion de grafos agentivos. Alternativa a LangGraph y Google ADK.

## Que es

Data Mirai Engine es un crate Rust que ejecuta grafos de bloques. Es el core del producto Data Mirai, publicado como open source. Se consume via FFI, WASM, HTTP o CLI.

## Stack

- Rust (motor principal — `engine/`)
- Tokio (async runtime)
- Axum (HTTP server)
- SQLite / rusqlite (persistencia local)
- Serde (serialización JSON/YAML)

## PRD y Arquitectura

Toda la documentacion tecnica del engine esta en `docs/prd/draft/DATAMIRAI-ENGINE-PRD.md`.
Leer ANTES de proponer cambios.

## Reglas absolutas

- Idioma: espanol
- Tono: senior, directo, sin walls of text
- Git: NUNCA push (PM lo hace). NUNCA force push, reset --hard, clean -f, rebase sin autorizacion.
- Archivos sagrados: NUNCA editar `.env*`
- Sin codigo en documentacion
- Siempre leer documentacion antes de proponer cambios

<!-- BLUEPRINT:BEGIN — no borrar estos markers; `blueprint init` los usa para actualizaciones idempotentes -->
## Blueprint Agents

Este proyecto usa **Blueprint Agents** para diseño + implementación.

Al iniciar una sesión, Claude Code DEBE cargar primero el orquestador del Core:

@/Users/gabo/.blueprint/CLAUDE.md

Ese archivo contiene el protocolo, la tabla de routing de intención y los gates obligatorios de los 2 ciclos. Todas sus reglas aplican a esta sesión.

Artefactos de Blueprint que pertenecen a este proyecto (no al Core):

- `blueprint/project.yaml` — config del proyecto
- `blueprint/workboard.db` — tickets, epics, bloques (SQLite)
- `blueprint/memory/` — decisions, patterns, rejected
- `blueprint/BITACORA.md` — historial narrativo append-at-top
- `blueprint/STATUS.json` — snapshot portable para observabilidad externa
- `docs/` — dominio, flujos, schema, API, pantallas, componentes, design guide, primitives, infra

Los artefactos del Core viven en `/Users/gabo/.blueprint/` y se comparten entre todos tus proyectos.

### Si el Core no está instalado

Si el import @/Users/gabo/.blueprint/CLAUDE.md falla:

```bash
git clone https://github.com/Gabo-TheCreator/blueprint-agents.git ~/projects/blueprint-agents
cd ~/projects/blueprint-agents && ./install.sh
```

### Primeros pasos

1. `/bootstrap` (una sola vez) — llenar docs con contenido del proyecto
2. `/init` en cada sesión · `/idea X` para diseñar · `/code BLOCK-X` para implementar
3. `/bhelp` para el cheat sheet completo
<!-- BLUEPRINT:END -->
