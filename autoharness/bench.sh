#!/bin/bash
set -euo pipefail

API="http://localhost:3111"
TAG="${1:-$(date +%b%d | tr '[:upper:]' '[:lower:]')}"

echo "=========================================="
echo "  autoagent-iii benchmark runner"
echo "  tag: $TAG"
echo "=========================================="

# -------------------------------------------------------------------
# 1. Check prerequisites
# -------------------------------------------------------------------

check() {
    if ! command -v "$1" &>/dev/null; then
        echo "ERROR: $1 not found. Install it first."
        exit 1
    fi
}

check iii
check curl
check jq

if ! curl -sf "$API/api/report/tags" >/dev/null 2>&1; then
    echo ""
    echo "iii-engine + orchestrator not running. Starting them..."
    echo ""

    cd "$(dirname "$0")"

    iii --config iii-config.yaml &
    III_PID=$!
    sleep 2

    cd workers/orchestrator
    python3 orchestrator.py &
    ORCH_PID=$!
    sleep 3
    cd ../..

    echo "Started iii-engine (PID $III_PID) + orchestrator (PID $ORCH_PID)"

    trap "kill $III_PID $ORCH_PID 2>/dev/null" EXIT
fi

echo ""
echo "API: $API"
echo ""

# -------------------------------------------------------------------
# 2. Setup experiment tag
# -------------------------------------------------------------------

echo "--- Setting up tag: $TAG ---"
SETUP=$(curl -sf -X POST "$API/api/experiment/setup" \
    -H "Content-Type: application/json" \
    -d "{\"tag\":\"$TAG\"}" 2>/dev/null || echo '{"body":{"error":"exists"}}')

echo "$SETUP" | jq -r '.body // .' 2>/dev/null || echo "$SETUP"

# -------------------------------------------------------------------
# 3. List available tasks
# -------------------------------------------------------------------

echo ""
echo "--- Available tasks ---"
TASKS=$(curl -sf "$API/api/task/list" | jq -r '.body.tasks[].name' 2>/dev/null)
echo "$TASKS"
TASK_COUNT=$(echo "$TASKS" | wc -l | tr -d ' ')
echo "Total: $TASK_COUNT tasks"

# -------------------------------------------------------------------
# 4. Register baseline experiment
# -------------------------------------------------------------------

echo ""
echo "--- Registering baseline experiment ---"
COMMIT=$(git rev-parse --short HEAD 2>/dev/null || echo "none")
REG=$(curl -sf -X POST "$API/api/experiment/register" \
    -H "Content-Type: application/json" \
    -d "{
        \"tag\": \"$TAG\",
        \"hypothesis\": \"Baseline harness — no modifications\",
        \"description\": \"baseline\",
        \"category\": \"other\",
        \"commit_sha\": \"$COMMIT\"
    }")

EXP_ID=$(echo "$REG" | jq -r '.body.experiment_id // empty')
echo "Experiment ID: $EXP_ID"

if [ -z "$EXP_ID" ]; then
    echo "ERROR: Failed to register experiment"
    echo "$REG"
    exit 1
fi

# -------------------------------------------------------------------
# 5. Run benchmark (all tasks)
# -------------------------------------------------------------------

echo ""
echo "--- Running benchmark (all tasks, concurrency=4) ---"
echo "This may take a few minutes..."

BATCH=$(curl -sf -X POST "$API/api/task/batch" \
    -H "Content-Type: application/json" \
    -d "{\"experiment_id\": \"$EXP_ID\", \"concurrency\": 4}" \
    --max-time 900 2>/dev/null || echo '{"body":{"error":"batch failed"}}')

PASSED=$(echo "$BATCH" | jq -r '.body.passed // 0')
TOTAL=$(echo "$BATCH" | jq -r '.body.total_tasks // 0')
SCORE=$(echo "$BATCH" | jq -r '.body.aggregate_score // 0')
DURATION=$(echo "$BATCH" | jq -r '.body.duration_seconds // 0')

echo ""
echo "Results: $PASSED/$TOTAL passed (score: $SCORE)"
echo "Duration: ${DURATION}s"

# -------------------------------------------------------------------
# 6. Record completion
# -------------------------------------------------------------------

echo ""
echo "--- Recording results ---"
COMPLETE=$(curl -sf -X POST "$API/api/experiment/complete" \
    -H "Content-Type: application/json" \
    -d "{
        \"experiment_id\": \"$EXP_ID\",
        \"passed\": $PASSED,
        \"total_tasks\": $TOTAL,
        \"aggregate_score\": $SCORE,
        \"task_scores\": $(echo "$BATCH" | jq '.body.task_scores // {}'),
        \"duration_seconds\": $DURATION
    }")

STATUS=$(echo "$COMPLETE" | jq -r '.body.status // "unknown"')
ACTION=$(echo "$COMPLETE" | jq -r '.body.action // "unknown"')
echo "Status: $STATUS | Action: $ACTION"

# -------------------------------------------------------------------
# 7. Show summary
# -------------------------------------------------------------------

echo ""
echo "--- Summary ---"
curl -sf -X POST "$API/api/report/summary" \
    -H "Content-Type: application/json" \
    -d "{\"tag\": \"$TAG\"}" | jq '.body | {
        tag,
        best: .best,
        stats: .stats,
        strategy,
        total_duration_minutes,
        common_failures: (.common_failures | keys)
    }' 2>/dev/null

# -------------------------------------------------------------------
# 8. Get suggestions for next experiment
# -------------------------------------------------------------------

echo ""
echo "--- Suggestions for next experiment ---"
curl -sf -X POST "$API/api/search/suggest" \
    -H "Content-Type: application/json" \
    -d "{\"tag\": \"$TAG\"}" | jq '.body.suggestions[]' 2>/dev/null

echo ""
echo "=========================================="
echo "  Benchmark complete!"
echo "  Tag: $TAG"
echo "  Passed: $PASSED/$TOTAL"
echo "=========================================="
echo ""
echo "Next steps:"
echo "  1. Edit agent.py (the editable section)"
echo "  2. Run: ./bench.sh $TAG"
echo "  3. The system auto-keeps or discards"
echo "  4. Repeat until score converges"
echo ""
echo "Or give program.md to a meta-agent:"
echo "  claude -p program.md"
