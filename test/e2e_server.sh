#!/bin/bash
# =============================================================================
# OpenMirai — HTTP Server End-to-End Test Suite
#
# Starts the server in mock mode, executes requests using curl, and checks
# response statuses and schemas.
# =============================================================================

set -e

PORT=8085
HOST="127.0.0.1"
URL="http://$HOST:$PORT"
BINARY="./target/debug/mirai"

if [ ! -f "$BINARY" ]; then
    echo "ERROR: mirai binary not found. Build it with 'cargo build' first."
    exit 1
fi

echo "============================================"
# 1. Start the HTTP server in the background using the mock provider
echo "Starting openmirai HTTP server on $URL..."
$BINARY serve --port $PORT --provider mock > /tmp/mirai_server_e2e.log 2>&1 &
SERVER_PID=$!

# Enforce cleanup on script exit
cleanup() {
    echo "Cleaning up server process (PID $SERVER_PID)..."
    kill $SERVER_PID 2>/dev/null || true
    wait $SERVER_PID 2>/dev/null || true
}
trap cleanup EXIT

# 2. Wait for the server to be healthy
echo "Waiting for server to become healthy..."
HEALTHY=false
for i in {1..20}; do
    if curl -s "$URL/health" | grep -q '"status":"ok"'; then
        HEALTHY=true
        break
    fi
    sleep 0.5
done

if [ "$HEALTHY" = false ]; then
    echo "ERROR: Server failed to start or report healthy."
    echo "--- Server Logs ---"
    cat /tmp/mirai_server_e2e.log
    exit 1
fi

echo "Server is healthy! Running tests..."
echo ""

# 3. Test GET /version
echo "Checking /version..."
VERSION_OUT=$(curl -s -f "$URL/version")
echo "  Result: $VERSION_OUT"
if ! echo "$VERSION_OUT" | grep -q '"version"'; then
    echo "FAIL: Invalid version output"
    exit 1
fi

# 4. Test GET /api/v1/tools
echo "Checking /api/v1/tools..."
TOOLS_OUT=$(curl -s -f "$URL/api/v1/tools")
if ! echo "$TOOLS_OUT" | grep -q '"tool_type":"logic/condition"'; then
    echo "FAIL: /api/v1/tools does not contain logic/condition"
    exit 1
fi
echo "  PASS: Registered tools list validated."

# 5. Test POST /api/v1/agents/from-spec
echo "Creating agent from spec..."
AGENT_SPEC=$(cat <<EOF
{
  "name": "e2e-agent",
  "description": "Created via E2E test",
  "version": "v1",
  "graph": {
    "nodes": [
      {
        "id": "start",
        "tool_type": "trigger/manual",
        "config": {
          "payload": {
            "query": "hello"
          }
        }
      },
      {
        "id": "respond",
        "tool_type": "output/response",
        "config": {
          "message": "E2E OK"
        }
      }
    ],
    "edges": [
      {
        "source": "start",
        "target": "respond"
      }
    ]
  }
}
EOF
)

AGENT_RESP=$(curl -s -f -X POST \
  -H "Content-Type: application/json" \
  -d "$AGENT_SPEC" \
  "$URL/api/v1/agents/from-spec")

AGENT_ID=$(echo "$AGENT_RESP" | grep -o '"id":"[^"]*' | grep -o '[^"]*$')
if [ -z "$AGENT_ID" ]; then
    echo "FAIL: Failed to extract agent ID from spec response: $AGENT_RESP"
    exit 1
fi
echo "  PASS: Agent created with ID: $AGENT_ID"

# 6. Test POST /api/v1/agents/{id}/execute
echo "Executing agent (sync)..."
EXEC_BODY='{"trigger_data":{}}'
EXEC_RESP=$(curl -s -f -X POST \
  -H "Content-Type: application/json" \
  -d "$EXEC_BODY" \
  "$URL/api/v1/agents/$AGENT_ID/execute")

if ! echo "$EXEC_RESP" | grep -q '"status":"Completed"'; then
    echo "FAIL: Sync execution did not complete: $EXEC_RESP"
    exit 1
fi
echo "  PASS: Sync execution completed successfully."

# 7. Test POST /api/v1/agents/{id}/stream
echo "Executing agent (stream)..."
STREAM_OUT=$(curl -s -f -X POST \
  -H "Content-Type: application/json" \
  -d "$EXEC_BODY" \
  "$URL/api/v1/agents/$AGENT_ID/stream")

if ! echo "$STREAM_OUT" | grep -q "event: completed" && ! echo "$STREAM_OUT" | grep -q "Completed"; then
    echo "FAIL: Stream execution did not return completed event: $STREAM_OUT"
    exit 1
fi
echo "  PASS: SSE Stream execution completed successfully."

echo ""
echo "============================================"
echo "  All server E2E tests completed successfully!"
echo "============================================"
exit 0
