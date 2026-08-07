#!/bin/bash
# start-engine-claude-cloud.sh — Arranca el engine usando Claude en la NUBE
# via la SUSCRIPCION de Claude Code (headless, SIN API key metered).
#
# Arquitectura:
#   engine :4321  --(provider openai, base-url proxy)-->  proxy :8787  --(claude -p)-->  suscripcion
#
# El adapter `claude` nativo del engine SOLO habla api.anthropic.com + ANTHROPIC_API_KEY,
# que la suscripcion NO expone. Por eso pasamos por el shim OpenAI-compat (claude-code-proxy.js).
#
# Uso:  bash tools/start-engine-claude-cloud.sh
# Fallback local:  MIRAI_LLM_PROVIDER=ollama MIRAI_LLM_MODEL=gemma4:latest ./target/release/mirai serve --port 4321 --ui-dir visualizer
set -euo pipefail
cd "$(dirname "$0")/.."   # raiz del repo Engine
PROXY_PORT="${CLAUDE_PROXY_PORT:-8787}"

# 1) Proxy de suscripcion (idempotente: no relanza si ya responde)
if ! curl -sf "http://127.0.0.1:${PROXY_PORT}/health" >/dev/null 2>&1; then
  echo "Arrancando claude-code-proxy en :${PROXY_PORT} ..."
  CLAUDE_PROXY_PORT="$PROXY_PORT" nohup node tools/claude-code-proxy.js > /tmp/mirai-claude-proxy.log 2>&1 &
  sleep 1.5
fi
curl -sf "http://127.0.0.1:${PROXY_PORT}/health" >/dev/null && echo "proxy OK :${PROXY_PORT}"

# 2) Engine apuntando al proxy. --api-key es un dummy: el proxy lo ignora
#    (el CLI exige una key NO vacia para el provider openai, pero no se usa).
echo "Arrancando engine :4321 -> Claude nube via proxy ..."
MIRAI_LLM_PROVIDER=openai \
MIRAI_LLM_MODEL=claude-sonnet-4-20250514 \
OPENAI_BASE_URL="http://127.0.0.1:${PROXY_PORT}/v1" \
OPENAI_API_KEY=sk-proxy-nokey \
  nohup ./target/release/mirai serve --port 4321 --ui-dir visualizer \
    --provider openai --base-url "http://127.0.0.1:${PROXY_PORT}/v1" \
    > /tmp/mirai-engine.log 2>&1 &
sleep 2
curl -sf http://localhost:4321/health >/dev/null && echo "engine OK :4321  (UI en http://localhost:4321/ui)"
echo "Provider activo:"; grep "LLM:" /tmp/mirai-engine.log || true

# 3) Para correr un agente por CLI contra la nube:
#   ./target/release/mirai run examples/hello-world.yaml \
#     --provider openai --base-url http://127.0.0.1:${PROXY_PORT}/v1 \
#     --model claude-sonnet-4-20250514 --api-key sk-proxy-nokey \
#     --input '{"question":"..."}'
