<!--
BLUEPRINT SEED — DESIGN-GUIDE.md
Responsable: → blueprint/agents/13-CURATOR.md

Estructura esperada:
1. Principios visuales (densidad, tono, referencias)
2. Tokens de color (tokens semánticos: primary, secondary, bg, surface, text, border, success/error/warning/info)
3. Tipografía (tokens semánticos: text-title-xl, text-body, text-caption)
4. Spacing (escala basada en múltiplos de 4px u 8px)
5. Radius / Shadows
6. Variantes de átomos (Button, Label, etc.)
7. Grid / Breakpoints

Reglas:
- Tokens son la fuente de verdad — componentes los consumen, nunca hardcodean valores
- Si un componente necesita un token que no existe, agregarlo aquí primero
- Cambios a tokens tienen impacto sistémico — Curator reporta `usos[]` afectados antes del cambio
- Los valores pueden definirse en CSS variables, theme object, o similar según stack (ver ARCHITECTURE.md)
- No duplicar componentes aquí — solo tokens + variantes conceptuales; la API va en COMPONENTS.md
-->

# DESIGN-GUIDE.md

## 1. Principios Visuales

> _[Por completar — `/init` pregunta al PM]_
> - Densidad: densa vs. aireada
> - Tono: formal / casual / técnico
> - Referencias (inspiración): …

## 2. Tokens de Color {#colors}

> _[Por completar]_

| Token | Valor | Uso |
|---|---|---|
| `color-primary` | | Botón primario, links, estados activos |
| `color-secondary` | | Botón secundario, acentos |
| `color-bg` | | Fondo principal |
| `color-surface` | | Tarjetas, modales |
| `color-text` | | Texto primario |
| `color-text-muted` | | Texto secundario |
| `color-border` | | Bordes de inputs, tablas |
| `color-success` | | Estado OK |
| `color-error` | | Estado error |
| `color-warning` | | Estado atención |
| `color-info` | | Estado informativo |

## 3. Tipografía {#typography}

> _[Por completar]_

| Token | Familia | Peso | Tamaño | Line-height | Uso |
|---|---|---|---|---|---|
| `text-title-xl` | | 700 | 32px | 1.2 | Títulos de página |
| `text-title-lg` | | 600 | 24px | 1.3 | Secciones |
| `text-title-md` | | 600 | 18px | 1.4 | Subsecciones |
| `text-body` | | 400 | 14px | 1.5 | Texto base |
| `text-caption` | | 400 | 12px | 1.4 | Metadatos, hints |

## 4. Spacing {#spacing}

> _[Por completar — escala basada en múltiplos de 4px u 8px]_

| Token | Valor |
|---|---|
| `space-xs` | 4px |
| `space-sm` | 8px |
| `space-md` | 16px |
| `space-lg` | 24px |
| `space-xl` | 32px |
| `space-2xl` | 48px |

## 5. Radius / Shadows {#radius-shadows}

> _[Por completar]_

| Token | Valor | Uso |
|---|---|---|
| `radius-sm` | 4px | Inputs, botones |
| `radius-md` | 8px | Cards |
| `radius-lg` | 12px | Modales |
| `shadow-sm` | | Cards en reposo |
| `shadow-md` | | Cards elevadas, dropdowns |

## 6. Variantes de Átomos {#variants}

> _[Por completar]_
>
> Por cada átomo con variantes:
>
> ```
> Button:
>   - primary: acción principal de la pantalla
>   - secondary: acción alternativa
>   - ghost: acciones en toolbars
>   - danger: acciones destructivas
> ```

## 7. Grid / Breakpoints {#layout}

> _[Por completar]_

| Breakpoint | Valor |
|---|---|
| mobile | < 640px |
| tablet | 640-1024px |
| desktop | > 1024px |
