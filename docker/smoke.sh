#!/usr/bin/env bash
# ===========================================================================
# OpenMirai — smoke e2e contra el motor dockerizado
#
# Golpea la API HTTP como lo haría un cliente de la nube: registra cada flujo
# con POST /api/v1/agents/from-spec y lo ejecuta con /execute o /stream.
#
# Uso:
#   docker compose run --rm smoke                 # dentro de la red de compose
#   OPENMIRAI_URL=http://127.0.0.1:4321 ./docker/smoke.sh   # contra un host
#
# Variables:
#   OPENMIRAI_URL     URL base del motor (default http://engine:3000)
#   MIRAI_API_KEY     clave para el header X-API-Key (vacío = sin auth)
#   MIRAI_FLOWS_DIR   directorio de los YAML (default /opt/openmirai/flows)
#
# Sobre las aserciones: se verifica el campo `status` del JSON de respuesta y
# el contenido de `state`/`trace`, nunca un grep de texto sobre la salida
# completa. El motivo está documentado en docker/README.md — la suite del repo
# (test/run_all.sh) busca la palabra "Completed" en cualquier parte de la
# salida y por eso reporta PASS sobre ejecuciones que en realidad fallaron.
#
# Códigos de salida: 0 = todo verde. 1 = al menos un fallo.
# Las BRECHAS conocidas del motor se reportan aparte y NO rompen la salida:
# son comportamientos actuales verificados a propósito.
# ===========================================================================
set -uo pipefail

URL="${OPENMIRAI_URL:-http://engine:3000}"
CLAVE="${MIRAI_API_KEY:-}"
FLOWS="${MIRAI_FLOWS_DIR:-/opt/openmirai/flows}"

VERDE=$'\033[32m'; ROJO=$'\033[31m'; AMARILLO=$'\033[33m'; GRIS=$'\033[2m'; FIN=$'\033[0m'

OK=0
FALLOS=0
BRECHAS=0
DETALLE_FALLOS=""

# --- helpers ---------------------------------------------------------------

# curl con auth. Uso: api <método> <ruta> [cuerpo]
api() {
    local metodo="$1" ruta="$2" cuerpo="${3:-}"
    local args=(-s -X "$metodo" "$URL$ruta" -H "Content-Type: application/json")
    [ -n "$CLAVE" ] && args+=(-H "X-API-Key: $CLAVE")
    [ -n "$cuerpo" ] && args+=(-d "$cuerpo")
    curl "${args[@]}"
}

# Igual que api() pero devuelve solo el código HTTP.
codigo_http() {
    local metodo="$1" ruta="$2" cuerpo="${3:-}" con_clave="${4:-si}"
    local args=(-s -o /dev/null -w '%{http_code}' -X "$metodo" "$URL$ruta" -H "Content-Type: application/json")
    [ -n "$CLAVE" ] && [ "$con_clave" = "si" ] && args+=(-H "X-API-Key: $CLAVE")
    [ -n "$cuerpo" ] && args+=(-d "$cuerpo")
    curl "${args[@]}"
}

yaml_a_json() {
    python3 -c 'import sys, json, yaml; json.dump(yaml.safe_load(open(sys.argv[1])), sys.stdout)' "$1"
}

# Registra un flujo y devuelve su agent_id.
registrar() {
    yaml_a_json "$FLOWS/$1" \
        | curl -s -X POST "$URL/api/v1/agents/from-spec" \
            -H "Content-Type: application/json" \
            ${CLAVE:+-H "X-API-Key: $CLAVE"} \
            -d @- \
        | jq -r '.id // empty'
}

ejecutar() {
    # Sin default inline: `${2:-{...}}` hace que bash corte la expansión en la
    # primera llave de cierre y concatene la sobrante al valor, rompiendo el JSON.
    local cuerpo="${2:-}"
    [ -z "$cuerpo" ] && cuerpo='{"trigger_data":{}}'
    api POST "/api/v1/agents/$1/execute" "$cuerpo"
}

# afirmar <nombre> <condición-ya-evaluada:0|1> [detalle]
afirmar() {
    local nombre="$1" estado="$2" detalle="${3:-}"
    if [ "$estado" -eq 0 ]; then
        printf "  %s✓%s %-52s %s%s%s\n" "$VERDE" "$FIN" "$nombre" "$GRIS" "$detalle" "$FIN"
        OK=$((OK + 1))
    else
        printf "  %s✗%s %-52s %s\n" "$ROJO" "$FIN" "$nombre" "$detalle"
        FALLOS=$((FALLOS + 1))
        DETALLE_FALLOS="${DETALLE_FALLOS}
  ✗ ${nombre} — ${detalle}"
    fi
}

# Brecha conocida del motor: se verifica el comportamiento actual, no rompe CI.
brecha() {
    printf "  %s!%s %-52s %s%s%s\n" "$AMARILLO" "$FIN" "$1" "$GRIS" "$2" "$FIN"
    BRECHAS=$((BRECHAS + 1))
}

titulo() { printf "\n%s\n" "$1"; }

# --- espera a que el motor esté arriba -------------------------------------

printf "OpenMirai — smoke e2e\n"
printf "%smotor: %s%s\n" "$GRIS" "$URL" "$FIN"

for intento in $(seq 1 60); do
    if curl -fsS "$URL/health" >/dev/null 2>&1; then break; fi
    if [ "$intento" -eq 60 ]; then
        printf "%s✗ el motor no respondió en %s tras 60 intentos%s\n" "$ROJO" "$URL" "$FIN"
        exit 1
    fi
    sleep 1
done

# ===========================================================================
titulo "── Contrato del servidor ──────────────────────────────────────────"
# ===========================================================================

SALUD=$(api GET /health)
ESTADO=$(jq -r '.status // "?"' <<<"$SALUD")
HERRAMIENTAS=$(jq -r '.tools_registered // 0' <<<"$SALUD")
VERSION=$(jq -r '.version // "?"' <<<"$SALUD")
[ "$ESTADO" = "ok" ] && afirmar "/health responde ok" 0 "v$VERSION" \
                     || afirmar "/health responde ok" 1 "status=$ESTADO"
[ "$HERRAMIENTAS" -ge 50 ] && afirmar "catálogo de herramientas cargado" 0 "$HERRAMIENTAS herramientas" \
                           || afirmar "catálogo de herramientas cargado" 1 "solo $HERRAMIENTAS (se esperaban ≥50)"

if [ -n "$CLAVE" ]; then
    CODIGO=$(codigo_http GET /api/v1/tools "" no)
    [ "$CODIGO" = "401" ] && afirmar "sin X-API-Key rechaza con 401" 0 \
                          || afirmar "sin X-API-Key rechaza con 401" 1 "devolvió $CODIGO — la API está abierta"
    CODIGO=$(codigo_http GET /api/v1/tools)
    [ "$CODIGO" = "200" ] && afirmar "con X-API-Key acepta" 0 \
                          || afirmar "con X-API-Key acepta" 1 "devolvió $CODIGO"
else
    brecha "servidor sin autenticación" "MIRAI_API_KEY vacío — no exponer fuera de la red local"
fi

CODIGO=$(codigo_http GET /health "" no)
[ "$CODIGO" = "200" ] && afirmar "/health queda público (sondas del orquestador)" 0 \
                      || afirmar "/health queda público (sondas del orquestador)" 1 "devolvió $CODIGO"

# ===========================================================================
titulo "── Flujos ─────────────────────────────────────────────────────────"
# ===========================================================================

# --- 01 pipeline lineal ---
ID=$(registrar 01-pipeline-lineal.yaml)
if [ -z "$ID" ]; then
    afirmar "01 pipeline lineal" 1 "no se pudo registrar el agente"
else
    R=$(ejecutar "$ID")
    EST=$(jq -r '.status // "?"' <<<"$R")
    N=$(jq -r '.trace | length' <<<"$R")
    { [ "$EST" = "Completed" ] && [ "$N" -eq 3 ]; } \
        && afirmar "01 pipeline lineal" 0 "3 nodos recorridos" \
        || afirmar "01 pipeline lineal" 1 "status=$EST nodos=$N"
fi

# --- 02 ruteo condicional (las dos ramas) ---
ID=$(registrar 02-ruteo-condicional.yaml)
for CASO in "alta:rama_urgente:rama_normal" "baja:rama_normal:rama_urgente"; do
    ENTRADA="${CASO%%:*}"; RESTO="${CASO#*:}"; ESPERADA="${RESTO%%:*}"; PROHIBIDA="${RESTO##*:}"
    R=$(ejecutar "$ID" "{\"trigger_data\":{\"prioridad\":\"$ENTRADA\"}}")
    NODOS=$(jq -r '[.trace[].node_id] | join(",")' <<<"$R")
    EST=$(jq -r '.status // "?"' <<<"$R")
    if [ "$EST" = "Completed" ] && [[ "$NODOS" == *"$ESPERADA"* ]] && [[ "$NODOS" != *"$PROHIBIDA"* ]]; then
        afirmar "02 ruteo condicional ($ENTRADA)" 0 "→ $ESPERADA"
    else
        afirmar "02 ruteo condicional ($ENTRADA)" 1 "status=$EST traza=[$NODOS]"
    fi
done

# --- 03 fan-out paralelo ---
ID=$(registrar 03-fanout-paralelo.yaml)
R=$(ejecutar "$ID")
EST=$(jq -r '.status // "?"' <<<"$R")
N=$(jq -r '.trace | length' <<<"$R")
{ [ "$EST" = "Completed" ] && [ "$N" -eq 5 ]; } \
    && afirmar "03 fan-out y fan-in" 0 "3 ramas en paralelo + cierre" \
    || afirmar "03 fan-out y fan-in" 1 "status=$EST nodos=$N (se esperaban 5)"

# --- 04 llamada a LLM ---
ID=$(registrar 04-llm-basico.yaml)
R=$(ejecutar "$ID" '{"trigger_data":{"pregunta":"¿Qué es OpenMirai?"}}')
EST=$(jq -r '.status // "?"' <<<"$R")
RESP=$(jq -r '.state.pensar.response // ""' <<<"$R")
{ [ "$EST" = "Completed" ] && [ -n "$RESP" ]; } \
    && afirmar "04 llamada a LLM" 0 "$(cut -c1-40 <<<"$RESP")…" \
    || afirmar "04 llamada a LLM" 1 "status=$EST respuesta='$RESP'"

# --- 05 contrato de entrada: válido e inválido ---
ID=$(registrar 05-contrato-inputs.yaml)
R=$(ejecutar "$ID" '{"trigger_data":{"documento":"texto de prueba"}}')
EST=$(jq -r '.status // "?"' <<<"$R")
[ "$EST" = "Completed" ] && afirmar "05 contrato de entrada (válido)" 0 \
                         || afirmar "05 contrato de entrada (válido)" 1 "status=$EST"

CODIGO=$(codigo_http POST "/api/v1/agents/$ID/execute" '{"trigger_data":{}}')
[ "$CODIGO" = "422" ] && afirmar "05 contrato de entrada (falta obligatorio)" 0 "422 desde el motor" \
                      || afirmar "05 contrato de entrada (falta obligatorio)" 1 "devolvió $CODIGO (se esperaba 422)"

# --- 06 volumen escribible ---
ID=$(registrar 06-persistencia-volumen.yaml)
R=$(ejecutar "$ID")
EST=$(jq -r '.status // "?"' <<<"$R")
LEIDO=$(jq -r '.state.leer.content // ""' <<<"$R")
{ [ "$EST" = "Completed" ] && [[ "$LEIDO" == *"persistencia-docker-ok"* ]]; } \
    && afirmar "06 escribe y relee en el volumen" 0 "$(jq -r '.state.escribir.path // ""' <<<"$R")" \
    || afirmar "06 escribe y relee en el volumen" 1 "status=$EST leído='$LEIDO'"

# --- 07 shell del contenedor y usuario del proceso ---
ID=$(registrar 07-bash-contenedor.yaml)
R=$(ejecutar "$ID")
EST=$(jq -r '.status // "?"' <<<"$R")
SALIDA=$(jq -r '.state.ejecutar.stdout // ""' <<<"$R")
CODIGO_SALIDA=$(jq -r '.state.ejecutar.exit_code // -1' <<<"$R")
USUARIO=$(tail -n1 <<<"$SALIDA" | tr -d '[:space:]')
{ [ "$EST" = "Completed" ] && [ "$CODIGO_SALIDA" = "0" ] && [[ "$SALIDA" == *"bash-en-docker-ok"* ]]; } \
    && afirmar "07 shell disponible en la imagen" 0 "exit=0" \
    || afirmar "07 shell disponible en la imagen" 1 "status=$EST exit=$CODIGO_SALIDA"
[ "$USUARIO" != "root" ] && afirmar "07 el motor no corre como root" 0 "usuario: $USUARIO" \
                         || afirmar "07 el motor no corre como root" 1 "corre como root — cualquier YAML tendría el contenedor entero"

# --- 08 agente live: ciclado autónomo ---
# El flujo declara interval_seconds 1 y max_cycles 3, y el primer ciclo sale
# sin esperar: a los 5 segundos tienen que estar los tres.
ID=$(registrar 08-agente-live.yaml)
PLAY=$(api POST "/api/v1/agents/$ID/play")
ESTADO_PLAY=$(jq -r '.status // "?"' <<<"$PLAY")
[ "$ESTADO_PLAY" = "playing" ] && afirmar "08 agente live acepta play" 0 "intervalo 1s, máximo 3 ciclos" \
                               || afirmar "08 agente live acepta play" 1 "status=$ESTADO_PLAY"

sleep 5
CICLOS=$(api GET "/api/v1/agents/$ID/cycles")
TOTAL=$(jq -r '.total_cycles // 0' <<<"$CICLOS")
COMPLETADOS=$(jq -r '[.cycles[]? | select(.status=="completed" or .status=="Completed")] | length' <<<"$CICLOS")
[ "$TOTAL" -eq 3 ] && afirmar "08 el agente cicla solo" 0 "$TOTAL ciclos, $COMPLETADOS completados" \
                   || afirmar "08 el agente cicla solo" 1 "total_cycles=$TOTAL (se esperaban 3)"

RECUERDO=$(api GET "/api/v1/agents/$ID/memory" | jq -r '.memory.ultimo_ciclo // "vacía"')
[ "$RECUERDO" = "3" ] && afirmar "08 la memoria persiste entre ciclos" 0 "ultimo_ciclo=$RECUERDO" \
                      || afirmar "08 la memoria persiste entre ciclos" 1 "ultimo_ciclo=$RECUERDO (se esperaba 3)"

# Al agotar max_cycles el agente tiene que quedar desregistrado: si siguiera
# figurando como activo, este segundo play devolvería 409.
CODIGO=$(codigo_http POST "/api/v1/agents/$ID/play")
[ "$CODIGO" = "200" ] && afirmar "08 se puede volver a lanzar tras terminar" 0 "segundo play aceptado" \
                      || afirmar "08 se puede volver a lanzar tras terminar" 1 "devolvió $CODIGO (409 = quedó registrado sin correr)"
api POST "/api/v1/agents/$ID/stop" >/dev/null

# --- SSE en vivo ---
ID=$(registrar 01-pipeline-lineal.yaml)
EVENTOS=$(timeout 30 curl -sN -X POST "$URL/api/v1/agents/$ID/stream" \
    -H "Content-Type: application/json" ${CLAVE:+-H "X-API-Key: $CLAVE"} \
    -d '{"trigger_data":{}}' 2>/dev/null)
FALTAN=""
for EV in graph.started node.started node.completed graph.completed; do
    grep -q "event: $EV" <<<"$EVENTOS" || FALTAN="$FALTAN $EV"
done
[ -z "$FALTAN" ] && afirmar "SSE emite el ciclo completo de eventos" 0 "$(grep -c '^event:' <<<"$EVENTOS") eventos" \
                 || afirmar "SSE emite el ciclo completo de eventos" 1 "faltaron:$FALTAN"

# ===========================================================================
titulo "── Estado del servidor tras las ejecuciones ───────────────────────"
# ===========================================================================

N=$(api GET /api/v1/sessions | jq -r 'length // 0')
[ "$N" -ge 1 ] && afirmar "las sesiones quedan consultables" 0 "$N sesiones" \
               || afirmar "las sesiones quedan consultables" 1 "no se registró ninguna"

CLAVES=$(api GET /api/v1/metrics | jq -r 'keys | join(",")')
[[ "$CLAVES" == *"sessions"* ]] && afirmar "/metrics expone contadores" 0 "$CLAVES" \
                               || afirmar "/metrics expone contadores" 1 "claves=$CLAVES"

# ===========================================================================
titulo "── Brechas conocidas del motor (verificadas, no son fallos) ────────"
# ===========================================================================

# --- 09 sub-agente: la ejecución real no está implementada ---
ID=$(registrar 09-subagente.yaml)
R=$(ejecutar "$ID")
SUB=$(jq -r '.state.delegar.status // "?"' <<<"$R")
if [ "$SUB" = "placeholder" ]; then
    brecha "09 sub-agente devuelve placeholder" "el grafo completa igual: verde falso si nadie mira el nodo"
else
    afirmar "09 sub-agente ejecuta de verdad (brecha resuelta)" 0 "status=$SUB — actualizar docker/README.md"
fi

# ===========================================================================
printf "\n───────────────────────────────────────────────────────────────────\n"
printf "  %s%s verificaciones OK%s" "$VERDE" "$OK" "$FIN"
[ "$BRECHAS" -gt 0 ] && printf "   ·   %s%s brechas conocidas%s" "$AMARILLO" "$BRECHAS" "$FIN"
[ "$FALLOS" -gt 0 ] && printf "   ·   %s%s fallos%s" "$ROJO" "$FALLOS" "$FIN"
printf "\n───────────────────────────────────────────────────────────────────\n"

if [ "$FALLOS" -gt 0 ]; then
    printf "%s%s%s\n\n" "$ROJO" "$DETALLE_FALLOS" "$FIN"
    exit 1
fi
exit 0
