<!--
BLUEPRINT SEED — DOMINIO.md
Responsable: → blueprint/agents/02-ROLES.md

Estructura esperada:
1. Glosario del dominio (tabla término/definición/sinónimos)
2. Roles (por rol: descripción, jerarquía, capabilities)
3. Jerarquía de roles (herencia)

Reglas:
- Capabilities: nombre único snake_case, atómicas
- Solo WHAT (qué puede hacer cada rol), no HOW (cómo se implementa)
- Otros archivos referencian aquí con → DOMINIO.md#capabilities-X o #rol-X
- Este es un documento raíz: no depende de ningún otro
-->

# DOMINIO.md

## 1. Glosario del Dominio {#glosario}

> _[Por completar — `/init` propone términos del dominio detectados en el código]_

| Término | Definición | Sinónimos |
|---|---|---|
| _[término]_ | _[definición]_ | _[si aplica]_ |

## 2. Roles {#roles}

> _[Por completar]_
>
> Por cada rol, usa este formato:
>
> ### rol-nombre {#rol-nombre}
> **Descripción:** qué hace este rol en el producto.
> **Jerarquía:** hereda de → rol-padre (si aplica).
> **Capabilities:** {#capabilities-nombre}
> - `capability_snake_case` — qué permite hacer
> - `otra_capability` — qué permite hacer

## 3. Jerarquía de Roles

> _[Por completar]_ Diagrama de herencia o tabla de quién hereda de quién.
