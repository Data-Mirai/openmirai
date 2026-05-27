# Engine

<!-- BLUEPRINT:BEGIN — no borrar estos markers; `blueprint init` los usa para actualizaciones idempotentes -->
## Blueprint Agents

Este proyecto usa **Blueprint Agents** para diseño + implementación.

Al iniciar una sesión, Claude Code DEBE:

1. Cargar el orquestador del Core:
@/Users/gabo/.blueprint/CLAUDE.md

2. **Ejecutar `/init` automáticamente** antes de cualquier otra interacción con el usuario. No esperar a que el PM lo pida — hacerlo siempre al arrancar.

El orquestador contiene el protocolo completo: routing de intención, los ciclos de diseño (tech-agnostic) e implementación (autónoma con E2E), y todas las reglas. **Es mandatorio — todas sus reglas aplican a esta sesión sin excepción.**

Artefactos de Blueprint que pertenecen a este proyecto (no al Core):

- `blueprint/project.yaml` — config del proyecto
- `blueprint/BITACORA.md` — bitácora del proyecto (cold memory)
- `blueprint/memory/patterns.md` — patrones aprendidos
- `docs/workboard.db` — PRDs + tests (SQLite)
- `docs/prd/` — PRDs con diseño lógico + bitácora de implementación + assets
- `docs/` — dominio, flujos, schema, operaciones, interfaces, componentes, design guide

Los artefactos del Core viven en `/Users/gabo/.blueprint/` y se comparten entre todos tus proyectos.

### Comandos

1. `/init` — setup (proyecto nuevo) o retomar (existente)
2. `/idea X` — diseñar feature (PRD tech-agnostic)
3. `/code PRD-XXX` — implementar en branch aislada
4. `/merge PRD-XXX` — integrar a main con E2E
5. `/bhelp` — cheat sheet completo
<!-- BLUEPRINT:END -->
