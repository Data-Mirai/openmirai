#!/bin/bash
# =============================================================================
# Data Mirai Engine — Integration Test Suite
#
# Runs 10 diverse agent tests covering different engine capabilities.
# Uses --provider mock for deterministic execution (no LLM needed).
#
# Usage: bash test/run_all.sh
# =============================================================================

set -e

BINARY="./target/release/mirai"
if [ ! -f "$BINARY" ]; then
    BINARY="./target/debug/mirai"
fi
if [ ! -f "$BINARY" ]; then
    echo "ERROR: mirai binary not found. Run 'cargo build' first."
    exit 1
fi

echo "============================================"
echo "  Data Mirai Engine — Integration Tests"
echo "  Binary: $BINARY"
echo "  Version: $($BINARY version)"
echo "============================================"
echo ""

PASSED=0
FAILED=0
ERRORS=""

run_test() {
    local name="$1"
    local file="$2"
    local extra_args="$3"

    printf "  %-45s" "$name"

    # First: validate the spec parses correctly
    if ! $BINARY validate "$file" > /dev/null 2>&1; then
        echo "FAIL (validation)"
        FAILED=$((FAILED + 1))
        ERRORS="$ERRORS\n  FAIL: $name — spec validation failed"
        return
    fi

    # Then: run with mock provider
    OUTPUT=$($BINARY run "$file" --provider mock $extra_args 2>&1) || true

    if echo "$OUTPUT" | grep -q '"status":"Completed"\|"status": "Completed"\|Completed'; then
        echo "PASS"
        PASSED=$((PASSED + 1))
    elif echo "$OUTPUT" | grep -q 'validation failed\|missing required'; then
        # Expected failure for validation tests
        if echo "$name" | grep -q "should_fail"; then
            echo "PASS (expected failure)"
            PASSED=$((PASSED + 1))
        else
            echo "FAIL"
            FAILED=$((FAILED + 1))
            ERRORS="$ERRORS\n  FAIL: $name — unexpected validation error"
        fi
    else
        echo "FAIL"
        FAILED=$((FAILED + 1))
        ERRORS="$ERRORS\n  FAIL: $name"
    fi
}

echo "--- Gradient 1: Basic traversal ---"
run_test "01 Linear pipeline" "test/test_01_linear_pipeline.yaml"
echo ""

echo "--- Gradient 2: Conditional routing ---"
run_test "02 Conditional equals" "test/test_02_conditional_equals.yaml"
run_test "03 Numeric routing" "test/test_03_numeric_routing.yaml"
echo ""

echo "--- Gradient 3: Parallelism ---"
run_test "04 Fan-out parallel" "test/test_04_fanout_parallel.yaml"
echo ""

echo "--- Gradient 4: Data flow ---"
run_test "05 Data map between nodes" "test/test_05_data_map_flow.yaml"
echo ""

echo "--- Gradient 5: Resilience ---"
run_test "06 Retry policy config" "test/test_06_retry_policy.yaml"
echo ""

echo "--- Gradient 6: Scale ---"
run_test "07 Deep chain (8 nodes)" "test/test_07_deep_chain.yaml"
echo ""

echo "--- Gradient 7: Edge cases ---"
run_test "08 Mixed conditions + fallback" "test/test_08_mixed_conditions.yaml"
echo ""

echo "--- Gradient 8: Contracts ---"
run_test "09 Input validation (valid)" "test/test_09_input_validation.yaml" "--input '{\"question\": \"hello\"}'"
run_test "09 Input validation (should_fail)" "test/test_09_input_validation.yaml" "--input '{}'"
echo ""

echo "--- Gradient 9: Real-world complexity ---"
run_test "10 Complex multi-feature graph" "test/test_10_complex_graph.yaml"
echo ""

echo "============================================"
echo "  Results: $PASSED passed, $FAILED failed"
echo "============================================"

if [ $FAILED -gt 0 ]; then
    echo ""
    echo "Failures:"
    echo -e "$ERRORS"
    echo ""
    exit 1
else
    echo ""
    echo "All tests passed."
    exit 0
fi
