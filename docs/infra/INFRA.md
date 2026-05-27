<!--
BLUEPRINT SEED — INFRA.md
Responsable: Orquestador (decisiones de infra) + PM

Estructura esperada:
1. Servicios cloud
2. Ambientes (local, staging, production)
3. Variables de entorno (NUNCA valores reales — solo descripción)
4. CI/CD (trigger, pasos, por ambiente)
5. Migraciones (herramienta, cómo crear/aplicar/revertir)
6. Checklist de deploy (por ambiente)

Reglas:
- NUNCA valores reales de variables sensibles (API keys, passwords, etc.)
- No código de scripts CI/CD — describir proceso
- Los secrets viven en el gestor del proyecto, no aquí
-->

# INFRA.md

## 1. Servicios Cloud

> _[Por completar]_

| Servicio | Proveedor | Propósito |
|---|---|---|

## 2. Ambientes

> _[Por completar]_

| Ambiente | URL | Propósito |
|---|---|---|
| local | http://localhost:… | Desarrollo |
| staging | | Pre-producción |
| production | | Producción |

## 3. Variables de Entorno

> _[Por completar — NUNCA valores reales]_

| Variable | Descripción | Ambientes | Ejemplo (sanitizado) |
|---|---|---|---|
| `DATABASE_URL` | conexión a DB | todos | `postgres://…` |

## 4. CI/CD

> _[Por completar]_
>
> **Trigger:** push a main / PR / manual
> **Pasos:** install → test → build → deploy
> **Por ambiente:** diferencias entre staging y production

## 5. Migraciones

> _[Por completar]_
>
> **Herramienta:** (ej. alembic, flyway, knex)
> **Crear migration:** comando
> **Aplicar:** comando
> **Revertir:** comando

## 6. Checklist de Deploy

> _[Por completar]_
>
> **Staging:**
> - [ ] Tests pasan
> - [ ] Migraciones revisadas
> - [ ] …
>
> **Production:**
> - [ ] Aprobación del PM
> - [ ] Backup de DB
> - [ ] …
