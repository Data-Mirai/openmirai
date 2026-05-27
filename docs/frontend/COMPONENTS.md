<!--
BLUEPRINT SEED — COMPONENTS.md
Responsable: → blueprint/agents/13-CURATOR.md

Estructura esperada:
1. Compuestos (nivel medio): Table, Form, Modal, Card, ListToolbar, etc.
2. Átomos (nivel micro): Button primary/secondary, Label title/body, Input, Icon, Spinner, Badge.

Pantallas/páginas NO viven aquí — viven en PANTALLAS.md (nivel macro).

Cada entrada: nombre, estado, responsabilidad, API (props + slots), estados visuales, tokens, usos[], creado-en

Reglas:
- Todo componente activo debe listar Usos[] (permite análisis de impacto)
- Cuando pasa a obsoleto, apuntar a su reemplazo
- No repetir tokens aquí — referenciar DESIGN-GUIDE.md
- Describir API + comportamiento, no el código
- Variantes (primary/secondary) van como un solo componente con prop `variant`
- Si un patrón se reutiliza 2+ veces y no está aquí → Curator lo reporta como inconsistencia
-->

# COMPONENTS.md

## 1. Compuestos (nivel medio)

> _[Por completar — Curator lo puebla a medida que detecta patrones reutilizables]_
>
> Ejemplos típicos: Table, Form, Modal, Card, ListToolbar, EmptyState.
>
> Por cada componente:
>
> ### comp-nombre {#comp-nombre}
> **Estado:** activo | candidato | obsoleto
> **Responsabilidad:** una frase.
> **API:**
> - `props: { campo: tipo, campo?: tipo }`
> - `slots: [slot-1, slot-2]`
>
> **Estados visuales:** loading | empty | error | con-datos | read-only
> **Tokens:** → DESIGN-GUIDE.md#spacing, #colors
> **Usos:**
> - PRD-XXX: PantallaX
>
> **Creado en:** PRD-XXX
> **Reemplaza / Reemplazado por:** (si aplica)
> **Notas:** (opcional)

## 2. Átomos (nivel micro)

> _[Por completar]_
>
> Ejemplos típicos: Button (primary/secondary/ghost/danger), Label (title/body/caption), Input, Icon, Spinner, Badge.
> Mismo formato que compuestos, usualmente sin slots.
