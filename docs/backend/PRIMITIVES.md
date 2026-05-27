<!--
BLUEPRINT SEED — PRIMITIVES.md
Responsable: → blueprint/agents/13-CURATOR.md

Estructura esperada:
1. Compuestos nivel medio (services, hooks compuestos, adapters, middlewares)
2. Átomos nivel micro (utils, helpers, formatters, validators, constantes)

Cada entrada: nombre, estado, responsabilidad, ubicación, API pública, dependencias, usos[], creado-en

Reglas:
- Todo primitive activo debe listar Usos[] (permite análisis de impacto)
- Cuando pasa a obsoleto, apuntar a su reemplazo
- Describir API y responsabilidad, NO el código
- Side-effects siempre explícitos
- Si un pattern se repite en 2+ módulos y no está aquí → Curator lo reporta
- Si cambia la API de un primitive activo, Curator reporta usos[] afectados ANTES del cambio
-->

# PRIMITIVES.md

## 1. Compuestos (nivel medio)

> _[Por completar — Curator lo puebla a medida que detecta patrones reutilizables]_
>
> Ejemplos típicos: `AuthService`, `NotificationService`, `usePermissions`, `httpClient`.
>
> Por cada primitive:
>
> ### prim-nombre {#prim-nombre}
> **Estado:** activo | candidato | obsoleto
> **Responsabilidad:** una frase.
> **Ubicación:** src/services/X, src/hooks/Y, etc.
> **API pública:**
> - `método(args): return`
> - `otro_método(args): return`
>
> **Side-effects:** escribe en DB, envía email, etc.
> **Dependencias:** otros primitives o libs externas.
> **Usos:**
> - PRD-XXX: módulo/endpoint
>
> **Creado en:** PRD-XXX
> **Reemplaza / Reemplazado por:** (si aplica)

## 2. Átomos (nivel micro)

> _[Por completar]_
>
> Ejemplos típicos: `formatCurrency`, `parseIsoDate`, `isValidEmail`, `groupBy`, `debounce`.
> Mismo formato que los compuestos, usualmente sin side-effects.
