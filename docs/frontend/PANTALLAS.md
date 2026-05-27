<!--
BLUEPRINT SEED — PANTALLAS.md
Responsable: → blueprint/agents/06-SCREENS.md

Estructura esperada:
- Por módulo, por pantalla: ruta, acceso, datos consumidos, acciones, estados UI, wireframe ASCII

Reglas:
- No repetir permisos ni contratos — referenciar a DOMINIO.md / API.md
- Estados UI son propiedad exclusiva de este archivo
- Wireframe = guía de layout, no diseño final (el diseño lo define DESIGNER)
- No código de componentes — apuntar a COMPONENTS.md si se usan componentes del catálogo
-->

# PANTALLAS.md

## Pantallas

> _[Por completar — `/init` infiere de las rutas/pages si existen]_
>
> Por módulo, por pantalla:
>
> ### pantalla-nombre {#pantalla-nombre}
> **Ruta:** /path/:param
> **Acceso:** → DOMINIO.md#rol-X (+ guard de capability)
> **Componentes del catálogo:** → COMPONENTS.md#comp-X (si usa alguno)
>
> **Datos:**
> | Dato | Endpoint | Descripción |
> |---|---|---|
>
> **Acciones:**
> | Acción | Componente | Endpoint | Descripción |
> |---|---|---|---|
>
> **Estados UI:**
> | Estado | Condición | Qué muestra |
> |---|---|---|
> | loading | fetch en vuelo | skeleton |
> | empty | 0 resultados | mensaje "sin datos" |
> | error | fetch falla | mensaje + retry |
> | ready | datos cargados | contenido real |
>
> **Wireframe (ASCII, layout general):**
> ```
> [header]
> [sidebar] [contenido]
> ```
