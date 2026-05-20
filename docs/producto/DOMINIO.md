# Data Mirai Engine — Dominio

## Jerarquía principal

```
Universe (instancia de aplicación)
  ├── Environments (dev / staging / prod)
  │     ├── Resources (DB, vector, storage, LLM — conexiones configuradas)
  │     ├── Agents (grafos desplegados con memoria)
  │     └── Auth config (provider + roles activos)
  └── Accounts (usuarios con roles por environment)
```

## Entidades

| Entidad | Definición | Pertenece a |
|---|---|---|
| **Universe** | Instancia de aplicación. Contiene ambientes, recursos, agentes | Root |
| **Environment** | Ambiente aislado (dev/staging/prod). Recursos y agentes separados por ambiente | Universe |
| **Resource** | Conexión configurada a servicio externo (DB, vector, storage, LLM) | Environment |
| **Agent** | Grafo desplegado que tiene memoria y ejecuta de forma continua | Environment |
| **Graph** | Flujo visual = nodos conectados por edges. Versión diseño de un agente | Environment |
| **Tool** | Herramienta reutilizable. Acción ejecutable con inputs/outputs definidos. Puede ser MCP server | Global (registry) |
| **Node** | Instancia de una herramienta dentro de un grafo. Tiene config específica | Graph |
| **Edge** | Conexión entre nodos. Puede tener condición y data_map | Graph |
| **Trigger** | Punto de entrada que activa ejecución (webhook, schedule, event, manual) | Agent |
| **Session** | Ejecución individual de un agente. Tiene traza y resultado | Agent |
| **Memory** | Conocimiento del agente: corto plazo (sesión) + largo plazo (persistente) + bitácora (compartida) | Agent / Environment |

## Roles

| Rol | Scope | Capacidades |
|---|---|---|
| **OWNER** | Universe | Crear/destruir environments, gestionar recursos, asignar roles, todo lo de ADMIN |
| **ADMIN** | Environment | Gestionar agentes, configurar recursos, ver logs, todo lo de EDITOR |
| **EDITOR** | Environment | Diseñar grafos, crear/editar agentes, ejecutar manual, ver resultados |
| **VIEWER** | Environment | Ver dashboards, logs, estado de agentes. Solo lectura |

Roles asignados por environment — un usuario puede ser ADMIN en dev y VIEWER en prod.

## Auth — Modelo agnóstico

```
Auth Provider (externo: Clerk, Auth0, custom)
        │ token firmado
        ▼
App/Gateway (middleware verifica firma)
        │ identidad verificada
        ▼
context.auth = { user_id, role, universe_id, environment_id }
        │
        ▼
Engine evalúa permisos sobre recursos/agentes
```

- Engine NO maneja: login, signup, sesiones, SSO, tokens, passwords
- Engine SÍ maneja: modelo de roles, permisos por recurso/agente, control de acceso
- Provider = pluggable. Sin auth configurado → modo sin restricciones (single-user)

## Permisos sobre recursos

| Recurso | OWNER | ADMIN | EDITOR | VIEWER |
|---|---|---|---|---|
| DB (relacional) | CRUD + schema | CRUD | Read + Write (tablas asignadas) | Read (tablas asignadas) |
| DB (vectorial) | CRUD + índices | CRUD | Read + Write | Read |
| Storage | CRUD + policies | CRUD | Read + Write (paths asignados) | Read |
| LLM | Config + uso | Config + uso | Uso | — |
| Agentes | CRUD + deploy | CRUD + deploy | Edit + execute | View logs |

## Glosario

Fuente de verdad: PRD §Glosario. Términos adicionales:

| Concepto | Definición |
|---|---|
| **Universe** | Instancia de aplicación con environments, recursos y agentes |
| **Environment** | Ambiente aislado dentro de un universe. Recursos no se comparten entre ambientes |
| **Resource** | Conexión configurada a servicio externo. Definida por tipo + credenciales + config |
| **Account** | Identidad de usuario dentro de un universe. Tiene roles por environment |
| **Role** | Conjunto de permisos asignado a un account dentro de un environment |
