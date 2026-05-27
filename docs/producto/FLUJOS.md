<!--
BLUEPRINT SEED — FLUJOS.md
Responsable: → blueprint/agents/04-STATE-MACHINES.md + blueprint/agents/08-RULES.md

Estructura esperada:
1. Flujos de negocio (por flujo: descripción, actor, pasos, resultado, errores)
2. Máquinas de estado (por entidad: estados, transiciones con guard + quién)
3. Reglas de negocio globales (REGLA-XX con descripción, condición, efecto)

Reglas:
- Cada transición tiene: de → a, condición (guard), quién (→ DOMINIO.md#rol-X)
- "Quién puede" siempre referencia a DOMINIO.md
- NO detalles de implementación (columnas, enums SQL) — eso va en SCHEMA.md
- Este archivo es FUENTE DE VERDAD para los enums de estado en SCHEMA.md
- Las reglas de negocio se identifican con ID único (REGLA-01, REGLA-02, ...)
-->

# FLUJOS.md

## 1. Flujos de Negocio

> _[Por completar]_
>
> Por flujo, usa este formato:
>
> ### flujo-nombre {#flujo-nombre}
> **Descripción:** qué logra el flujo.
> **Actor principal:** → DOMINIO.md#rol-X
> **Pasos:**
> 1. …
> 2. …
> **Resultado esperado:** …
> **Errores posibles:** …

## 2. Máquinas de Estado

> _[Por completar]_
>
> Por entidad con ciclo de vida:
>
> ### maquina-nombre {#maquina-nombre}
> **Entidad:** [nombre-entidad]
> **Estados:** STATE_A | STATE_B | STATE_C
>
> | De | A | Guard (condición) | Quién puede |
> |---|---|---|---|
> | STATE_A | STATE_B | [condición] | → DOMINIO.md#rol-X |
>
> **Side-effects:** notificaciones, cascadas a otras entidades, timestamps.

## 3. Reglas de Negocio Globales

> _[Por completar]_
>
> Por regla:
>
> ### REGLA-01 {#regla-01}
> **Descripción:** qué invariante protege.
> **Condición:** cuándo aplica.
> **Efecto:** qué pasa si se cumple / viola.
