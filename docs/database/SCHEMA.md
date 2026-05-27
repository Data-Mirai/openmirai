<!--
BLUEPRINT SEED — SCHEMA.md
Responsable: → blueprint/agents/03-ENTITIES.md

Estructura esperada:
1. Entidades (por entidad: campos, relaciones, índices, RLS, notas N+1)
2. Enums (derivados de FLUJOS.md cuando hay máquinas de estado)

Reglas:
- NO SQL literal (CREATE TABLE, ALTER, etc.). Todo declarativo en tablas markdown
- Enums derivan de FLUJOS.md — si FLUJOS cambia, enum cambia
- Referencias: → DOMINIO.md#rol-X y → FLUJOS.md#maquina-X
- RLS: operación + rol + condición (si el DB engine lo soporta; si no, policy en código)
- Índices y notas N+1 se agregan durante optimización, no al crear
- PKs siempre UUID (gen_random_uuid() o equivalente según engine)
- No DELETE físico — soft delete via campo status
-->

# SCHEMA.md

## 1. Entidades

> _[Por completar — `/init` infiere del schema actual si existe]_
>
> Por cada entidad:
>
> ### entidad-nombre {#entidad-nombre}
>
> **Campos:**
> | Campo | Tipo | Nullable | Default | Descripción |
> |---|---|---|---|---|
> | id | UUID | No | gen_random_uuid() | PK |
>
> **Relaciones:**
> | Campo FK | Entidad relacionada | Tipo | Descripción |
> |---|---|---|---|
>
> **Índices** {#indices-nombre} _(se agregan durante optimización)_
>
> **RLS / Policies** {#rls-nombre}
> | Operación | Rol | Condición |
> |---|---|---|
>
> **Notas N+1** _(si aplican)_

## 2. Enums {#enums}

> _[Por completar — derivar de FLUJOS.md]_
>
> Por cada enum:
>
> ### enum-nombre {#enum-nombre}
> **Usado en:** → entidad-X.campo
> **Fuente:** → FLUJOS.md#maquina-X
>
> | Valor | Descripción |
> |---|---|
