<!--
BLUEPRINT SEED — API.md
Responsable: → blueprint/agents/07-CONTRACTS.md

Estructura esperada:
- Por recurso, por endpoint: descripción, auth, guard, validaciones, request, response, errores
- Cada endpoint con anchor único: {#METODO-ruta}

Reglas:
- NO copiar la estructura de SCHEMA.md — REFERENCIAR con → SCHEMA.md#entidad-X
- Guards se definen aquí (el nombre concreto del guard vive en ARCHITECTURE.md)
- Reglas de negocio se REFERENCIAN a FLUJOS.md, no se duplican aquí
- No código de controllers/services
- Endpoints en /api/v1/ con kebab-case plural (o la convención del proyecto)
- Toda mutación requiere auth
- Paginación estándar para listados (page, page_size)
-->

# API.md

## Endpoints

> _[Por completar — `/init` infiere de los controllers/routes si existen]_
>
> Por cada endpoint:
>
> ### METODO /api/v1/ruta {#METODO-ruta}
> **Descripción:** qué hace.
> **Auth:** JWT / API Key / público.
> **Autorización:** → DOMINIO.md#capabilities-X
> **Guard:** `NombreDelGuard` (ver ARCHITECTURE.md para implementación concreta).
>
> **Validaciones:**
> | Validación | Regla | Referencia |
> |---|---|---|
>
> **Request:** base → SCHEMA.md#entidad-X + campos DTO si difieren.
>
> **Response:** estructura + referencia a entidad.
>
> **Errores:**
> | Código | Condición | Mensaje |
> |---|---|---|
> | 400 | … | … |
> | 403 | sin capability | … |
> | 404 | recurso no existe | … |
>
> **Notas N+1:** → SCHEMA.md#n1-nombre (si aplica).
