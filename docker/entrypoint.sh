#!/usr/bin/env bash
# ---------------------------------------------------------------------------
# Entrypoint del contenedor del motor OpenMirai.
#
# Razón de existir: `mirai serve` lee host y puerto SOLO de flags (--host /
# --port), nunca del entorno. En un contenedor lo natural es configurarlo por
# variable de entorno, así que acá traducimos MIRAI_HOST / MIRAI_PORT a flags.
#
# El resto de la configuración sí la lee el motor directo del entorno:
#   MIRAI_API_KEY        → autenticación de la API (header X-API-Key)
#   MIRAI_LLM_PROVIDER   → ollama | claude | openai | gemini | groq | nvidia |
#                          openrouter | mock
#   MIRAI_LLM_MODEL      → modelo por defecto
#   ANTHROPIC_API_KEY / GOOGLE_API_KEY / OPENAI_API_KEY / GROQ_API_KEY /
#   NVIDIA_API_KEY / OPENROUTER_API_KEY / OLLAMA_BASE_URL
# ---------------------------------------------------------------------------
set -euo pipefail

MIRAI_BIN=/usr/local/bin/mirai

# ¿Está ya presente este flag en los argumentos?
tiene_flag() {
    local buscado="$1"
    shift
    local arg
    for arg in "$@"; do
        [ "$arg" = "$buscado" ] && return 0
    done
    return 1
}

# Sin argumentos → arrancar el servidor.
if [ "$#" -eq 0 ]; then
    set -- serve
fi

# Permitir `docker run <imagen> bash` y equivalentes sin pasar por el CLI.
case "$1" in
    bash | sh | /bin/bash | /bin/sh)
        exec "$@"
        ;;
esac

if [ "$1" = "serve" ]; then
    shift
    args=(serve)

    tiene_flag --port "$@" || args+=(--port "${MIRAI_PORT:-3000}")
    tiene_flag --host "$@" || args+=(--host "${MIRAI_HOST:-0.0.0.0}")
    args+=("$@")

    echo "───────────────────────────────────────────────"
    echo " OpenMirai engine $("$MIRAI_BIN" version 2>/dev/null || echo '?')"
    echo " host      : ${MIRAI_HOST:-0.0.0.0}:${MIRAI_PORT:-3000}"
    echo " provider  : ${MIRAI_LLM_PROVIDER:-ollama (default del CLI)}"
    if [ -n "${MIRAI_API_KEY:-}" ]; then
        echo " auth      : X-API-Key activa"
    else
        echo " auth      : SIN CLAVE"
        case "${MIRAI_HOST:-0.0.0.0}" in
            127.*|localhost|::1)
                echo "             (bind local: el motor arranca igual)" ;;
            *)
                echo "             El motor se va a NEGAR a arrancar: desde 0.7.0"
                echo "             no bindea fuera de loopback sin MIRAI_API_KEY."
                echo "             Definila en .env o usá MIRAI_HOST=127.0.0.1." ;;
        esac
    fi
    if [ -n "${MIRAI_DB_PATH:-}" ]; then
        echo " registro  : ${MIRAI_DB_PATH}"
    else
        echo " registro  : en memoria — los agentes se pierden al reiniciar"
    fi
    echo " flows     : ${MIRAI_FLOWS_DIR:-/opt/openmirai/flows}"
    echo "───────────────────────────────────────────────"

    # exec → el binario queda como PID 1 y recibe SIGTERM directo; el motor
    # tiene graceful shutdown, así que `docker stop` termina limpio.
    exec "$MIRAI_BIN" "${args[@]}"
fi

# Cualquier otro subcomando del CLI (run, validate, tools, version…).
exec "$MIRAI_BIN" "$@"
