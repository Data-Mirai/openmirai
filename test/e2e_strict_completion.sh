#!/bin/bash
# =============================================================================
# OpenMirai — strict_completion end-to-end suite (PRD-022)
#
# Drives the compiled `mirai` binary against real agents and real tools. No
# mock provider: every graph here is deterministic (trigger, merge, response,
# read_file), so the guarantee is exercised without an LLM in the loop.
#
# Usage: bash test/e2e_strict_completion.sh
# =============================================================================

set -u

BINARY="./target/release/mirai"
[ -f "$BINARY" ] || BINARY="./target/debug/mirai"
if [ ! -f "$BINARY" ]; then
    echo "ERROR: mirai binary not found. Run 'cargo build' first."
    exit 1
fi

DIR="test/strict"
OUT_DIR="${STRICT_E2E_OUT:-/tmp/mirai_strict_e2e}"
mkdir -p "$OUT_DIR"

PASSED=0
FAILED=0
ERRORS=""

# check <name> <expected_exit> <expected_substring> -- <command...>
check() {
    local name="$1"; shift
    local want_exit="$1"; shift
    local want_text="$1"; shift
    [ "$1" = "--" ] && shift

    printf "  %-52s" "$name"
    local log="$OUT_DIR/${name}.log"
    "$@" > "$log" 2>&1
    local got_exit=$?

    if [ "$got_exit" != "$want_exit" ]; then
        echo "FAIL (exit $got_exit, wanted $want_exit)"
        FAILED=$((FAILED + 1))
        ERRORS="$ERRORS\n  FAIL: $name — exit $got_exit != $want_exit (see $log)"
        return
    fi
    if [ -n "$want_text" ] && ! grep -qF "$want_text" "$log"; then
        echo "FAIL (missing: $want_text)"
        FAILED=$((FAILED + 1))
        ERRORS="$ERRORS\n  FAIL: $name — output lacks '$want_text' (see $log)"
        return
    fi
    echo "PASS"
    PASSED=$((PASSED + 1))
}

echo "============================================"
echo "  strict_completion E2E (PRD-022)"
echo "  Binary: $BINARY"
echo "  Logs:   $OUT_DIR"
echo "============================================"
echo ""

# --- Layer 1: rejected at load time ----------------------------------------

check "TEST-207 validate rejects conditional dead end" 1 \
    "has only conditional outgoing edges" \
    -- $BINARY validate "$DIR/207_conditional_dead_end.yaml"

check "TEST-207 run rejects it too" 1 \
    "has only conditional outgoing edges" \
    -- $BINARY run "$DIR/207_conditional_dead_end.yaml"

check "TEST-208 same graph without the flag validates" 0 \
    "Valid agent spec" \
    -- $BINARY validate "$DIR/208_same_graph_no_flag.yaml"

check "TEST-208 same graph without the flag completes" 0 \
    '"status": "Completed"' \
    -- $BINARY run "$DIR/208_same_graph_no_flag.yaml"

check "TEST-209 validate rejects fan-out without join" 1 \
    "never converges" \
    -- $BINARY validate "$DIR/209_fanout_without_join.yaml"

check "TEST-209 error names the branches" 1 \
    "[branch_a, branch_b]" \
    -- $BINARY validate "$DIR/209_fanout_without_join.yaml"

# --- Layer 2: caught at run time -------------------------------------------

check "TEST-210 run without a value fails" 1 \
    "strict_completion: run ended at node 'summarize' with no value" \
    -- $BINARY run "$DIR/210_ends_without_value.yaml"

check "TEST-211 skipped error surfaces the cause" 1 \
    "strict_completion: node 'save' failed and was skipped" \
    -- $BINARY run "$DIR/211_skip_swallows_error.yaml"

check "TEST-211 keeps the original cause" 1 \
    "no-such-file.txt" \
    -- $BINARY run "$DIR/211_skip_swallows_error.yaml"

check "TEST-212 route_to_error without an error edge fails" 1 \
    "strict_completion: node 'save' routed to error but no error edge matched" \
    -- $BINARY run "$DIR/212_route_to_error_without_edge.yaml"

# --- No false positives -----------------------------------------------------

check "TEST-213 healthy graph still completes" 0 \
    '"status": "Completed"' \
    -- $BINARY run "$DIR/213_healthy_graph.yaml"

check "TEST-213 healthy graph reports no error" 0 \
    '"error": null' \
    -- $BINARY run "$DIR/213_healthy_graph.yaml"

check "TEST-214 human input still pauses" 1 \
    '"status": "Paused"' \
    -- $BINARY run "$DIR/214_human_input_pauses.yaml"

# --- Adoption path: --strict on a graph that does not declare it ------------

check "TEST-215 validate --strict forces the guarantee" 1 \
    "has only conditional outgoing edges" \
    -- $BINARY validate "$DIR/208_same_graph_no_flag.yaml" --strict

check "TEST-215 run --strict forces the guarantee" 1 \
    "has only conditional outgoing edges" \
    -- $BINARY run "$DIR/208_same_graph_no_flag.yaml" --strict

echo ""
echo "============================================"
echo "  Passed: $PASSED   Failed: $FAILED"
echo "============================================"
if [ "$FAILED" -gt 0 ]; then
    echo -e "$ERRORS"
    exit 1
fi
