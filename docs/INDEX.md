<!--
BLUEPRINT SEED — INDEX.md
Responsable: → blueprint/agents/00-WORKBOARD.md

Estructura esperada:
1. Overview del producto (máximo 1 párrafo)
2. Stack tecnológico (referencia breve a ARCHITECTURE.md)
3. Mapa de documentación (tabla con ruta a cada archivo)
4. Decisiones arquitectónicas clave (solo enlaces a ARCHITECTURE.md o memory/decisions.md)
5. PRDs (índice con enlaces a prd/backlog y prd/vX.X.X)

Reglas:
- Es un mapa de navegación, no repositorio de información
- No duplicar contenido. Solo apuntar
- Se actualiza cuando nace un PRD nuevo o cuando cambia el mapa de docs
-->

# INDEX.md

## 1. Overview del Producto

> _[Por completar — `/init` lo llena]_
> Qué es, qué problema resuelve, para quién. Máximo 1 párrafo.

## 2. Stack Tecnológico

> _[Por completar]_ Referencia breve. Detalles completos en → ARCHITECTURE.md.

## 3. Mapa de Documentación

| Archivo | Descripción | Ruta |
|---|---|---|
| DOMINIO.md | Roles, capabilities, glosario | producto/DOMINIO.md |
| FLUJOS.md | Flujos de negocio, máquinas de estado, reglas | producto/FLUJOS.md |
| SCHEMA.md | Esquema de base de datos, entidades, enums, RLS | database/SCHEMA.md |
| STORAGE.md | Buckets de storage y políticas | database/STORAGE.md |
| API.md | Endpoints REST, guards, contratos | backend/API.md |
| PRIMITIVES.md | Catálogo de services/hooks/utils reutilizables | backend/PRIMITIVES.md |
| PANTALLAS.md | Pantallas, rutas, estados UI | frontend/PANTALLAS.md |
| COMPONENTS.md | Catálogo de componentes UI (compuestos + átomos) | frontend/COMPONENTS.md |
| DESIGN-GUIDE.md | Tokens de diseño y variantes visuales | frontend/DESIGN-GUIDE.md |
| INFRA.md | Infraestructura, ambientes, deploy | infra/INFRA.md |
| ARCHITECTURE.md | Stack, patrones, convenciones | ARCHITECTURE.md |
| TESTS.md | Escenarios GWT con test_id | TESTS.md |

## 4. Decisiones Arquitectónicas Clave

> _[Por completar — se pobla a medida que se toman decisiones]_
> Formato: `[YYYY-MM-DD] Decisión — Razón.` Detalles en memory/decisions.md.

## 5. PRDs

> _[Por completar — se pobla al crear PRDs]_
> Lista con links a prd/backlog/ y prd/vX.X.X/.
