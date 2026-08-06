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

## Overview

OpenMirai is a **headless agentic engine** with no graphical user interface. The only user-facing design surface is the **CLI terminal UX**. This document specifies the terminal design conventions, ANSI color usage, and interaction patterns implemented in `cli/src/terminal.rs`.

All design is terminal-native: no CSS, no components framework, no breakpoints. The interface is text-based and keyboard-driven.

---

## 1. Principios Visuales

- **Tipo**: Interfaz de línea de comandos (CLI) — terminal-native
- **Densidad**: Densa pero legible — información compacta en pocas líneas
- **Tono**: Técnico, directo, sin ambigüedad — para usuarios que programan
- **Paradigma**: Agentic loop with real-time feedback — el usuario ve la máquina pensar y actuar
- **No tiene**: Diseño responsivo, breakpoints, componentes visuales reutilizables, iconografía consistente fuera de Unicode

---

## 2. Tokens de Color (ANSI) {#colors}

OpenMirai usa ANSI escape codes para colorear la salida terminal. Definidos en `cli/src/colors.rs`:

| Token | Código ANSI | Uso |
|---|---|---|
| `RESET` | `\x1b[0m` | Anula cualquier formato previo |
| `BOLD` | `\x1b[1m` | Énfasis: títulos, comandos, valores críticos |
| `DIM` | `\x1b[2m` | Información secundaria: metadata, timestamps, notas |
| `RED` | `\x1b[31m` | Errores, fallos, acciones destructivas |
| `GREEN` | `\x1b[32m` | Éxito, confirmación, resultado OK |
| `YELLOW` | `\x1b[33m` | Advertencias, confirmaciones pendientes, hints |
| `BLUE` | `\x1b[34m` | Inputs del usuario, prompts primarios |
| `MAGENTA` | `\x1b[35m` | Información del sistema, banners, encabezados |
| `CYAN` | `\x1b[36m` | Nombre de herramientas, acciones en curso |
| `WHITE` | `\x1b[37m` | Reservado (raramente usado) |
| `BRIGHT_RED` | `\x1b[91m` | Errores críticos |
| `BG_RED` | `\x1b[41m` | Reservado para casos extremos |

### Paleta en contexto

```
Banner de bienvenida:        MAGENTA + BOLD
Prompt principal (>):        BLUE + BOLD
Información de sesión:       DIM
Nombre de herramienta:       CYAN + `▶` (U+25B6)
Argumento de herramienta:    DIM
Resultado OK:                GREEN + `✓` (U+2713)
Resultado ERROR:             RED + `✗` (U+2717)
Confirmación pendiente:      YELLOW + `?` (U+003F)
Skip por usuario:            YELLOW + `⏭` (U+23ED)
Contexto/Token tracking:     DIM
```

---

## 3. Tipografía {#typography}

No hay fuentes configurables — OpenMirai usa el monoespacé del terminal del usuario. La estructura se logra con:

- **BOLD**: para énfasis jerárquico (títulos, valores críticos)
- **DIM**: para información de bajo nivel (metadata, hints, detalles)
- **Espaciado**: líneas en blanco, indentación de 4 espacios

### Patrones de línea

| Elemento | Patrón | Ejemplo |
|---|---|---|
| Encabezado de sección | `{BOLD}{KEY}:{RESET}    {VALUE}` | `Model:    claude-3-sonnet` |
| Metadata | `{DIM}... info ...{RESET}` | `(round 2)` |
| Acción de herramienta | `{CYAN}▶ {tool_type}{RESET} {DIM}{args}{RESET}` | `▶ fs/write_file path=/tmp/x` |
| Resultado | `{GREEN}✓{RESET} {DIM}summary (1.2s){RESET}` | `✓ files_changed=2 (1.2s)` |
| Error | `{RED}✗ {error_msg}{RESET} {DIM}(0.5s){RESET}` | `✗ file not found (0.5s)` |
| Confirmación | `{YELLOW}? Confirm {tool}? [Y/n]: {RESET}` | `? Confirm fs_write_file? [Y/n]:` |

---

## 4. Estructura y Espaciado {#spacing}

El terminal usa caracteres monoespacios. Espaciado se define en saltos de línea e indentación:

| Contexto | Espaciado | Uso |
|---|---|---|
| Antes de prompt | 1 línea en blanco | Separación visual entre turnos |
| Indentación de herramienta | 2 espacios | Indica herramienta anidada bajo agentic loop |
| Indentación de resultado | 4 espacios | Indica resultado bajo herramienta |
| Entre secciones | 1 línea en blanco | Separación lógica (setup, sesión, loop) |

---

## 5. Elementos de Feedback {#feedback}

### Spinner animado

Mientras el LLM piensa, se muestra un spinner Unicode (Braille patterns):

```rust
SPINNER: ["\u{280B}", "\u{2819}", "\u{2839}", "\u{2838}", "\u{283C}", "\u{2834}", "\u{2826}", "\u{2827}", "\u{2807}", "\u{280F}"]
```

Formato: `{MAGENTA}{spinner_char}{RESET} {DIM}Generating...{RESET}`

Se borra cuando la respuesta llega (línea se sobrescribe con `\r`).

### Timing

Todas las acciones de herramienta muestran tiempo transcurrido:

```
✓ count=42 (1.234s)
✗ Connection refused (0.045s)
```

### Token tracking

Resumen al final de la sesión o con `/tokens`:

```
Session: 124,356 tokens (98,234 in / 26,122 out) across 47 calls
Context: ~42,100 tokens in 38 messages
```

---

## 6. Banner de Bienvenida {#banner}

Se muestra al iniciar sesión interactiva. El banner y la información de sesión incluyen:

```
╔════════════════════════════════════════╗
║         OpenMirai v0.7.0               ║
║   Agentic coding in your terminal      ║
╚════════════════════════════════════════╝

Provider: claude
Model:    claude-3-sonnet-20250219
CWD:      /Users/gabo/Documents/project
Tools loaded: 45
Autonomy: copilot (max 25 rounds/turn)
Context window: 200K
Session: 550e8400-e29b-41d4-a716-446655440000
Type /help for commands, Ctrl+C while thinking to interrupt
```

Colores:
- `╔╗╚╝` → MAGENTA + BOLD
- Texto dentro → RESET
- Keys → DIM
- Values → BOLD

---

## 7. Loop Agentico {#agentic-loop}

### Turno típico

```
> (user input)

  (round 2)
  ▶ fs/read_file path=/src/main.rs
    ✓ lines=234 (0.023s)
  ▶ fs/edit_file path=/src/main.rs
    ✓ edits=1 (0.015s)

(assistant response text)
```

### Estados

| Estado | Icon | Color | Semántica |
|---|---|---|---|
| Herramienta ejecutándose | `▶` | CYAN | Acción inminente |
| Resultado OK | `✓` | GREEN | Éxito |
| Resultado ERROR | `✗` | RED | Falló |
| Saltado por usuario | `⏭` | YELLOW | Usuario rechazó |
| Confirmación pendiente | `?` | YELLOW | Esperando input |

---

## 8. Comandos Slash {#slash-commands}

El usuario puede ejecutar comandos internos con `/`:

```
/help              — Muestra ayuda con todos los comandos
/quit, /exit, /q   — Guardar sesión y salir
/clear             — Limpiar contexto (mantiene system prompt)
/tokens            — Mostrar uso de tokens
/tools             — Listar herramientas disponibles
/session           — Info de sesión actual
/sessions          — Listar sesiones guardadas (últimas 15)
/checkpoint        — Crear checkpoint con etiqueta
/compact           — Comprimir contexto manualmente
```

Formato de respuesta: línea informativa sin saltos extra.

---

## 9. Errores y Excepciones {#errors}

### Errores de LLM

```
{RED}Error: Model timed out. Try a smaller model or reduce context window.{RESET}
{RED}Error: Cannot connect to provider. Is it running?{RESET}
{RED}Error: {detailed_error_message}{RESET}
```

### Errores de herramienta

```
  ▶ fs/write_file path=/tmp/x
    {RED}✗ Permission denied (0.008s){RESET}
```

---

## 10. Configuración de Autonomía {#autonomy}

OpenMirai soporta 4 niveles de autonomía. Cada uno afecta el comportamiento del terminal y requiere confirmación de acciones de escritura según el nivel:

| Nivel | Max Rounds | Confirm Writes | Descripción |
|---|---|---|---|
| **assisted** (L1) | 1 | SÍ | Pregunta-respuesta. El humano lidera cada paso. |
| **copilot** (L2) | 25 | NO | Humano pide, agente ejecuta N pasos, humano revisa. |
| **autopilot** (L3) | 50 | NO | Agente reacciona a eventos, humano supervisa. |
| **self_driving** (L4) | 100 | NO | Agente persigue objetivos, humano observa. |

Mostrado en el encabezado de sesión:

```
Autonomy: autopilot (max 50 rounds/turn)
```

Las herramientas que requieren confirmación en modo **assisted** son:
- `fs/write_file`, `fs/edit_file`, `fs/move`, `fs/copy`, `fs/delete`, `fs/mkdir`
- `system/bash`, `git/commit`

---

## 11. Persistencia de Sesión {#session-storage}

Las sesiones se guardan en `~/.datamirai/sessions/<session_id>/` con:
- `manifest.json` — metadata (id, provider, model, cwd, timestamps, message_count, checkpoint_count, status)
- `transcript.jsonl` — append-only log de cada evento (user messages, assistant responses, tool calls, tool results, checkpoints)

Las sesiones pueden cerrarse (status = "closed") o quedar activas. Se pueden listar las últimas 15 sesiones y crear checkpoints dentro de cada una.

---

## 12. Resumen de Diseño

**OpenMirai tiene un diseño minimalista porque es headless:**

✓ Terminal-native (ANSI, Unicode)  
✓ Texto monoespaciado, sin fuentes vectoriales  
✓ Colores semánticos (no temas)  
✓ Información densa pero legible  
✓ Feedback en tiempo real (spinner, timing, resultados)  
✓ Sin componentes reutilizables (CLI ≠ UI framework)  
✓ Persistencia de sesiones en `~/.datamirai/`  

**No hay:**
✗ Tokens de tipografía (una fuente del terminal)  
✗ Spacing escala (solo líneas en blanco e indentación)  
✗ Radius, shadows, efectos de profundidad  
✗ Breakpoints responsivos  
✗ Variantes de componentes visuales  
✗ Tema claro/oscuro (usa colores del terminal del usuario)  

---

## 13. Referencias de Implementación

Toda la lógica de terminal vive en:

- **`cli/src/terminal.rs`** → Loop agentico, spinner, feedback, execution, slash commands
- **`cli/src/colors.rs`** → Constantes ANSI (RESET, BOLD, DIM, RED, GREEN, YELLOW, BLUE, MAGENTA, CYAN, WHITE, BRIGHT_RED, BG_RED)
- **`cli/src/setup_wizard.rs`** → Wizard interactivo de inicio, detección de providers (Ollama, OpenAI, Claude, Gemini, Groq, NVIDIA, OpenRouter), selección de autonomía
- **`cli/src/session_storage.rs`** → Persistencia de sesiones en `~/.datamirai/sessions/`, manifest + transcript JSONL
- **`cli/src/main.rs`** → Puntos de entrada (`mirai run`, `mirai serve`, `mirai validate`, etc.)
