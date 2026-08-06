#!/usr/bin/env bash
# ===========================================================================
# OpenMirai — verificación del registro persistente
#
# El smoke corre contra un motor ya levantado, así que no puede probar lo único
# que importa acá: que los agentes sobrevivan a que el proceso se caiga. Este
# script parte la prueba en dos fases con el reinicio en el medio.
#
#   ./docker/persistencia.sh preparar     # registra agentes y arranca un live
#   docker compose restart engine         # (o matar y relevantar el motor)
#   ./docker/persistencia.sh verificar    # ¿volvieron? ¿el live sigue ciclando?
#
# O de una sola vez, pasándole cómo reiniciar:
#
#   REINICIO_CMD="docker compose restart engine" ./docker/persistencia.sh auto
#
# Variables: las mismas que smoke.sh (OPENMIRAI_URL, MIRAI_API_KEY,
# MIRAI_FLOWS_DIR) más ESTADO_FILE, donde se guardan los ids entre fases.
# ===========================================================================
set -uo pipefail

URL="${OPENMIRAI_URL:-http://engine:3000}"
CLAVE="${MIRAI_API_KEY:-}"
FLOWS="${MIRAI_FLOWS_DIR:-/opt/openmirai/flows}"
ESTADO="${ESTADO_FILE:-/tmp/openmirai-persistencia.json}"
REINICIO_CMD="${REINICIO_CMD:-}"

VERDE=$'\033[32m'; ROJO=$'\033[31m'; GRIS=$'\033[2m'; FIN=$'\033[0m'
FALLOS=0

api() {
    local metodo="$1" ruta="$2" cuerpo="${3:-}"
    local args=(-s -X "$metodo" "$URL$ruta" -H "Content-Type: application/json")
    [ -n "$CLAVE" ] && args+=(-H "X-API-Key: $CLAVE")
    [ -n "$cuerpo" ] && args+=(-d "$cuerpo")
    curl "${args[@]}"
}

registrar() {
    python3 -c 'import sys, json, yaml; json.dump(yaml.safe_load(open(sys.argv[1])), sys.stdout)' "$FLOWS/$1" \
        | curl -s -X POST "$URL/api/v1/agents/from-spec" \
            -H "Content-Type: application/json" \
            ${CLAVE:+-H "X-API-Key: $CLAVE"} -d @- \
        | jq -r '.id // empty'
}

esperar_motor() {
    for _ in $(seq 1 60); do
        curl -fsS "$URL/health" >/dev/null 2>&1 && return 0
        sleep 1
    done
    printf "%s✗ el motor no respondió en %s%s\n" "$ROJO" "$URL" "$FIN"
    exit 1
}

afirmar() {
    local nombre="$1" estado="$2" detalle="${3:-}"
    if [ "$estado" -eq 0 ]; then
        printf "  %s✓%s %-48s %s%s%s\n" "$VERDE" "$FIN" "$nombre" "$GRIS" "$detalle" "$FIN"
    else
        printf "  %s✗%s %-48s %s\n" "$ROJO" "$FIN" "$nombre" "$detalle"
        FALLOS=$((FALLOS + 1))
    fi
}

# --- fase 1 ----------------------------------------------------------------
preparar() {
    esperar_motor
    printf "Preparando estado en %s\n" "$URL"

    local id_normal id_live play
    id_normal=$(registrar 01-pipeline-lineal.yaml)
    id_live=$(registrar 08-agente-live.yaml)

    if [ -z "$id_normal" ] || [ -z "$id_live" ]; then
        printf "%s✗ no se pudieron registrar los agentes%s\n" "$ROJO" "$FIN"
        exit 1
    fi

    play=$(api POST "/api/v1/agents/$id_live/play" | jq -r '.status // "?"')
    [ "$play" = "playing" ] || {
        printf "%s✗ el agente live no arrancó (status=%s)%s\n" "$ROJO" "$play" "$FIN"
        exit 1
    }

    jq -n --arg n "$id_normal" --arg l "$id_live" \
        '{normal: $n, live: $l}' > "$ESTADO"

    printf "  agente normal : %s\n  agente live   : %s (ciclando)\n" "$id_normal" "$id_live"
    printf "%sEstado guardado en %s — ahora reiniciá el motor y corré 'verificar'.%s\n" \
        "$GRIS" "$ESTADO" "$FIN"
}

# --- fase 2 ----------------------------------------------------------------
verificar() {
    [ -f "$ESTADO" ] || {
        printf "%s✗ no existe %s — falta correr 'preparar'%s\n" "$ROJO" "$ESTADO" "$FIN"
        exit 1
    }
    esperar_motor

    local id_normal id_live
    id_normal=$(jq -r .normal "$ESTADO")
    id_live=$(jq -r .live "$ESTADO")

    printf "\nDespués del reinicio:\n"

    # El motor arrancó de cero: si los agentes están, salieron del disco.
    local cod_normal cod_live
    cod_normal=$(curl -s -o /dev/null -w '%{http_code}' "$URL/api/v1/agents/$id_normal" ${CLAVE:+-H "X-API-Key: $CLAVE"})
    cod_live=$(curl -s -o /dev/null -w '%{http_code}' "$URL/api/v1/agents/$id_live" ${CLAVE:+-H "X-API-Key: $CLAVE"})

    [ "$cod_normal" = "200" ] && afirmar "el agente sobrevive al reinicio" 0 "mismo id: $id_normal" \
                              || afirmar "el agente sobrevive al reinicio" 1 "HTTP $cod_normal para $id_normal"
    [ "$cod_live" = "200" ] && afirmar "el agente live sobrevive al reinicio" 0 "mismo id: $id_live" \
                            || afirmar "el agente live sobrevive al reinicio" 1 "HTTP $cod_live para $id_live"

    # Sigue siendo ejecutable, no solo un registro consultable.
    local est
    est=$(api POST "/api/v1/agents/$id_normal/execute" '{"trigger_data":{}}' | jq -r '.status // "?"')
    [ "$est" = "Completed" ] && afirmar "el agente restaurado sigue ejecutándose" 0 \
                             || afirmar "el agente restaurado sigue ejecutándose" 1 "status=$est"

    # Lo que hace que la ejecución continua sobreviva: nadie llamó a /play.
    sleep 4
    local ciclos
    ciclos=$(api GET "/api/v1/agents/$id_live/cycles" | jq -r '.total_cycles // 0')
    [ "$ciclos" -ge 1 ] && afirmar "el live vuelve a ciclar sin que nadie lo lance" 0 "$ciclos ciclos" \
                        || afirmar "el live vuelve a ciclar sin que nadie lo lance" 1 "total_cycles=$ciclos"

    api POST "/api/v1/agents/$id_live/stop" >/dev/null 2>&1
    rm -f "$ESTADO"

    printf "\n"
    if [ "$FALLOS" -gt 0 ]; then
        printf "%s%s fallos%s\n\n" "$ROJO" "$FALLOS" "$FIN"
        exit 1
    fi
    printf "%sEl registro persistente funciona: los agentes y el ciclado sobreviven al reinicio.%s\n\n" "$VERDE" "$FIN"
}

case "${1:-auto}" in
    preparar) preparar ;;
    verificar) verificar ;;
    auto)
        [ -n "$REINICIO_CMD" ] || {
            printf "%s✗ el modo auto necesita REINICIO_CMD%s\n" "$ROJO" "$FIN"
            printf "  ej: REINICIO_CMD=\"docker compose restart engine\" %s auto\n" "$0"
            exit 1
        }
        preparar
        printf "\n%sReiniciando: %s%s\n" "$GRIS" "$REINICIO_CMD" "$FIN"
        eval "$REINICIO_CMD" || { printf "%s✗ falló el reinicio%s\n" "$ROJO" "$FIN"; exit 1; }
        verificar
        ;;
    *)
        printf "uso: %s [preparar|verificar|auto]\n" "$0"
        exit 1
        ;;
esac
