<!--
BLUEPRINT SEED — STORAGE.md
Responsable: → blueprint/agents/03-ENTITIES.md

Estructura esperada:
- Por bucket: descripción, público (sí/no), límite tamaño, tipos permitidos, estructura de paths, policies

Reglas:
- NO SQL / código — declarativo en tablas
- Referenciar → DOMINIO.md#rol-X en las policies
- Incluir límites concretos (tamaño, tipos de archivo)
- Si el proyecto no usa object storage, este archivo puede quedarse vacío o eliminarse
-->

# STORAGE.md

## Buckets

> _[Por completar si el proyecto usa object storage (S3, Supabase Storage, GCS, etc.)]_
>
> Por cada bucket:
>
> ### bucket-nombre {#bucket-nombre}
> **Descripción:** para qué sirve.
> **Público:** sí / no
> **Límite de tamaño:** MB por archivo
> **Tipos permitidos:** image/png, application/pdf, etc.
> **Estructura de paths:** `{tenant-id}/{entidad}/{uuid}.{ext}`
>
> **Policies:**
> | Operación | Rol | Condición |
> |---|---|---|
> | SELECT | → DOMINIO.md#rol-X | [condición] |
